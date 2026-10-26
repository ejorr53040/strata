//! In-daemon latency: `cargo test --release -p strata-core --test bench -- --ignored --nocapture`

#[path = "support/big_repo.rs"]
mod big_repo;

use serde_json::json;
use std::time::{Duration, Instant};
use strata_core::snapshot::Snapshot;
use strata_core::walk::WalkOptions;

#[test]
#[ignore = "benchmark: run with --release --ignored"]
fn warm_queries_are_under_1ms_p50_and_5ms_p99_on_100k_lines() {
    let dir = tempfile::tempdir().unwrap();
    let lines = big_repo::write(dir.path());
    assert!(lines >= 100_000, "{lines} lines");
    let start = Instant::now();
    let snap = Snapshot::build(dir.path(), &WalkOptions::default());
    let built = start.elapsed();
    let mut times: Vec<Duration> = Vec::with_capacity(10_000);
    for n in 0..10_000 {
        let (i, k) = (n % big_repo::MODULES, n % big_repo::FNS);
        let lang = if n % 2 == 0 { "pkg" } else { "gen" };
        let symbol = format!("{lang}.m{i}.f{i}_{k}");
        let (tool, args) = match n % 4 {
            0 => ("symbols", json!({"query": format!("f{i}_{k}")})),
            1 => ("callers", json!({"symbol": symbol})),
            2 => ("callees", json!({"symbol": symbol})),
            _ => ("references", json!({"symbol": symbol})),
        };
        let t = Instant::now();
        let out = snap.call(tool, &args).unwrap();
        let text = serde_json::to_string(&out).unwrap();
        times.push(t.elapsed());
        assert!(text.len() > 2, "{tool} {args} answered nothing");
    }
    times.sort();
    let (p50, p99) = (
        big_repo::percentile(&times, 0.5),
        big_repo::percentile(&times, 0.99),
    );
    println!("{lines} lines, build {built:?}, p50 {p50:?}, p99 {p99:?}");
    assert!(p50 < Duration::from_millis(1), "p50 {p50:?}");
    assert!(p99 < Duration::from_millis(5), "p99 {p99:?}");
}
