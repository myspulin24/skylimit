//! End-to-end (R7): the real `Core` over a temp copy of Claude JSONL fixtures
//! and a fake `codex app-server` (Node script answering with the live
//! fixtures), with an injected clock. No network: FX is offline.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use usage_core::*;

/// 2026-10-28 12:20 UTC, before the fixture's 5h reset (15:52:56 UTC).
const T0: i64 = 1_793_190_000;
const RESET_5H: i64 = 1_793_202_776;
const RESET_WEEK: i64 = 1_793_789_576;
const NOTE_DOWN: &str = "Codex app-server neodpovídá, restartuji";
const NOTE_NOT_FOUND: &str = "Codex CLI nenalezen";

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

struct Env {
    tmp: tempfile::TempDir,
    projects: PathBuf,
    control: PathBuf,
    launches: PathBuf,
    requests: PathBuf,
    clock: Arc<AtomicI64>,
}

impl Env {
    fn new() -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let projects = tmp.path().join("home").join(".claude").join("projects");
        copy_dir(&manifest().join("tests/fixtures/claude/projects"), &projects);
        let control = tmp.path().join("control.json");
        fs::write(&control, "{}").unwrap();
        Env {
            projects,
            control,
            launches: tmp.path().join("launches.txt"),
            requests: tmp.path().join("requests.txt"),
            clock: Arc::new(AtomicI64::new(T0)),
            tmp,
        }
    }

    fn config(&self, codex: CodexSource) -> Config {
        let clock = self.clock.clone();
        Config {
            claude_projects_dir: self.projects.clone(),
            t3_logs_dir: None,
            claude_code_dir: None,
            claude_config_file: None,
            db_path: self.tmp.path().join("skylimit-data").join("history.sqlite"),
            diag_dir: self.tmp.path().join("diag"),
            codex,
            settings: Settings::default(),
            clock: Arc::new(move || clock.load(Ordering::SeqCst)),
            fx: FxSource::Offline,
            pricing_refresh: false,
        }
    }

    fn fake_codex(&self) -> CodexSource {
        let s = |p: &Path| p.to_string_lossy().to_string();
        CodexSource::Spawn {
            login_path: false,
            program: "node".into(),
            args: vec![
                s(&manifest().join("tests/fake-codex/server.mjs")),
                "--fixtures".into(),
                s(&manifest().join("tests/fixtures/codex")),
                "--control".into(),
                s(&self.control),
                "--launches".into(),
                s(&self.launches),
                "--requests".into(),
                s(&self.requests),
                "--events".into(),
                s(&self.tmp.path().join("events.txt")),
            ],
        }
    }

    fn set_clock(&self, t: i64) {
        self.clock.store(t, Ordering::SeqCst);
    }

    fn now(&self) -> i64 {
        self.clock.load(Ordering::SeqCst)
    }

    fn control(&self, json: &str) {
        fs::write(&self.control, json).unwrap();
    }

    fn launches(&self) -> Vec<String> {
        fs::read_to_string(&self.launches)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// Wait until the core has noticed the exit of every app-server it
    /// started, so the next clock jump lands after the restart backoff was set.
    fn wait_reaped(&self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let diag = fs::read_to_string(self.tmp.path().join("diag/diag.log")).unwrap_or_default();
            if diag.matches(" gone (").count() >= self.launches().len() {
                return;
            }
            assert!(Instant::now() < deadline, "app-server exits not noticed:\n{}", self.debug());
            std::thread::sleep(Duration::from_millis(40));
        }
    }

    /// Everything useful when a wait times out.
    fn debug(&self) -> String {
        let read = |p: &Path| fs::read_to_string(p).unwrap_or_default();
        format!(
            "launches:\n{}\nevents:\n{}\nrequests:\n{}\ndiag:\n{}\ncontrol: {}",
            read(&self.launches),
            read(&self.tmp.path().join("events.txt")),
            read(&self.requests),
            read(&self.tmp.path().join("diag/diag.log")),
            read(&self.control)
        )
    }
}

fn wait_for(h: &CoreHandle, env: &Env, what: &str, pred: impl Fn(&Snapshot) -> bool) -> Snapshot {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let s = h.snapshot();
        if pred(&s) {
            return s;
        }
        if Instant::now() > deadline {
            panic!(
                "timed out waiting for {what}; last snapshot:\n{}\n{}",
                serde_json::to_string_pretty(&s).unwrap(),
                env.debug()
            );
        }
        std::thread::sleep(Duration::from_millis(40));
    }
}

fn wait_until(env: &Env, what: &str, cond: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}\n{}", env.debug());
        std::thread::sleep(Duration::from_millis(40));
    }
}

fn codex_limits(s: &Snapshot) -> &ProviderLimits {
    s.limits_for(Provider::Codex).unwrap()
}

fn october_codex_tokens() -> u64 {
    let v: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(manifest().join("tests/fixtures/codex/usage-account.json")).unwrap(),
    )
    .unwrap();
    v["result"]["dailyUsageBuckets"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|b| b["startDate"].as_str().unwrap().starts_with("2026-10"))
        .map(|b| b["tokens"].as_u64().unwrap())
        .sum()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9 * b.abs().max(1.0)
}

#[cfg(windows)]
fn pid_alive(pid: &str) -> bool {
    let out = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).contains(&format!("\"{pid}\""))
}

/// Claude fixture expectations for October 2026 (USD, LiteLLM 2026-10-28):
/// - msg_a1 opus-5-5 (2 lines, final output 3000 wins): 1000×4e-6 + 3000×2e-5 + 50000×2e-7 = 0.074
/// - msg_a2 opus-5-5: 500×4e-6 + 1000×2e-5 + 20000×2e-7 + 4000×8e-6 (1h write) = 0.058
/// - msg_a3 claude-unreleased-9: unpriced, 200 tokens
/// - msg_b2 haiku-4-5: 2000×1e-6 + 1000×5e-6 + 1 web search × 0.01 = 0.017
/// - msg_c1 opus-4-7 (subagent file, claude-desktop): 100×5e-6 + 200×2.5e-5 = 0.0055
/// - msg_b1 is in September and does not count.
const CLAUDE_OCT_USD: f64 = 0.1545;
const CLAUDE_OCT_TOKENS: u64 = 54_000 + 25_500 + 200 + 3_000 + 300;

#[test]
fn e2e_core_with_fake_codex_server() {
    let env = Env::new();
    let h = Core::start(env.config(env.fake_codex()));

    // ---- normal operation
    let s = wait_for(&h, &env, "backfill + codex known + estimate", |s| {
        s.backfill.done
            && s.limits.len() == 2
            && codex_limits(s).state == LimitsState::Known
            && s.score_for(Provider::Codex).map(|c| c.state) == Some(ScoreState::Estimate)
    });
    assert_eq!(s.month, "2026-10");
    assert_eq!(s.currency, Currency::Eur);
    assert_eq!(s.backfill.files_total, 3);
    assert_eq!(s.backfill.files_done, 3);

    // Claude limits are off by decision; never Unknown, never confused.
    let claude = s.limits_for(Provider::Claude).unwrap();
    assert_eq!(s.limits[0].provider, Provider::Claude);
    assert_eq!(claude.state, LimitsState::Off);
    assert!(claude.windows.is_empty());
    assert_eq!(claude.note.as_deref(), Some(CLAUDE_LIMITS_NOTE));

    let codex = codex_limits(&s);
    assert_eq!(codex.governing, Some(0.0));
    assert_eq!(codex.note, None);
    assert_eq!(codex.windows[0].kind, WindowKind::FiveHour);
    assert_eq!(codex.windows[0].label, "5h");
    assert_eq!(codex.windows[0].percent, Some(0.0));
    assert_eq!(codex.windows[0].resets_at, Some(RESET_5H));
    assert_eq!(codex.windows[0].fetched_at, Some(T0));
    assert_eq!(codex.windows[1].kind, WindowKind::Week);
    assert_eq!(codex.windows[1].resets_at, Some(RESET_WEEK));

    let cs = s.score_for(Provider::Claude).unwrap();
    assert!(close(cs.usd, CLAUDE_OCT_USD), "claude usd {}", cs.usd);
    assert_eq!(cs.tokens, CLAUDE_OCT_TOKENS);
    assert_eq!(cs.unpriced_tokens, 200);
    assert_eq!(cs.state, ScoreState::Partial);
    let xs = s.score_for(Provider::Codex).unwrap();
    let codex_tokens = october_codex_tokens();
    assert_eq!(xs.tokens, codex_tokens);
    assert!(close(xs.usd, codex_tokens as f64 * 5.7e-6), "codex usd {}", xs.usd);
    let eur_per_usd = 21.834 / 24.340;
    assert!(close(s.total.display, (CLAUDE_OCT_USD + xs.usd) * eur_per_usd));
    assert_eq!(s.total.multiple, None);

    assert_eq!(s.mood_for(Provider::Claude), Some(Mood::Sleep));
    assert_eq!(s.mood_for(Provider::Codex), Some(Mood::Calm));
    assert_eq!(s.tray, Tray::Green);
    assert_eq!(s.daily.len(), 28);
    assert!(s.daily.iter().any(|d| d.date == "2026-10-20" && d.claude_display > 0.0));
    let sources: Vec<&str> = s.by_source.iter().map(|r| r.key.as_str()).collect();
    for k in ["cli", "sdk-ts", "claude-desktop", "codex"] {
        assert!(sources.contains(&k), "bySource lacks {k}: {sources:?}");
    }
    let projects: Vec<&str> = s.by_project.iter().map(|r| r.key.as_str()).collect();
    assert!(projects.contains(&"alpha") && projects.contains(&"beta"));
    assert!(s.by_model.iter().any(|r| r.key == "claude-unreleased-9" && r.usd == 0.0 && r.tokens == 200));
    let claude_streak = s.streaks.iter().find(|x| x.provider == Provider::Claude).unwrap();
    assert_eq!((claude_streak.current, claude_streak.longest), (0, 2));
    let codex_streak = s.streaks.iter().find(|x| x.provider == Provider::Codex).unwrap();
    assert_eq!((codex_streak.current, codex_streak.longest), (2, 9));
    assert!(s.celebration.is_none());

    // No message text anywhere in what leaves the core or in diagnostics.
    assert!(!serde_json::to_string(&s).unwrap().contains("SECRET"));

    // JSON-RPC handshake as documented.
    let requests = fs::read_to_string(&env.requests).unwrap();
    let methods: Vec<serde_json::Value> = requests.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(methods[0]["method"], "initialize");
    assert_eq!(methods[0]["params"]["clientInfo"]["name"], "skylimit");
    assert_eq!(methods[1]["method"], "initialized");
    assert_eq!(methods[1]["hasId"], false);
    assert!(methods.iter().any(|m| m["method"] == "account/rateLimits/read" && m["params"].is_null()));
    assert!(methods.iter().any(|m| m["method"] == "account/usage/read" && m["params"].is_null()));

    // ---- a new JSONL line shows up quickly (notify, not the 30 s poll) and
    // wakes Claude up. Design target: within 1 s; asserted with slack.
    let appended = Instant::now();
    {
        let mut f = fs::OpenOptions::new()
            .append(true)
            .open(env.projects.join("C--dev-alpha/aaaa-session.jsonl"))
            .unwrap();
        writeln!(
            f,
            r#"{{"type":"assistant","message":{{"model":"claude-opus-5-5","id":"msg_live1","usage":{{"input_tokens":10,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":20}}}},"requestId":"req_live1","timestamp":"2026-10-28T12:19:00.000Z","entrypoint":"cli","cwd":"C:\\dev\\alpha"}}"#
        )
        .unwrap();
    }
    let s = wait_for(&h, &env, "live JSONL line", |s| {
        s.score_for(Provider::Claude).unwrap().tokens == CLAUDE_OCT_TOKENS + 30
    });
    let latency = appended.elapsed();
    eprintln!("new JSONL line visible after {latency:?}");
    assert!(latency < Duration::from_secs(3), "new line took {latency:?}");
    assert!(close(s.score_for(Provider::Claude).unwrap().usd, CLAUDE_OCT_USD + 10.0 * 4e-6 + 20.0 * 2e-5));
    assert_eq!(s.mood_for(Provider::Claude), Some(Mood::Focus));
    assert!(s.celebration.is_none(), "30 tokens is no daily record");

    // ---- account/rateLimits/updated triggers a read after the 60 s gap
    // (the regular read would only come at T0 + 120).
    env.control(r#"{"primaryUsedPercent": 80, "pushUpdate": 1}"#);
    std::thread::sleep(Duration::from_millis(400));
    env.set_clock(T0 + 61);
    let s = wait_for(&h, &env, "codex 80 % after notification", |s| codex_limits(s).governing == Some(80.0));
    assert_eq!(s.mood_for(Provider::Codex), Some(Mood::Sweat));
    assert_eq!(s.tray, Tray::Orange);

    // ---- the app-server dies: note at once, values stay Known for 10 min
    env.control(r#"{"primaryUsedPercent": 80, "die": true}"#);
    let s = wait_for(&h, &env, "codex down note", |s| codex_limits(s).note.as_deref() == Some(NOTE_DOWN));
    assert_eq!(codex_limits(&s).state, LimitsState::Known);
    let pid_before = env.launches().last().cloned().unwrap();

    // 11 minutes later: Stale, dozing, dim tray (restarts keep failing).
    env.set_clock(T0 + 61 + 660);
    let s = wait_for(&h, &env, "codex stale", |s| codex_limits(s).state == LimitsState::Stale);
    assert_eq!(codex_limits(&s).windows[0].state, WindowState::Stale);
    assert_eq!(codex_limits(&s).windows[0].percent, Some(80.0));
    assert_eq!(codex_limits(&s).governing, Some(80.0));
    assert_eq!(s.mood_for(Provider::Codex), Some(Mood::Dozing));
    assert_eq!(s.tray, Tray::OrangeDim);
    // A restart was attempted (backoff 5 s passed) and failed again.
    wait_until(&env, "first restart attempt", || env.launches().len() >= 2);
    env.wait_reaped();

    // The 5h reset passes while Stale: no percent, it stops governing.
    env.set_clock(RESET_5H + 60);
    wait_until(&env, "second restart attempt", || env.launches().len() >= 3);
    env.wait_reaped();
    let s = wait_for(&h, &env, "5h after reset", |s| codex_limits(s).windows[0].state == WindowState::AfterReset);
    assert_eq!(codex_limits(&s).windows[0].percent, None);
    assert_eq!(codex_limits(&s).windows[1].state, WindowState::Stale);
    assert_eq!(codex_limits(&s).governing, Some(0.0));
    assert_eq!(codex_limits(&s).state, LimitsState::Stale);
    assert_eq!(s.tray, Tray::GreenDim);
    assert_eq!(s.mood_for(Provider::Codex), Some(Mood::Dozing));

    // Server healthy again: restart (backoff ≤ 15 min) → Known with a fresh read.
    env.control("{}");
    env.set_clock(RESET_5H + 60 + 16 * 60);
    let s = wait_for(&h, &env, "codex known after restart", |s| {
        codex_limits(s).state == LimitsState::Known && codex_limits(s).windows[0].fetched_at == Some(RESET_5H + 60 + 16 * 60)
    });
    assert_eq!(codex_limits(&s).note, None);
    assert_eq!(s.tray, Tray::Green);
    assert_eq!(s.mood_for(Provider::Codex), Some(Mood::Calm));
    let launches = env.launches();
    assert!(launches.len() >= 4, "expected restarts, launches: {launches:?}");
    let pid_live = launches.last().cloned().unwrap();
    assert_ne!(pid_live, pid_before);

    let diag = fs::read_to_string(env.tmp.path().join("diag/diag.log")).unwrap_or_default();
    assert!(!diag.contains("SECRET"), "diag leaked content");

    h.shutdown();
    #[cfg(windows)]
    {
        let deadline = Instant::now() + Duration::from_secs(5);
        while pid_alive(&pid_live) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(!pid_alive(&pid_live), "fake app-server survived shutdown");
    }

    // ---- restart over the same database: nothing counted twice, last Codex
    // values come back Stale, and a missing CLI says so.
    env.set_clock(env.now() + 11 * 60);
    let h = Core::start(env.config(CodexSource::Spawn {
        login_path: false,
        program: "skylimit-test-no-such-codex".into(),
        args: Vec::new(),
    }));
    let s = wait_for(&h, &env, "restart backfill + codex not found", |s| {
        s.backfill.done && codex_limits(s).note.as_deref() == Some(NOTE_NOT_FOUND)
    });
    assert_eq!(s.score_for(Provider::Claude).unwrap().tokens, CLAUDE_OCT_TOKENS + 30);
    assert_eq!(s.score_for(Provider::Codex).unwrap().state, ScoreState::Estimate);
    assert_eq!(s.score_for(Provider::Codex).unwrap().tokens, codex_tokens);
    assert_eq!(codex_limits(&s).state, LimitsState::Stale);
    assert_eq!(s.mood_for(Provider::Codex), Some(Mood::Dozing));

    // ---- "Smazat historii": tables wiped, existing transcripts read again.
    h.clear_history();
    let s = wait_for(&h, &env, "history rebuilt after clear", |s| {
        s.backfill.done && s.score_for(Provider::Claude).unwrap().tokens == CLAUDE_OCT_TOKENS + 30
    });
    assert_eq!(s.score_for(Provider::Codex).unwrap().state, ScoreState::Unknown);
    assert_eq!(s.score_for(Provider::Codex).unwrap().tokens, 0);
    h.shutdown();
}

#[test]
fn codex_cli_missing_is_unknown_and_confused() {
    let env = Env::new();
    // Production shape: the shell wrapper starts, the command inside does not exist.
    let codex = if cfg!(windows) {
        CodexSource::Spawn {
            login_path: false,
            program: "cmd.exe".into(),
            args: vec!["/C".into(), "skylimit-test-no-such-codex".into(), "app-server".into()],
        }
    } else {
        CodexSource::Spawn {
            login_path: false,
            program: "skylimit-test-no-such-codex".into(),
            args: vec!["app-server".into()],
        }
    };
    let h = Core::start(env.config(codex));
    let s = wait_for(&h, &env, "codex not found", |s| {
        s.backfill.done && s.limits.len() == 2 && codex_limits(s).note.as_deref() == Some(NOTE_NOT_FOUND)
    });
    let codex = codex_limits(&s);
    assert_eq!(codex.state, LimitsState::Unknown);
    assert_eq!(codex.windows.len(), 2);
    assert!(codex.windows.iter().all(|w| w.state == WindowState::Unknown && w.percent.is_none()));
    assert_eq!(s.mood_for(Provider::Codex), Some(Mood::Confused));
    assert_ne!(s.mood_for(Provider::Claude), Some(Mood::Confused));
    assert_eq!(s.tray, Tray::Grey);
    let xs = s.score_for(Provider::Codex).unwrap();
    assert_eq!(xs.state, ScoreState::Unknown);

    // With a Codex plan set, the missing price drops that plan from the total.
    let mut settings = h.settings();
    settings.plans.claude = Some(PlanSetting {
        amount: 0.1,
        currency: PlanCurrency::Usd,
        period: PlanPeriod::Month,
    });
    settings.plans.codex = Some(PlanSetting {
        amount: 20.0,
        currency: PlanCurrency::Usd,
        period: PlanPeriod::Month,
    });
    h.set_settings(settings.clone());
    assert_eq!(h.settings(), settings);
    let s = wait_for(&h, &env, "total multiple", |s| s.total.multiple.is_some());
    assert!(close(s.total.multiple.unwrap(), CLAUDE_OCT_USD / 0.1));
    assert_eq!(s.total.excluded_note.as_deref(), Some("Codex bez ceny, paušál nezapočten"));
    h.shutdown();
}

#[test]
fn all_limits_off_gives_mono_tray_and_calm_characters() {
    let env = Env::new();
    let h = Core::start(env.config(CodexSource::Disabled));
    let s = wait_for(&h, &env, "backfill", |s| s.backfill.done && s.limits.len() == 2);
    assert!(s.limits.iter().all(|l| l.state == LimitsState::Off));
    assert_eq!(s.tray, Tray::Mono);
    assert_eq!(s.score_for(Provider::Codex).unwrap().state, ScoreState::Off);
    // Off limits never confuse or doze; Claude sleeps (no activity for days).
    assert_eq!(s.mood_for(Provider::Claude), Some(Mood::Sleep));
    assert_eq!(s.mood_for(Provider::Codex), Some(Mood::Calm));
    // Excluding an entrypoint filters at query time.
    let mut settings = h.settings();
    settings.excluded_entrypoints = vec!["cli".into()];
    settings.currency = Currency::Czk;
    h.set_settings(settings);
    let s = wait_for(&h, &env, "excluded cli", |s| s.currency == Currency::Czk);
    let cs = s.score_for(Provider::Claude).unwrap();
    assert_eq!(cs.tokens, 3_000 + 300);
    assert!(close(cs.usd, 0.017 + 0.0055));
    assert!(close(cs.display, (0.017 + 0.0055) * 21.834));
    assert!(!s.by_source.iter().any(|r| r.key == "cli"));
    h.shutdown();
}

fn t3_event(stamp: &str, five: f64, week: f64, reset5: i64, reset7: i64) -> String {
    format!(
        r#"[{stamp}] NTIVE: {{"provider":"claudeAgent","kind":"protocol","event":{{"direction":"incoming","payload":{{"type":"rate_limit_event","rate_limit_info":{{"status":"allowed","unifiedWindows":{{"five_hour":{{"utilization":{five},"resetsAt":{reset5},"status":"allowed"}},"seven_day":{{"utilization":{week},"resetsAt":{reset7},"status":"allowed"}}}}}}}}}}}}"#
    )
}

#[test]
fn claude_limits_come_from_t3_code_logs_and_survive_a_restart() {
    let env = Env::new();
    let t3 = env.tmp.path().join("t3").join("logs").join("provider");
    fs::create_dir_all(&t3).unwrap();
    let log = t3.join("events.thread-1.log");
    let reset7 = T0 + 5 * 86_400;
    fs::write(&log, format!("{}\n", t3_event("2026-10-28T12:10:00Z", 0.42, 0.12, RESET_5H, reset7))).unwrap();

    let mut config = env.config(CodexSource::Disabled);
    config.t3_logs_dir = Some(t3.clone());
    let h = Core::start(config.clone());
    let s = wait_for(&h, &env, "claude limits known", |s| {
        s.limits_for(Provider::Claude).is_some_and(|l| l.state == LimitsState::Known)
    });
    let claude = s.limits_for(Provider::Claude).unwrap();
    assert_eq!(claude.governing, Some(42.0));
    assert_eq!(claude.note, None);
    assert_eq!(claude.windows[0].kind, WindowKind::FiveHour);
    assert_eq!(claude.windows[0].percent, Some(42.0));
    assert_eq!(claude.windows[0].resets_at, Some(RESET_5H));
    assert_eq!(claude.windows[0].fetched_at, Some(T0 - 600));
    assert_eq!(claude.windows[1].kind, WindowKind::Week);
    assert_eq!(claude.windows[1].percent, Some(12.0));
    assert_eq!(s.tray, Tray::Green);
    // Last Claude work in the fixtures is days old: limits fine, so it sleeps.
    assert_eq!(s.mood_for(Provider::Claude), Some(Mood::Sleep));

    let mut f = fs::OpenOptions::new().append(true).open(&log).unwrap();
    writeln!(f, "{}", t3_event("2026-10-28T12:15:00Z", 0.80, 0.13, RESET_5H, reset7)).unwrap();
    drop(f);
    let s = wait_for(&h, &env, "claude at 80 %", |s| {
        s.limits_for(Provider::Claude).and_then(|l| l.governing) == Some(80.0)
    });
    assert_eq!(s.mood_for(Provider::Claude), Some(Mood::Sweat));
    assert_eq!(s.tray, Tray::Orange);
    h.shutdown();

    // The logs are gone (rotated, T3 Code removed): the database keeps the reading.
    fs::remove_dir_all(env.tmp.path().join("t3")).unwrap();
    let h = Core::start(config);
    let s = wait_for(&h, &env, "restored claude limits", |s| {
        s.limits_for(Provider::Claude).and_then(|l| l.governing) == Some(80.0)
    });
    assert_eq!(s.limits_for(Provider::Claude).unwrap().state, LimitsState::Known);
    assert_eq!(s.limits_for(Provider::Claude).unwrap().windows[0].fetched_at, Some(T0 - 300));
    h.shutdown();
}

fn count_requests(env: &Env, method: &str) -> usize {
    fs::read_to_string(&env.requests)
        .unwrap_or_default()
        .lines()
        .filter(|l| l.contains(&format!("\"method\":\"{method}\"")))
        .count()
}

#[test]
fn refresh_reads_codex_and_t3_code_now() {
    let env = Env::new();
    let t3 = env.tmp.path().join("t3");
    fs::create_dir_all(&t3).unwrap();
    let log = t3.join("events.thread-1.log");
    fs::write(&log, format!("{}\n", t3_event("2026-10-28T12:10:00Z", 0.10, 0.10, RESET_5H, T0 + 86_400))).unwrap();
    let mut config = env.config(env.fake_codex());
    config.t3_logs_dir = Some(t3);
    let h = Core::start(config);
    wait_for(&h, &env, "codex and claude known", |s| {
        s.limits.len() == 2
            && codex_limits(s).state == LimitsState::Known
            && s.limits_for(Provider::Claude).is_some_and(|l| l.state == LimitsState::Known)
    });
    let limits_reads = count_requests(&env, "account/rateLimits/read");
    let usage_reads = count_requests(&env, "account/usage/read");
    assert!(limits_reads >= 1 && usage_reads >= 1);

    // The schedule would wait 2 and 5 minutes; a refresh only waits the 10 s gap.
    env.set_clock(env.now() + 11);
    let mut f = fs::OpenOptions::new().append(true).open(&log).unwrap();
    writeln!(f, "{}", t3_event("2026-10-28T12:15:00Z", 0.55, 0.20, RESET_5H, T0 + 86_400)).unwrap();
    drop(f);
    let asked = Instant::now();
    h.refresh();
    wait_until(&env, "codex re-read after refresh", || {
        count_requests(&env, "account/rateLimits/read") > limits_reads
            && count_requests(&env, "account/usage/read") > usage_reads
    });
    wait_for(&h, &env, "claude 55 % after refresh", |s| {
        s.limits_for(Provider::Claude).and_then(|l| l.governing) == Some(55.0)
    });
    // The Claude limits reader polls every 5 s; the refresh woke it before that.
    assert!(asked.elapsed() < Duration::from_secs(4), "refresh took {:?}", asked.elapsed());
    h.shutdown();
}

/// What Claude Code hands its status line command (trimmed to what matters).
fn status_line_input(session: &str, api_ms: u64, five: f64, week: f64) -> String {
    format!(
        r#"{{"session_id":"{session}","cwd":"/Users/someone/project","cost":{{"total_api_duration_ms":{api_ms}}},"rate_limits":{{"five_hour":{{"used_percentage":{five},"resets_at":{RESET_5H}}},"seven_day":{{"used_percentage":{week},"resets_at":{RESET_WEEK}}}}}}}"#
    )
}

#[test]
fn claude_limits_come_from_the_claude_code_status_line_and_the_newest_source_wins() {
    let env = Env::new();
    let t3 = env.tmp.path().join("t3");
    fs::create_dir_all(&t3).unwrap();
    let log = t3.join("events.thread-1.log");
    fs::write(&log, format!("{}\n", t3_event("2026-10-28T12:10:00Z", 0.10, 0.10, RESET_5H, RESET_WEEK))).unwrap();
    let code = env.tmp.path().join("skylimit-data").join(claude_code::DIR_NAME);
    let mut config = env.config(CodexSource::Disabled);
    config.t3_logs_dir = Some(t3);
    config.claude_code_dir = Some(code.clone());
    let h = Core::start(config);
    wait_for(&h, &env, "claude 10 % from T3 Code", |s| {
        s.limits_for(Provider::Claude).and_then(|l| l.governing) == Some(10.0)
    });

    // `skylimit statusline` as Claude Code runs it: the first run of a
    // session only remembers it, the run after the next response counts.
    let run = |api_ms, five, week| {
        let sample = claude_code::parse(&status_line_input("sess-1", api_ms, five, week)).unwrap();
        claude_code::record(&code, &sample, env.now()).unwrap()
    };
    assert!(!run(100, 50.0, 20.0).fresh);
    assert!(run(900, 63.0, 21.0).fresh);
    h.refresh();
    let s = wait_for(&h, &env, "claude 63 % from Claude Code", |s| {
        s.limits_for(Provider::Claude).and_then(|l| l.governing) == Some(63.0)
    });
    let claude = s.limits_for(Provider::Claude).unwrap();
    assert_eq!(claude.state, LimitsState::Known);
    assert_eq!(claude.windows[0].fetched_at, Some(env.now()), "newer than T3 Code's reading");
    assert_eq!(claude.windows[1].percent, Some(21.0));
    h.shutdown();
}

/// Claude Code's `.claude.json` with its usage cache (trimmed to what matters).
fn claude_config(fetched_at: i64, five: f64, week: f64, fable: f64) -> String {
    let iso = |t: i64| chrono_free_iso(t);
    serde_json::json!({
        "projects": { "C:/work": { "history": [] } },
        "oauthAccount": { "accountUuid": "acc-1", "emailAddress": "jana@example.com", "organizationType": "claude_max" },
        "cachedUsageUtilization": {
            "fetchedAtMs": fetched_at * 1000,
            "accountUuid": "acc-1",
            "utilization": {
                "five_hour": { "utilization": five, "resets_at": iso(RESET_5H) },
                "seven_day": { "utilization": week, "resets_at": iso(RESET_WEEK) },
                "limits": [
                    { "kind": "weekly_scoped", "group": "weekly", "percent": fable, "resets_at": iso(RESET_WEEK),
                      "scope": { "model": { "display_name": "Fable" } } }
                ]
            }
        }
    })
    .to_string()
}

/// RFC 3339 in UTC without pulling chrono into the test.
fn chrono_free_iso(t: i64) -> String {
    let days = t.div_euclid(86_400);
    let secs = t.rem_euclid(86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}+00:00", secs / 3600, secs / 60 % 60, secs % 60)
}

#[test]
fn the_fable_limit_comes_from_claude_codes_cache_below_the_week_and_survives_a_restart() {
    let env = Env::new();
    let file = env.tmp.path().join("home").join(claude_config::FILE_NAME);
    fs::write(&file, claude_config(T0 - 120, 30.0, 40.0, 12.5)).unwrap();
    let mut config = env.config(CodexSource::Disabled);
    config.claude_config_file = Some(file.clone());
    let h = Core::start(config.clone());
    let s = wait_for(&h, &env, "claude limits from the cache", |s| {
        s.limits_for(Provider::Claude).is_some_and(|l| l.windows.len() == 3)
    });
    let claude = s.limits_for(Provider::Claude).unwrap();
    assert_eq!(claude.state, LimitsState::Known);
    assert_eq!(claude.governing, Some(40.0), "Fable never governs");
    let fable = &claude.windows[2];
    assert_eq!(fable.kind, WindowKind::WeekModel);
    assert_eq!(fable.label, "Fable");
    assert_eq!(fable.percent, Some(12.5));
    assert_eq!(fable.resets_at, Some(RESET_WEEK));
    assert_eq!(fable.fetched_at, Some(T0 - 120));

    // Claude Code refreshes its cache: Skylimit notices the changed file.
    fs::write(&file, claude_config(T0 - 60, 31.0, 41.0, 20.0)).unwrap();
    h.refresh();
    wait_for(&h, &env, "fable at 20 %", |s| {
        s.limits_for(Provider::Claude).is_some_and(|l| l.windows.get(2).and_then(|w| w.percent) == Some(20.0))
    });
    h.shutdown();

    // Claude Code signed out (cache gone): the database keeps the last values.
    fs::write(&file, "{}").unwrap();
    let h = Core::start(config);
    let s = wait_for(&h, &env, "restored fable", |s| {
        s.limits_for(Provider::Claude).is_some_and(|l| l.windows.len() == 3)
    });
    let claude = s.limits_for(Provider::Claude).unwrap();
    assert_eq!(claude.windows[1].percent, Some(41.0));
    assert_eq!((claude.windows[2].percent, claude.windows[2].fetched_at), (Some(20.0), Some(T0 - 60)));
    assert_eq!(claude_config::account(&file), None);
    h.shutdown();
}

#[test]
fn the_codex_account_comes_from_the_app_server() {
    let env = Env::new();
    let h = Core::start(env.config(env.fake_codex()));
    wait_until(&env, "codex account", || h.codex_account().is_some_and(|a| a.reset_credits.is_some()));
    let a = h.codex_account().unwrap();
    assert_eq!(a.kind, "chatgpt");
    assert_eq!(a.email.as_deref(), Some("tester@example.com"));
    assert_eq!(a.plan.as_deref(), Some("team"));
    assert_eq!(a.workspace.as_deref(), Some("Test"));
    assert_eq!(a.cli_version.as_deref(), Some("0.0.0"));
    assert_eq!(a.reset_credits, Some(2));
    assert!(count_requests(&env, "account/read") >= 1);
    // The e-mail stays out of the snapshot the widget gets.
    assert!(!serde_json::to_string(&h.snapshot()).unwrap().contains("tester@example.com"));
    h.shutdown();
}

#[test]
fn without_t3_code_or_claude_code_the_note_points_to_settings() {
    let env = Env::new();
    let mut config = env.config(CodexSource::Disabled);
    config.t3_logs_dir = Some(env.tmp.path().join("no-t3"));
    config.claude_code_dir = Some(env.tmp.path().join("not-connected"));
    let h = Core::start(config);
    let s = wait_for(&h, &env, "claude limits off", |s| {
        s.limits_for(Provider::Claude).is_some_and(|l| l.state == LimitsState::Off)
    });
    assert_eq!(s.limits_for(Provider::Claude).unwrap().note.as_deref(), Some(CLAUDE_LIMITS_NOTE));
    h.shutdown();
}
