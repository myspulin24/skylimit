//! Manual smoke run against real data: reads `~/.claude/projects` (read-only),
//! writes a throwaway database and prints the first full snapshot as JSON.
//!
//! ```text
//! cargo run --release -p usage-core --example snapshot -- [--codex] [--db <path>]
//! ```
//!
//! Without `--codex` the Codex app-server is not started. The ČNB rate is
//! fetched once (network).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use usage_core::{CodexSource, Config, Core};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .expect("home directory");
    let tmp = std::env::temp_dir().join("skylimit-smoke");
    let db = args
        .iter()
        .position(|a| a == "--db")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .unwrap_or_else(|| tmp.join(format!("history-{}.sqlite", std::process::id())));

    let mut config = Config::new(home.join(".claude").join("projects"), db.clone(), tmp.join("diag"));
    config.t3_logs_dir = Some(home.join(".t3").join("userdata").join("logs").join("provider"));
    if !args.iter().any(|a| a == "--codex") {
        config.codex = CodexSource::Disabled;
    }
    let started = Instant::now();
    let handle = Core::start(config);
    let mut last_report = Instant::now();
    loop {
        let s = handle.snapshot();
        if s.backfill.done {
            break;
        }
        if last_report.elapsed() > Duration::from_secs(5) {
            eprintln!("backfill {}/{} files", s.backfill.files_done, s.backfill.files_total);
            last_report = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    eprintln!("backfill done in {:.1} s (db {})", started.elapsed().as_secs_f64(), db.display());
    // Let Codex and the ČNB fetch land.
    std::thread::sleep(Duration::from_secs(3));
    println!("{}", serde_json::to_string_pretty(&handle.snapshot()).unwrap());
    handle.shutdown();
}
