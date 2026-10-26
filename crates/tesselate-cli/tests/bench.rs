//! CLI round trip: `cargo test --release -p tesselate-cli --test bench -- --ignored --nocapture`

#[path = "../../tesselate-core/tests/support/big_repo.rs"]
mod big_repo;
mod common;

use std::time::{Duration, Instant};

#[test]
#[ignore = "benchmark: run with --release --ignored"]
fn warm_cli_round_trip_is_under_10ms_on_100k_lines() {
    let (repo, runtime) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    big_repo::write(repo.path());
    let run = |symbol: &str| {
        let out = common::tesselate_in_runtime(runtime.path())
            .args(["callers", symbol])
            .current_dir(repo.path())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(!out.stdout.is_empty(), "no callers of {symbol}");
    };
    run("pkg.m0.f0_0"); // starts the daemon and builds the index
    let mut times: Vec<Duration> = (0..200)
        .map(|n| {
            let i = n % big_repo::MODULES;
            let symbol = format!("gen.m{i}.f{i}_{}", n % big_repo::FNS);
            let t = Instant::now();
            run(&symbol);
            t.elapsed()
        })
        .collect();
    times.sort();
    let (p50, p99) = (
        big_repo::percentile(&times, 0.5),
        big_repo::percentile(&times, 0.99),
    );
    println!("round trip p50 {p50:?}, p99 {p99:?}");
    assert!(p50 < Duration::from_millis(10), "p50 {p50:?}");
}
