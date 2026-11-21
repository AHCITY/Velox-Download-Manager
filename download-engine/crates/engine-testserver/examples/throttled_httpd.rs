//! Throttled origin server for the benchmark suite (Part 5). Serves a deterministic payload
//! with per-connection pacing and optional latency injection. Runs until killed.
//!
//! Usage: throttled_httpd <size_mib> <rate_bps_per_conn> [delay_ms]
//! Prints the served URL on stdout, then sleeps.
//!
//! NOTE on fairness: `rate_bps` paces EACH connection; multi-connection tools (velox -cN,
//! aria2 -xN) therefore multiply their aggregate. That is exactly what the multi-connection
//! benchmark is meant to measure — quote per-connection rate alongside results.

use engine_testserver::{Behavior, TestServer};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let size_mib = args
        .get(1)
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(64);
    let rate: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
    let delay: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);

    let mut b = Behavior::new(size_mib * 1024 * 1024);
    b.slow_bps = rate;
    if delay > 0 {
        b.delay_ms = delay;
    }
    let server = TestServer::start(b);
    println!("{}", server.url("/file"));
    use std::io::Write;
    std::io::stdout().flush().unwrap();
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}
