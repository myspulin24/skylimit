//! Manual smoke run against real data: reads `~/.claude/projects` (read-only),
//! writes a throwaway database and prints the first full snapshot as JSON,
//! then a summary of the History tab's 90 days on stderr.
//!
//! ```text
//! cargo run --release -p usage-core --example snapshot -- [--codex] [--db <path>]
//! ```
//!
//! Without `--codex` the Codex app-server is not started. The ČNB rate is
//! fetched once (network).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use usage_core::{claude_config, CodexSource, Config, Core, HistoryFilter, HistoryRange};

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
    config.t3_userdata_dir = Some(home.join(".t3").join("userdata"));
    let config_dir = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from);
    config.claude_config_file = Some(claude_config::path(config_dir.as_deref(), &home));
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
    if let Some(h) = handle.history(HistoryRange::Quarter, HistoryFilter::All) {
        let m = |t: u64| format!("{:.2}M", t as f64 / 1e6);
        eprintln!(
            "history 90 d: {:.2} {:?}, {} tokens, {} active days, first data {:?}, cache saved {:.2}",
            h.total_display, h.currency, m(h.total_tokens), h.active_buckets, h.first_data_at, h.cache_savings_display
        );
        for k in &h.kinds {
            eprintln!("  kind {:?}: {} = {:.2}", k.kind, m(k.tokens), k.display);
        }
        for r in h.by_model.iter().take(8) {
            eprintln!("  model {}: {} = {:.2}", r.key, m(r.tokens), r.display);
        }
        for r in &h.by_source {
            eprintln!("  source {}: {} = {:.2}", r.key, m(r.tokens), r.display);
        }
    }
    // Accounts without the e-mail, which has no business in a terminal log.
    let claude = config_dir_account(&home);
    eprintln!("claude account: plan {:?}, e-mail known {}", claude.as_ref().and_then(|a| a.organization_type.clone()), claude.is_some_and(|a| a.email.is_some()));
    let codex = handle.codex_account();
    eprintln!("codex account: {:?} {:?}, e-mail known {}", codex.as_ref().map(|a| a.kind.clone()), codex.as_ref().and_then(|a| a.plan.clone()), codex.is_some_and(|a| a.email.is_some()));
    handle.shutdown();
}

fn config_dir_account(home: &std::path::Path) -> Option<usage_core::ClaudeAccount> {
    let config_dir = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from);
    claude_config::account(&claude_config::path(config_dir.as_deref(), home))
}
