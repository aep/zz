use super::ast;
use super::parser;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use super::name::Name;
use pbr;
use rayon::prelude::*;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{Ordering};
use super::make::Stage;

pub enum Module {
    C(PathBuf),
    ZZ(ast::Module),
}

pub fn load(
    modules:        &mut HashMap<Name, Module>,
    artifact_name:  &Name,
    src:            &Path,
    features:       &HashMap<String, bool>,
    stage:          &Stage,
) {


    let mut files = Vec::new();
    for entry in std::fs::read_dir(src).expect(&format!("cannot open src directory {:?} ", src)) {
        let entry = entry.unwrap();
        let path  = entry.path().canonicalize().unwrap();
        if path.is_file() {
            let ext = match path.extension().map(|v|v.to_str()) {
                Some(ext) => ext,
                None => continue
            };
            match ext {
                Some("h") => {
                    let mut name = artifact_name.clone();
                    name.push(path.file_stem().unwrap().to_string_lossy().to_string());
                    modules.insert(name, Module::C(path.into()));
                },
                Some("zz") => {
                    files.push(path.clone());
                },
                _ => {},
            }
        }
    }

    let pb = Arc::new(Mutex::new(pbr::ProgressBar::new(files.len() as u64)));
    pb.lock().unwrap().show_speed = false;
    let silent = parser::ERRORS_AS_JSON.load(Ordering::SeqCst);

    if !silent{
        pb.lock().unwrap().finish_print(&format!("parsing {}", artifact_name));
    }
    let om : HashMap<Name, Module> = files.into_par_iter().map(|path_buf| {
        let path = path_buf.canonicalize().unwrap();
        if !silent{
            pb.lock().unwrap().message(&format!("parsing {:?} ", path));
        }
        let mut m = parser::parse(&path, features, stage);
        m.name = artifact_name.clone();
        let stem = path.file_stem().unwrap().to_string_lossy().to_string();
        if stem != "lib" {
            m.name.push(stem);
        }
        debug!("loaded {:?} as {}", path, m.name);
        if !silent{
            pb.lock().unwrap().inc();
        }
        (m.name.clone(), Module::ZZ(m))
    }).collect();
    modules.extend(om);
    if !silent{
        pb.lock().unwrap().finish_print(&format!("finished parsing {}", artifact_name));
    }

}
