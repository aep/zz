
#[macro_use] extern crate pest_derive;
extern crate metrohash;
#[macro_use] extern crate log;
extern crate pbr;
extern crate rayon;
extern crate askama;

pub mod ast;
pub mod parser;
pub mod project;
pub mod make;
pub mod loader;
pub mod flatten;
pub mod emitter;
pub mod emitter_rs;
pub mod emitter_js;
pub mod abs;
pub mod name;
pub mod pp;
pub mod symbolic;
pub mod expand;
pub mod smt;
pub mod emitter_docs;
pub mod makro;

use std::path::Path;
use name::Name;
use std::collections::HashSet;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};


pub struct Error {
    message:    String,
    details:    Vec<(ast::Location, String)>,
}

impl Error {
    pub fn new(message: String, details:    Vec<(ast::Location, String)>) -> Self {
        Self{
            message,
            details,
        }
    }
}

static ABORT: AtomicBool = AtomicBool::new(false);

#[derive(PartialEq)]
pub enum BuildSet {
    Tests,
    Run,
    Check,
    All,
}

pub fn build(buildset: BuildSet, variant: &str, stage: make::Stage, slow: bool) {
    use rayon::prelude::*;
    use std::sync::{Arc, Mutex};

    let (root, mut project) = project::load_cwd();
    //std::env::set_current_dir(root).unwrap();

    std::fs::create_dir_all(root.join("target").join(stage.to_string()).join("c")).expect("create target dir");
    std::fs::create_dir_all(root.join("target").join(stage.to_string()).join("zz")).expect("create target dir");
    std::fs::create_dir_all(root.join("target").join(stage.to_string()).join("include")
                            .join("zz").join(&project.project.name)).expect("create target dir");

    let project_name        = Name(vec![String::new(), project.project.name.clone()]);
    let project_tests_name  = Name(vec![String::new(), project.project.name.clone(), "tests".to_string()]);



    let mut modules = HashMap::new();
    let features = project.features(variant).into_iter().map(|(n,(e,_))|(n,e)).collect();
    if root.join("src").exists() {
        loader::load(&mut modules, &project_name, &root.join("src"), &features, &stage);
    }
    if root.join("tests").exists() {
        loader::load(&mut modules, &project_tests_name, &root.join("tests").canonicalize().unwrap(), &features, &stage);
    }



    let mut searchpaths = HashSet::new();
    searchpaths.insert(std::env::current_exe().expect("self path")
        .canonicalize().expect("self path")
        .parent().expect("self path")
        .parent().expect("self path")
        .parent().expect("self path")
        .join("modules"));
    searchpaths.insert(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("modules")
    );

    if let Ok(zz_path) = std::env::var("ZZ_MODULE_PATHS") {
        let module_paths = if cfg!(windows) {
            zz_path.split(";")
        } else {
            zz_path.split(":")
        };

        for path in module_paths {
            searchpaths.insert(std::path::Path::new(&path).to_path_buf());
        }
    }

    if let Some(deps) = &project.dependencies {
        for (name, dep) in deps {
            match dep {
                toml::Value::String(_) => {
                    getdep(name, &mut modules, &mut project.project, &mut searchpaths, &stage);
                },
                _ => (),
            }
        }
    }



    let mut ext = abs::Ext::new();
    let mut names : Vec<Name> = modules.keys().cloned().collect();
    names.sort_unstable();

    let mut pb = pbr::ProgressBar::new(names.len() as u64);
    pb.show_speed = false;

    for name in &names {
        let mut md = modules.remove(name).unwrap();
        match &mut md {
            loader::Module::C(_) => (),
            loader::Module::ZZ(ast) => {
                abs::abs(ast, &modules, &mut ext);
            }
        }
        modules.insert(name.clone(), md);
        pb.message(&format!("abs {}", name));
        pb.inc();
    }
    pb.finish_print("done abs");

    let pb = Arc::new(Mutex::new(pbr::ProgressBar::new(names.len() as u64)));
    pb.lock().unwrap().show_speed = false;

    let silent = parser::ERRORS_AS_JSON.load(Ordering::SeqCst);
    let working_on_these = Arc::new(Mutex::new(HashSet::new()));

    let iterf =  |modules: HashMap<Name, loader::Module>, macropass: bool, name: Name| {
        let (_, outname) = emitter::outname(&project.project, &stage, &name, false);

        let cachename = format!("{}.buildcache", outname);
        let cached : Option<emitter::CFile> = match std::fs::read_to_string(&cachename) {
            Ok(f) => {
                match serde_json::from_str(&f) {
                    Ok(cf) => Some(cf),
                    Err(_) => {
                        std::fs::remove_file(&cachename).expect(&format!("cannot remove {}", cachename));
                        None
                    }
                }

            },
            Err(_) => None,
        };

        //only emit if any source file is newer than the cache or output
        if let Some(cached) = cached {
            if !cached.is_newer_than(&outname) && !cached.is_newer_than(&cachename) {
                if !silent {
                    //pb.lock().unwrap().message(&format!("cached {} ", module.name));
                    pb.lock().unwrap().inc();
                }
                return Ok((Vec::new(), (Some((cached.name.clone(), cached)))));
            }
        }


        let mut modules = modules.clone();
        let mut module = modules.remove(&name).unwrap();

        let mut macro_mods = Vec::new();
        let mut module = match &mut module {
            loader::Module::C(c) => {
                let cf = emitter::CFile{
                    name:       name,
                    filepath:   c.to_string_lossy().into(),
                    sources:    HashSet::new(),
                    deps:       HashSet::new(),
                };
                return Ok((Vec::new(),Some((cf.name.clone(), cf))));
            }
            loader::Module::ZZ(ast) => {
                if macropass {
                    macro_mods = makro::sieve(ast);
                }
                flatten::flatten(ast, &modules, &ext)
            }
        };

        let module_human_name = module.name.human_name();
        if !silent {
            working_on_these.lock().unwrap().insert(module_human_name.clone());
            let mut indic = String::new();
            for working_on in  working_on_these.lock().unwrap().iter() {
                if !indic.is_empty() {
                    indic.push_str(", ");
                }
                if indic.len() > 30 {
                    indic = format!("{}.. ", indic);
                    break;
                }
                indic = format!("{}{} ", indic, working_on);
            }
            indic = format!("prove [ {}]  ", indic);
            pb.lock().unwrap().message(&indic);
            pb.lock().unwrap().tick();
        }

        expand::expand(&mut module)?;
        let (ok, complete) = symbolic::execute(&mut module, !macropass);
        if !ok {
            ABORT.store(true, Ordering::Relaxed);
            return Ok((Vec::new(), None));
        }


        let header  = emitter::Emitter::new(&project.project, stage.clone(), module.clone(), true);
        header.emit();

        let rsbridge = emitter_rs::Emitter::new(&project.project, stage.clone(), module.clone());
        rsbridge.emit();

        let jsbridge = emitter_js::Emitter::new(&project.project, stage.clone(), module.clone());
        jsbridge.emit();

        let docs = emitter_docs::Emitter::new(&project.project, stage.clone(), module.clone());
        docs.emit();

        let em = emitter::Emitter::new(&project.project, stage.clone(), module, false);
        let cf = em.emit();


        if !silent {
            working_on_these.lock().unwrap().remove(&module_human_name);
            let mut indic = String::new();
            for working_on in  working_on_these.lock().unwrap().iter() {
                if !indic.is_empty() {
                    indic.push_str(", ");
                }
                if indic.len() > 30 {
                    indic = format!("{}.. ", indic);
                    break;
                }
                indic = format!("{}{} ", indic, working_on);
            }
            indic = format!("prove [ {}]  ", indic);
            pb.lock().unwrap().message(&indic);
            pb.lock().unwrap().inc();
        }

        if complete {
            let cachefile = std::fs::File::create(&cachename).expect(&format!("cannot create {}", cachename));
            serde_json::ser::to_writer(cachefile, &cf).expect(&format!("cannot write {}", cachename));
        }

        Ok((macro_mods, Some((cf.name.clone(), cf))))
    };


    // pass 1: sieve macros
    *pb.lock().unwrap() = pbr::ProgressBar::new(names.len() as u64);
    let cfiles_r : Vec<Result<(Vec<ast::Module>, Option<(Name, emitter::CFile)>), Error>> = if slow {
        names.clone().into_iter().map(|m|iterf(modules.clone(), true,m)).collect()
    } else {
        names.clone().into_par_iter().map(|m|iterf(modules.clone(), true,m)).collect()
    };

    // build macros
    let mut cfiles = HashMap::new();
    for r in cfiles_r {
        match r {
            Ok((nm, cf)) => {
                if let Some(v) = cf {
                    cfiles.insert(v.0, v.1);
                }
                for macromod in nm {
                    let artifact = project::Artifact{
                        name:   macromod.name.0[1..].join("_"),
                        main:   format!("{}", macromod.name),
                        typ:    project::ArtifactType::Macro,
                        indexjs: None,
                    };
                    let mut need = Vec::new();
                    need.push(macromod.name.clone());

                    let mut make = make::Make::new(project.clone(), variant, stage.clone(), artifact);

                    modules.insert(macromod.name.clone(), loader::Module::ZZ(macromod));
                    names = modules.keys().cloned().collect();
                    names.sort_unstable();

                    *pb.lock().unwrap() = pbr::ProgressBar::new(names.len() as u64);
                    let cfiles_r : Vec<Result<(Vec<ast::Module>, Option<(Name, emitter::CFile)>), Error>> = if slow {
                        names.clone().into_iter().map(|m|iterf(modules.clone(), true,m)).collect()
                    } else {
                        names.clone().into_par_iter().map(|m|iterf(modules.clone(), true,m)).collect()
                    };
                    if ABORT.load(Ordering::Relaxed) {
                        std::process::exit(9);
                    }
                    let mut cfiles = HashMap::new();
                    for r in cfiles_r {
                        if let Ok((_,Some(v))) = r {
                            cfiles.insert(v.0, v.1);
                        }
                    };

                    let mut used = HashSet::new();
                    while need.len() > 0 {
                        for n in std::mem::replace(&mut need, Vec::new()) {
                            if !used.insert(n.clone()) {
                                continue
                            }
                            let n = cfiles.get(&n).expect(&format!("ICE: dependency {} module doesnt exist", n));
                            for d in &n.deps {
                                need.push(d.clone());
                            }
                            make.build(n);
                        }
                    }
                    make.link();
                }
            },
            Err(e) => {
                parser::emit_error(e.message.clone(), &e.details);
                ABORT.store(true, Ordering::Relaxed);
            }
        }
    };

    // pass 2: macros now available
    *pb.lock().unwrap() = pbr::ProgressBar::new(names.len() as u64);
    let cfiles_r : Vec<Result<(Vec<ast::Module>, Option<(Name, emitter::CFile)>), Error>> = if slow {
        names.clone().into_iter().map(|m|iterf(modules.clone(), false ,m)).collect()
    } else {
        names.clone().into_par_iter().map(|m|iterf(modules.clone(), false, m)).collect()
    };

    let mut cfiles = HashMap::new();
    for r in cfiles_r {
        match r {
            Ok((_, None)) => {},
            Ok((_, Some(v))) => {
                cfiles.insert(v.0, v.1);
            }
            Err(e) => {
                parser::emit_error(e.message.clone(), &e.details);
                ABORT.store(true, Ordering::Relaxed);
            }
        }
    };



    if ABORT.load(Ordering::Relaxed) {
        std::process::exit(9);
    }

    if !silent {
        pb.lock().unwrap().finish_print("done emitting");
    }

    for artifact in std::mem::replace(&mut project.artifacts, None).expect("no artifacts") {
        match (&artifact.typ, &buildset) {
            (project::ArtifactType::Test, BuildSet::Tests)  => (),
            (project::ArtifactType::Test, _)                => continue,
            (project::ArtifactType::Exe, _)                 => (),
            (_, BuildSet::Run)                              => continue,
            (_,_)                                           => (),
        };
        let mut make = make::Make::new(project.clone(), variant, stage.clone(), artifact.clone());

        let mut main = Name::from(&artifact.main);
        if !main.is_absolute() {
            main.0.insert(0,String::new());
        }
        let main = cfiles.get(&main).expect(&format!(
                "cannot build artifact '{}', main module '{}' does not exist", artifact.name, main));

        let mut need = Vec::new();
        need.push(main.name.clone());
        let mut used = HashSet::new();

        while need.len() > 0 {
            for n in std::mem::replace(&mut need, Vec::new()) {
                if !used.insert(n.clone()) {
                    continue
                }
                let n = cfiles.get(&n).expect(&format!("ICE: dependency {} module doesnt exist", n));
                for d in &n.deps {
                    need.push(d.clone());
                }
                make.build(n);
            }
        }

        if let project::ArtifactType::Lib = artifact.typ {
        }

        for entry in std::fs::read_dir("./src").unwrap() {
            let entry = entry.unwrap();
            let path  = entry.path();
            if path.is_file() {
                if let Some("c") = path.extension().map(|v|v.to_str().expect("invalid file name")) {
                    make.cobject(&path);
                }
            }
        }

        if buildset != BuildSet::Check {
            make.link();
        }

    };
}

fn getdep(
        name: &str,
        modules: &mut HashMap<Name, loader::Module>,
        rootproj: &mut project::Project,
        searchpaths: &mut HashSet<std::path::PathBuf>,
        stage:  &make::Stage,
) {

    searchpaths.insert(
        std::env::current_dir().unwrap().join("modules")
    );

    let mut found = None;
    for searchpath in searchpaths.iter() {
        let modpath = searchpath.join(name).join("zz.toml");
        if modpath.exists() {
            found = Some(searchpath.join(name));
        }
    }

    let found = match found {
        Some(v) => v,
        None => {
            eprintln!("dependency \"{}\" not found in any of {:#?}", name, searchpaths);
            std::process::exit(9);
        }
    };

    //let pp = std::env::current_dir().unwrap();
    //std::env::set_current_dir(&found).unwrap();
    let (root, project)  = project::load(&found);
    let project_name     = Name(vec![String::new(), project.project.name.clone()]);        
    if found.join("src").exists() {
        let features = project.features("default").into_iter().map(|(n,(e,_))|(n,e)).collect();
        loader::load(modules, &project_name, &found.join("src"), &features, &stage);
    }
    //std::env::set_current_dir(pp).unwrap();

    searchpaths.insert(
        root.join("modules")
    );


    for i in project.project.cincludes {
        let ii = root.join(&i);
        let i = std::fs::canonicalize(&ii).expect(&format!("{}: cannot resolve cinclude {:?}", name, ii));
        rootproj.cincludes.push(i.to_string_lossy().into());
    }
    for i in project.project.cobjects {
        let ii = root.join(&i);
        let i = std::fs::canonicalize(&ii).expect(&format!("{}: cannot resolve cobject {:?}", name, ii));
        rootproj.cobjects.push(i.to_string_lossy().into());
    }
    rootproj.pkgconfig.extend(project.project.pkgconfig);
    rootproj.cflags.extend(project.project.cflags);
    rootproj.lflags.extend(project.project.lflags);


    if let Some(deps) = &project.dependencies {
        for (name, dep) in deps {
            match dep {
                toml::Value::String(_) => {
                    getdep(name, modules, rootproj, searchpaths, stage);
                },
                _ => (),
            }
        }
    }
}




