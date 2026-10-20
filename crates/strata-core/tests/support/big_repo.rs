//! A deterministic ~100k-line Python + Rust repo for latency benchmarks.

use std::fmt::Write;
use std::path::Path;

pub const MODULES: usize = 160;
pub const FNS: usize = 30;

/// Write `MODULES` Python modules (`pkg.mI`) and `MODULES` Rust modules (`gen.mI`), each
/// with `FNS` functions that call into a neighbouring module. Returns the line count.
pub fn write(root: &Path) -> usize {
    write_modules(root, MODULES)
}

/// [`write`] with `modules` modules per language.
pub fn write_modules(root: &Path, modules: usize) -> usize {
    std::fs::create_dir_all(root.join("pkg")).unwrap();
    std::fs::create_dir_all(root.join("gen/src")).unwrap();
    let mut lines = 0;
    let mut lib = String::new();
    for i in 0..modules {
        let j = (i * 7 + 3) % modules;
        let (mut py, mut rs) = (String::new(), String::new());
        for k in 0..FNS {
            writeln!(py, "from pkg.m{j} import f{j}_{k}").unwrap();
            writeln!(rs, "use crate::m{j}::f{j}_{k};").unwrap();
        }
        for k in 0..FNS {
            let k2 = (k + 1) % FNS;
            write!(
                py,
                "\n\ndef f{i}_{k}(x):\n    \"\"\"Step {k} of module {i}.\"\"\"\n    y = x + {k}\n    \
                 f{j}_{k}(y)\n    if y > 3:\n        y = f{i}_{k2}(y - 1)\n    return y\n"
            )
            .unwrap();
            write!(
                rs,
                "\n/// Step {k} of module {i}.\npub fn f{i}_{k}(x: u32) -> u32 {{\n    let y = x + {k};\n    \
                 f{j}_{k}(y);\n    if y > 3 {{\n        return f{i}_{k2}(y - 1);\n    }}\n    y\n}}\n"
            )
            .unwrap();
        }
        lines += py.lines().count() + rs.lines().count();
        std::fs::write(root.join(format!("pkg/m{i}.py")), py).unwrap();
        std::fs::write(root.join(format!("gen/src/m{i}.rs")), rs).unwrap();
        writeln!(lib, "pub mod m{i};").unwrap();
    }
    std::fs::write(root.join("pkg/__init__.py"), "").unwrap();
    std::fs::write(root.join("gen/src/lib.rs"), lib).unwrap();
    lines
}

pub fn percentile(sorted: &[std::time::Duration], p: f64) -> std::time::Duration {
    sorted[((sorted.len() - 1) as f64 * p) as usize]
}
