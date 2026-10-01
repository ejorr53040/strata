//! Language plugins: each language turns source text into language-neutral [`FileFacts`];
//! the resolver in [`crate::graph`] works only on those facts.

pub mod python;
pub mod rust;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefKind {
    Function,
    Class,
    Struct,
    Enum,
    Trait,
    Method,
}

impl DefKind {
    pub fn label(self) -> &'static str {
        match self {
            DefKind::Function => "function",
            DefKind::Class => "class",
            DefKind::Struct => "struct",
            DefKind::Enum => "enum",
            DefKind::Trait => "trait",
            DefKind::Method => "method",
        }
    }

    /// Kinds that own methods and can be inherited from or implemented.
    pub fn is_type(self) -> bool {
        matches!(
            self,
            DefKind::Class | DefKind::Struct | DefKind::Enum | DefKind::Trait
        )
    }
}

/// A definition. `qualname` is relative to its module, e.g. `Circle.radius`.
#[derive(Debug, Clone)]
pub struct Def {
    pub qualname: String,
    pub kind: DefKind,
    pub line: usize,
    /// For classes: base-class expressions as written (`Base`, `models.Base`).
    pub bases: Vec<String>,
}

/// A name bound by an import. `module` is always absolute (plugins resolve relative imports).
/// `name` is `Some` for `from module import name`.
#[derive(Debug, Clone)]
pub struct Import {
    pub local: String,
    pub module: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Call {
    /// Qualname of the enclosing function/method; `None` at module level.
    pub enclosing: Option<String>,
    /// Callee expression as written, e.g. `helper`, `u.other`, `self.step`.
    pub callee: String,
    pub line: usize,
}

#[derive(Debug, Clone, Default)]
pub struct FileFacts {
    pub defs: Vec<Def>,
    pub imports: Vec<Import>,
    pub calls: Vec<Call>,
}

pub trait Language: Sync {
    fn name(&self) -> &'static str;
    fn extensions(&self) -> &'static [&'static str];
    /// Dotted module name for a repo-relative path.
    fn module_name(&self, rel_path: &str) -> String;
    fn extract(&self, rel_path: &str, src: &str) -> FileFacts;
    /// Names that are always in scope without an import.
    fn is_builtin(&self, name: &str) -> bool;
}

static PYTHON: python::Python = python::Python;
static RUST: rust::Rust = rust::Rust;

pub fn for_path(rel_path: &str) -> Option<&'static dyn Language> {
    let ext = rel_path.rsplit_once('.')?.1;
    [&PYTHON as &dyn Language, &RUST]
        .into_iter()
        .find(|l| l.extensions().contains(&ext))
}
