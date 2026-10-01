use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn strata(repo: &Path, args: &[&str]) -> Output {
    let xdg = repo.join("_xdg");
    Command::new(env!("CARGO_BIN_EXE_strata"))
        .args(args)
        .current_dir(repo)
        .env("XDG_CONFIG_HOME", xdg)
        .output()
        .unwrap()
}

fn write(repo: &Path, rel: &str, text: &str) {
    let p = repo.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

fn ok(out: &Output) -> String {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout.clone()).unwrap()
}

/// Repo with one indexed file and one skipped file per reason.
fn repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "src/app.py", "def handler():\n    pass\n");
    write(r, ".gitignore", "gen/\n");
    write(r, "gen/out.py", "def generated(): pass\n");
    write(r, ".strataignore", "scratch/\n");
    write(r, "scratch/tmp.py", "def scratchy(): pass\n");
    write(r, "node_modules/lib/index.py", "def vendored_fn(): pass\n");
    write(r, "target/x.py", "def built(): pass\n");
    write(r, "build/y.py", "def built2(): pass\n");
    write(r, ".venv/lib/z.py", "def venv_fn(): pass\n");
    write(r, "src/main.go", "package main\n");
    write(r, ".git/info/exclude", "excluded.py\n");
    write(r, "excluded.py", "def excl(): pass\n");
    fs::write(r.join("src/bad.py"), [0xff, 0xfe, 0x00, b'd']).unwrap();
    d
}

#[test]
fn files_unindexed_lists_each_skipped_file_with_its_reason() {
    let d = repo();
    let text = ok(&strata(d.path(), &["files", "--unindexed"]));
    let lines: Vec<&str> = text.lines().collect();
    for want in [
        "unindexed gen/out.py gitignore",
        "unindexed excluded.py gitignore",
        "unindexed scratch/tmp.py strataignore",
        "unindexed node_modules/lib/index.py vendored",
        "unindexed target/x.py vendored",
        "unindexed build/y.py vendored",
        "unindexed .venv/lib/z.py vendored",
        "unindexed src/main.go unsupported-language",
        "unindexed src/bad.py parse-failure",
    ] {
        assert!(lines.contains(&want), "missing `{want}` in:\n{text}");
    }
    assert!(!text.contains("src/app.py"), "{text}");
}

#[test]
fn ignored_and_vendored_files_are_not_parsed() {
    let d = repo();
    let text = ok(&strata(d.path(), &["symbols", ""]));
    for hidden in [
        "generated",
        "scratchy",
        "vendored_fn",
        "built",
        "venv_fn",
        "excl",
    ] {
        assert!(!text.contains(hidden), "{hidden} leaked:\n{text}");
    }
    assert!(text.contains("function handler src/app.py:1"), "{text}");
}

#[test]
fn config_include_reincludes_a_skipped_directory() {
    let d = repo();
    write(
        d.path(),
        ".strata/config.toml",
        "[index]\ninclude = [\"node_modules\", \"gen\"]\n",
    );
    let un = ok(&strata(d.path(), &["files", "--unindexed"]));
    assert!(!un.contains("node_modules/lib/index.py"), "{un}");
    assert!(!un.contains("gen/out.py"), "{un}");
    let text = ok(&strata(d.path(), &["symbols", "vendored_fn"]));
    assert!(text.contains("node_modules/lib/index.py:1"), "{text}");
    let text = ok(&strata(d.path(), &["symbols", "generated"]));
    assert!(text.contains("gen/out.py:1"), "{text}");
}
