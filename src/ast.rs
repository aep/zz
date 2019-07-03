use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt;
use std::path::PathBuf;

#[derive(PartialEq, Clone)]
pub struct Location {
    pub file:   String,
    pub span:   pest::Span<'static>,
}

impl Location {
    pub fn line(&self) -> usize {
        self.span.start_pos().line_col().0
    }
}

impl std::fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}:{}", self.file, self.line())
    }
}


#[derive(Clone)]
pub enum Storage {
    Static,
    ThreadLocal,
    Atomic,
}

#[derive(Clone, PartialEq)]
pub enum Visibility {
    Shared,
    Object,
    Export,
}

#[derive(Clone, PartialEq)]
pub struct Import {
    pub namespace:  Vec<String>,
    pub vis:        Visibility,
    pub loc:        Location,
}



#[derive(Clone)]
pub enum Def {
    Static {
        typeref:    TypeUse,
        expr:       CExpr,
        muta:       bool,
        storage:    Storage,
    },
    Const {
        typeref:    TypeUse,
        expr:       CExpr,
    },
    Function {
        ret:        Option<AnonArg>,
        args:       Vec<NamedArg>,
        body:       CExpr,
    },
    Struct {
        fields:     Vec<Field>,
        packed:     bool,
    },
    Macro {
        args:       Vec<String>,
        body:       CExpr,
        imports:    Vec<Import>,
    }
}


#[derive(Clone)]
pub struct Local {
    pub name:   String,
    pub vis:    Visibility,
    pub loc:    Location,
    pub def:    Def,
}


#[derive(Clone)]
pub struct Include {
    pub expr:   String,
    pub loc:    Location,
}

#[derive(Clone)]
pub struct TypeUse {
    pub name:   String,
    pub loc:    Location,
}

#[derive(Default, Clone)]
pub struct Module {
    pub namespace:  Vec<String>,
    pub source:     PathBuf,
    pub locals:     Vec<Local>,
    pub includes:   Vec<Include>,
    pub imports:    Vec<Import>,
    pub sources:    HashSet<PathBuf>,
}

#[derive(Clone)]
pub struct AnonArg {
    pub typeref:    TypeUse
}

#[derive(Clone)]
pub struct NamedArg {
    pub typeref:    TypeUse,
    pub name:       String,
    pub muta:       bool,
    pub ptr:        bool,
    pub namespace:  Option<String>,
}

#[derive(Clone)]
pub struct CExpr {
    pub expr:   String,
    pub loc:    Location,
}



#[derive(Clone)]
pub struct Field {
    pub typeref: TypeUse,
    pub expr:    CExpr,
    pub loc:     Location,
}
