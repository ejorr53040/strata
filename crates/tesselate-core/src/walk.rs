//! Repo walker: decides which source files are indexed and why each other one is not.

use ignore::WalkBuilder;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Directory names never indexed unless re-included via `index.include`.
/// Hidden directories (leading `.`) are vendored too.
const VENDORED_DIRS: &[&str] = &["target", "node_modules", "build"];

/// Extensions the walker recognises as source code but has no language plugin for yet.
const UNSUPPORTED_SOURCE: &[&str] = &[
    "c", "h", "cc", "cpp", "cxx", "hpp", "hh", "js", "mjs", "ts", "tsx", "jsx", "go", "java",
    "html", "css",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    Gitignore,
    Tesselateignore,
    Vendored,
    UnsupportedLanguage,
    ParseFailure,
}

impl Reason {
    pub fn label(self) -> &'static str {
        match self {
            Reason::Gitignore => "gitignore",
            Reason::Tesselateignore => "tesselateignore",
            Reason::Vendored => "vendored",
            Reason::UnsupportedLanguage => "unsupported-language",
            Reason::ParseFailure => "parse-failure",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct WalkOptions {
    /// Repo-relative directories (or files) to index even if ignored or vendored.
    pub include: Vec<String>,
}

impl WalkOptions {
    /// Parse the `index.include` config value: comma-separated repo-relative paths.
    pub fn from_config_value(include: &str) -> Self {
        WalkOptions {
            include: include
                .split(',')
                .map(|s| s.trim().trim_matches('/').to_string())
                .filter(|s| !s.is_empty())
                .collect(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Source {
    pub rel: String,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct Skipped {
    pub rel: String,
    pub reason: Reason,
}

#[derive(Debug, Clone, Default)]
pub struct Walk {
    /// Files with a language plugin, to parse, sorted by path.
    pub indexed: Vec<Source>,
    /// Source files that were not indexed, sorted by path.
    pub unindexed: Vec<Skipped>,
}

pub fn walk(root: &Path, opts: &WalkOptions) -> Walk {
    let all = list(root, |b| {
        b.standard_filters(false);
    });
    let not_gitignored = list(root, |b| {
        b.standard_filters(false)
            .git_ignore(true)
            .git_exclude(true)
            .require_git(false);
    });
    let not_tesselateignored = list(root, |b| {
        b.standard_filters(false)
            .add_custom_ignore_filename(".tesselateignore");
    });

    let mut out = Walk::default();
    for rel in all {
        let ext = rel.extension().and_then(|e| e.to_str()).unwrap_or("");
        let rel_str = rel.to_string_lossy();
        let supported =
            crate::lang::for_path(&rel_str).is_some() || crate::markup::is_markup(&rel_str);
        if !supported && !UNSUPPORTED_SOURCE.contains(&ext) {
            continue;
        }
        let rel_s = rel.to_string_lossy().replace('\\', "/");
        let reason = if included(&rel_s, opts) {
            None
        } else if !not_tesselateignored.contains(&rel) {
            Some(Reason::Tesselateignore)
        } else if !not_gitignored.contains(&rel) {
            Some(Reason::Gitignore)
        } else if is_vendored(&rel) {
            Some(Reason::Vendored)
        } else {
            None
        };
        let reason = match reason {
            Some(r) => Some(r),
            None if !supported => Some(Reason::UnsupportedLanguage),
            None => None,
        };
        if let Some(reason) = reason {
            out.unindexed.push(Skipped { rel: rel_s, reason });
            continue;
        }
        match std::fs::read_to_string(root.join(&rel)) {
            Ok(text) => out.indexed.push(Source { rel: rel_s, text }),
            Err(_) => out.unindexed.push(Skipped {
                rel: rel_s,
                reason: Reason::ParseFailure,
            }),
        }
    }
    out.indexed.sort_by(|a, b| a.rel.cmp(&b.rel));
    out.unindexed.sort_by(|a, b| a.rel.cmp(&b.rel));
    out
}

fn included(rel: &str, opts: &WalkOptions) -> bool {
    opts.include.iter().any(|p| {
        rel == p
            || rel
                .strip_prefix(p.as_str())
                .is_some_and(|r| r.starts_with('/'))
    })
}

fn is_vendored(rel: &Path) -> bool {
    let Some(dir) = rel.parent() else {
        return false;
    };
    dir.components().any(|c| {
        let n = c.as_os_str().to_string_lossy();
        n.starts_with('.') || VENDORED_DIRS.contains(&n.as_ref())
    })
}

/// Repo-relative paths of regular files reachable under the given walker configuration.
/// `.git` is never entered.
fn list(root: &Path, configure: impl FnOnce(&mut WalkBuilder)) -> HashSet<PathBuf> {
    let mut b = WalkBuilder::new(root);
    b.parents(false)
        .git_global(false)
        .filter_entry(|e| e.file_name() != ".git");
    configure(&mut b);
    b.build()
        .flatten()
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .filter_map(|e| e.path().strip_prefix(root).ok().map(Path::to_path_buf))
        .collect()
}
