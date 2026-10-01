//! Language plugins: each language turns source text into language-neutral [`FileFacts`];
//! the resolver in [`crate::graph`] works only on those facts.

use crate::graph::EdgeKind;

pub mod javascript;
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
    Module,
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
            DefKind::Module => "module",
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
    /// Declared type as a dotted path, generics and references stripped: a field's type, a
    /// function's return type, a const's type. `Self` is kept as written.
    pub ty: Option<String>,
    /// For items of an `impl Type { .. }` block: the type path as written, so an impl outside
    /// the type's own module can still be found through the type.
    pub owner: Option<String>,
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
    /// The receiver's root (`x` in `x.a.m()`) is a local binding, so module names of the same
    /// spelling must not be tried.
    pub recv_local: bool,
    /// Callee expression as written, e.g. `helper`, `u.other`, `self.step`.
    pub callee: String,
    pub line: usize,
    /// A bare name that a local binding of the same name might shadow. The call is to the
    /// function of that name when one is in scope (`let x = x();`), else to a closure we
    /// cannot see, in which case it is dropped.
    pub local: bool,
}

/// What a local binding's type is known from.
#[derive(Debug, Clone)]
pub enum TypeHint {
    /// `x: Type` (a parameter or annotated `let`): a type path.
    Declared(String),
    /// `let x = Type { .. }`: a type path.
    Struct(String),
    /// `let x = f(..)` / `let x = a.m(..)`: the return type of that callee.
    Call(String),
}

/// A local binding (parameter or `let`) and what is known about its type.
#[derive(Debug, Clone)]
pub struct Binding {
    /// Qualname of the function the binding lives in.
    pub enclosing: String,
    pub name: String,
    pub hint: TypeHint,
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
    /// See [`Call::recv_local`].
    pub recv_local: bool,
    /// What the plugin knows: `Type`, `Path` (a prefix of a longer path), `Value`, `Field`.
    /// The resolver refines it from what the name turns out to be.
    pub kind: EdgeKind,
}

#[derive(Debug, Clone, Default)]
pub struct FileFacts {
    pub bindings: Vec<Binding>,
    pub refs: Vec<Ref>,
    pub defs: Vec<Def>,
    pub imports: Vec<Import>,
    pub calls: Vec<Call>,
    /// The file's top-level definitions live in one scope shared with every other such file
    /// (a classic `<script>`): a bare name not found here may be defined in any of them.
    pub shares_globals: bool,
}

/// What a reader of a definition wants besides its name: its signature and summary line.
#[derive(Debug, Clone)]
pub struct Described {
    /// Same line as the matching [`Def`]; `name` is its last qualname segment.
    pub line: usize,
    pub name: String,
    pub sig: String,
    pub doc: Option<String>,
}

pub trait Language: Sync {
    /// Signatures and doc summaries for every definition in a file. A `<module>` entry at
    /// line 1 carries the file's own doc (`//!`).
    fn describe(&self, _rel_path: &str, _src: &str) -> Vec<Described> {
        Vec::new()
    }

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
static JAVASCRIPT: javascript::Javascript = javascript::Javascript;

pub fn for_path(rel_path: &str) -> Option<&'static dyn Language> {
    let ext = rel_path.rsplit_once('.')?.1;
    [&PYTHON as &dyn Language, &RUST, &JAVASCRIPT]
        .into_iter()
        .find(|l| l.extensions().contains(&ext))
}
