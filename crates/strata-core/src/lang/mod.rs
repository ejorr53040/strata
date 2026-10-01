//! Language plugins: each language turns source text into language-neutral [`FileFacts`];
//! the resolver in [`crate::graph`] works only on those facts.

use crate::graph::EdgeKind;

pub mod python;
pub mod rust;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefKind {
    Function,
    Class,
    Struct,
    Enum,
    Trait,
    TypeAlias,
    Field,
    Variant,
    Const,
    Static,
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
            DefKind::TypeAlias => "type",
            DefKind::Field => "field",
            DefKind::Variant => "variant",
            DefKind::Const => "const",
            DefKind::Static => "static",
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
    /// A bare name that a local binding of the same name might shadow. The call is to the
    /// function of that name when one is in scope (`let x = x();`), else to a closure we
    /// cannot see, in which case it is dropped.
    pub local: bool,
}

/// A non-call use of a name: a type in a signature, a field access, a path segment...
/// `expr` is spelled like a callee (`a.b.C`, `self.field`); the resolver decides what it names.
#[derive(Debug, Clone)]
pub struct Ref {
    /// Qualname of the innermost enclosing item (fn, impl type, struct...), if any.
    pub enclosing: Option<String>,
    pub expr: String,
    pub line: usize,
    /// A name that is also bound locally. Kept only if it resolves to a variant or constant
    /// (a pattern `Round =>` after `use Kind::*`), never to a function a local shadows.
    pub local: bool,
    /// What the plugin knows: `Type`, `Path` (a prefix of a longer path), `Value`, `Field`.
    /// The resolver refines it from what the name turns out to be.
    pub kind: EdgeKind,
}

#[derive(Debug, Clone, Default)]
pub struct FileFacts {
    pub refs: Vec<Ref>,
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
    /// A member that no indexed definition has (a method on a receiver of unknown type, or
    /// `Type::name` where `Type` has no such member) cannot reach indexed code when the
    /// language is statically typed: it is std, a dependency, or derived, so `external`.
    /// Dynamic languages keep it `unresolved` (the member may live in code we cannot see).
    fn unindexed_members_are_external(&self) -> bool {
        false
    }
}

static PYTHON: python::Python = python::Python;
static RUST: rust::Rust = rust::Rust;

pub fn for_path(rel_path: &str) -> Option<&'static dyn Language> {
    let ext = rel_path.rsplit_once('.')?.1;
    [&PYTHON as &dyn Language, &RUST]
        .into_iter()
        .find(|l| l.extensions().contains(&ext))
}
