//! Claude cost source: Claude Code transcripts `<projects>/**/*.jsonl`.
//!
//! Limits for Claude are deliberately OFF (Anthropic terms forbid third-party
//! apps from using Claude OAuth tokens), so this module only reads the
//! local transcripts and never talks to the network.
//!
//! ```text
//!  file ──BufReader──▶ complete line? ──no──▶ stop, offset stays before it
//!                          │yes
//!                          ▼
//!          contains "type":"assistant"? ──no──▶ skip (cheap prefilter)
//!                          │yes
//!                          ▼
//!       parse top level (message kept raw) ──fail──▶ diag: path, offset, len, error
//!                          │
//!          top-level type == assistant? ──no──▶ skip (nested progress lines)
//!                          │yes
//!                          ▼
//!           usage with input/output tokens? ──no──▶ health "bad" + diag keys only
//!                          │yes
//!                          ▼
//!           ClaudeRecord ──▶ batch ──▶ core (dedupe + aggregates + offset, one tx)
//! ```
//!
//! The offset only ever advances past the last `\n`-terminated line. A file
//! that shrank below the stored offset or changed identity (creation time or
//! hash of its first bytes) is read again from 0; the dedupe key
//! (`message.id` + `requestId`) keeps that from double counting. Deleted files
//! keep their aggregates.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::value::RawValue;

use crate::diag::{error_category, top_level_keys, Diag};
use crate::pricing::context_band;
use crate::{Clock, CoreMsg, Update};

/// Quiet period before a changed file is read (editors write in bursts).
const QUIET: Duration = Duration::from_millis(300);
/// Safety poll in case a file system event gets lost.
const POLL: Duration = Duration::from_secs(30);
/// Records per message to the core during a long read.
const BATCH: usize = 2000;
/// Bytes hashed to recognise a replaced file.
const PREFIX_BYTES: u64 = 4096;
/// Diagnostic lines per file read, so one broken file cannot flood the log.
const DIAG_BUDGET: u32 = 20;

const ASSISTANT_NEEDLE: &[u8] = br#""type":"assistant""#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileState {
    /// Creation time in ms, when the platform reports it.
    pub created: Option<i64>,
    pub prefix_len: u32,
    pub prefix_hash: u64,
    pub size: u64,
    /// Modification time in ms.
    pub mtime: i64,
    pub offset: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ClaudeRecord {
    pub ts: i64,
    pub hour: i64,
    pub model: String,
    pub entrypoint: String,
    pub project: String,
    pub speed: String,
    pub service_tier: String,
    /// Input-side context band in thousands (see `pricing::context_band`).
    pub band: u32,
    /// input, output, cache_read, cache_write_5m, cache_write_1h
    pub tokens: [u64; 5],
    pub web_search: u64,
    /// 64-bit FNV-1a of `message.id` + `requestId`.
    pub key: u64,
}

#[derive(Debug)]
pub(crate) struct FileBatch {
    pub epoch: u64,
    pub path: String,
    pub state: FileState,
    pub records: Vec<ClaudeRecord>,
    /// (line timestamp, has recognizable usage) for assistant lines of the
    /// last hour; feeds the format health check (D5).
    pub health: Vec<(i64, bool)>,
}

pub(crate) enum ClaudeIn {
    Fs(Vec<PathBuf>),
    /// Manual refresh: run the folder poll now instead of waiting for it.
    PollNow,
    Reset(u64),
    Shutdown,
}

pub(crate) fn fnv1a64(parts: &[&[u8]]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for b in *part {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

// ---------------------------------------------------------------- parsing

#[derive(Deserialize)]
struct RawLine<'a> {
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(borrow)]
    message: Option<&'a RawValue>,
    #[serde(rename = "requestId")]
    request_id: Option<String>,
    entrypoint: Option<String>,
    cwd: Option<String>,
    timestamp: Option<String>,
}

#[derive(Deserialize)]
struct Msg {
    model: Option<String>,
    id: Option<String>,
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Usage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
    cache_creation: Option<CacheCreation>,
    server_tool_use: Option<ServerToolUse>,
    speed: Option<String>,
    service_tier: Option<String>,
}

#[derive(Deserialize)]
struct CacheCreation {
    ephemeral_5m_input_tokens: Option<u64>,
    ephemeral_1h_input_tokens: Option<u64>,
}

#[derive(Deserialize)]
struct ServerToolUse {
    web_search_requests: Option<u64>,
}

#[derive(Debug, PartialEq)]
pub(crate) enum LineOutcome {
    NotAssistant,
    Record(ClaudeRecord),
    /// Healthy assistant line that carries no billable tokens.
    Synthetic(i64),
    /// Assistant line without a recognizable `usage` (format change?).
    NoUsage(i64),
    /// Not even the top level parsed. Only the error category is kept.
    ParseError(String),
}

pub(crate) fn parse_line(line: &[u8], now: i64) -> LineOutcome {
    if memchr::memmem::find(line, ASSISTANT_NEEDLE).is_none() {
        return LineOutcome::NotAssistant;
    }
    let raw: RawLine = match serde_json::from_slice(line) {
        Ok(r) => r,
        Err(e) => return LineOutcome::ParseError(error_category(&e)),
    };
    if raw.kind.as_deref() != Some("assistant") {
        return LineOutcome::NotAssistant;
    }
    let ts = raw
        .timestamp
        .as_deref()
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map(|d| d.timestamp())
        .unwrap_or(now);
    let Some(msg) = raw.message.and_then(|m| serde_json::from_str::<Msg>(m.get()).ok()) else {
        return LineOutcome::NoUsage(ts);
    };
    let Some(usage) = msg.usage else {
        return LineOutcome::NoUsage(ts);
    };
    let (Some(input), Some(output)) = (usage.input_tokens, usage.output_tokens) else {
        return LineOutcome::NoUsage(ts);
    };
    let model = msg.model.unwrap_or_else(|| "unknown".into());
    if model == "<synthetic>" {
        return LineOutcome::Synthetic(ts);
    }
    let cache_read = usage.cache_read_input_tokens.unwrap_or(0);
    let cache_total = usage.cache_creation_input_tokens.unwrap_or(0);
    let (mut cw5m, cw1h) = match &usage.cache_creation {
        Some(cc) => (
            cc.ephemeral_5m_input_tokens.unwrap_or(0),
            cc.ephemeral_1h_input_tokens.unwrap_or(0),
        ),
        None => (cache_total, 0),
    };
    // Split missing part of the total: bill it as the (cheaper) 5m write.
    cw5m += cache_total.saturating_sub(cw5m + cw1h);
    let key = match (&msg.id, &raw.request_id) {
        (Some(id), Some(req)) => fnv1a64(&[id.as_bytes(), b"\x1f", req.as_bytes()]),
        (Some(id), None) => fnv1a64(&[id.as_bytes()]),
        _ => fnv1a64(&[line]),
    };
    let input_side = input + cache_total.max(cw5m + cw1h) + cache_read;
    LineOutcome::Record(ClaudeRecord {
        ts,
        hour: ts.div_euclid(3600) * 3600,
        model,
        entrypoint: raw.entrypoint.filter(|e| !e.is_empty()).unwrap_or_else(|| "unknown".into()),
        project: project_name(raw.cwd.as_deref()),
        speed: usage.speed.filter(|s| !s.is_empty()).unwrap_or_else(|| "standard".into()),
        service_tier: usage
            .service_tier
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "standard".into()),
        band: context_band(input_side),
        tokens: [input, output, cache_read, cw5m, cw1h],
        web_search: usage
            .server_tool_use
            .and_then(|s| s.web_search_requests)
            .unwrap_or(0),
        key,
    })
}

/// Last path segment of `cwd` (Windows or POSIX separators).
pub(crate) fn project_name(cwd: Option<&str>) -> String {
    cwd.map(|c| c.trim_end_matches(['/', '\\']))
        .and_then(|c| c.rsplit(['/', '\\']).next())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "(bez projektu)".into())
}

// ---------------------------------------------------------------- scanning

pub(crate) struct Scanner {
    pub files: HashMap<String, FileState>,
    diag: Diag,
    clock: Clock,
}

fn millis(t: std::io::Result<std::time::SystemTime>) -> Option<i64> {
    t.ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as i64)
}

fn prefix_hash(path: &Path, len: u64) -> Option<u64> {
    let mut f = File::open(path).ok()?;
    let mut buf = vec![0u8; len as usize];
    f.read_exact(&mut buf).ok()?;
    Some(fnv1a64(&[&buf]))
}

impl Scanner {
    pub(crate) fn new(files: HashMap<String, FileState>, diag: Diag, clock: Clock) -> Self {
        Scanner { files, diag, clock }
    }

    /// Read whatever is new in `path` and hand it out in batches. The last
    /// batch always carries the final position, even without records.
    pub(crate) fn scan_file(
        &mut self,
        path: &Path,
        epoch: u64,
        stop: &AtomicBool,
        emit: &mut dyn FnMut(FileBatch) -> bool,
    ) {
        let key = path.to_string_lossy().to_string();
        // Gone or unreadable: keep state and aggregates untouched.
        let Ok(meta) = fs::metadata(path) else { return };
        let size = meta.len();
        let mtime = millis(meta.modified()).unwrap_or(0);
        let created = millis(meta.created());
        let now = (self.clock)();

        let mut start = 0;
        if let Some(prev) = self.files.get(&key) {
            if prev.size == size && prev.mtime == mtime && prev.offset <= size {
                return;
            }
            let same_creation = match (prev.created, created) {
                (Some(a), Some(b)) => a == b,
                _ => true,
            };
            let same_prefix = prev.prefix_len == 0
                || (size >= u64::from(prev.prefix_len)
                    && prefix_hash(path, u64::from(prev.prefix_len)) == Some(prev.prefix_hash));
            if same_creation && same_prefix && size >= prev.offset {
                start = prev.offset;
            } else {
                self.diag.log(
                    now,
                    "claude",
                    &format!(
                        "file replaced or truncated, rereading from 0: file={key} size={size} stored_offset={}",
                        prev.offset
                    ),
                );
            }
        }

        let Ok(mut file) = File::open(path) else { return };
        if file.seek(SeekFrom::Start(start)).is_err() {
            return;
        }
        let mut reader = BufReader::with_capacity(256 * 1024, file);
        let mut line = Vec::with_capacity(64 * 1024);
        let mut offset = start;
        let mut records = Vec::new();
        let mut health = Vec::new();
        let mut budget = DIAG_BUDGET;
        let mut lines_seen: u64 = 0;
        let health_from = now - 3600;

        loop {
            line.clear();
            let n = match reader.read_until(b'\n', &mut line) {
                Ok(n) => n,
                Err(e) => {
                    self.diag.log(now, "claude", &format!("read error: file={key} offset={offset} kind={:?}", e.kind()));
                    break;
                }
            };
            if n == 0 || line.last() != Some(&b'\n') {
                // EOF, or a line still being written: leave it for next time.
                break;
            }
            let line_offset = offset;
            offset += n as u64;
            lines_seen += 1;
            if lines_seen.is_multiple_of(4096) && stop.load(Ordering::Relaxed) {
                return;
            }
            match parse_line(&line, now) {
                LineOutcome::NotAssistant => {}
                LineOutcome::Record(rec) => {
                    if rec.ts >= health_from {
                        health.push((rec.ts, true));
                    }
                    records.push(rec);
                }
                LineOutcome::Synthetic(ts) => {
                    if ts >= health_from {
                        health.push((ts, true));
                    }
                }
                LineOutcome::NoUsage(ts) => {
                    if ts >= health_from {
                        health.push((ts, false));
                    }
                    if budget > 0 {
                        budget -= 1;
                        let keys = top_level_keys(&line).unwrap_or_default().join(",");
                        self.diag.log(
                            now,
                            "claude",
                            &format!(
                                "assistant line without usage: file={key} offset={line_offset} len={n} keys=[{keys}]"
                            ),
                        );
                    }
                }
                LineOutcome::ParseError(cat) => {
                    if budget > 0 {
                        budget -= 1;
                        self.diag.log(
                            now,
                            "claude",
                            &format!("unparsable line: file={key} offset={line_offset} len={n} error={cat}"),
                        );
                    }
                }
            }
            if records.len() >= BATCH {
                let state = self.state_for(path, created, size, mtime, offset);
                let batch = FileBatch {
                    epoch,
                    path: key.clone(),
                    state,
                    records: std::mem::take(&mut records),
                    health: std::mem::take(&mut health),
                };
                if !emit(batch) {
                    return;
                }
            }
        }

        let state = self.state_for(path, created, size, mtime, offset);
        self.files.insert(key.clone(), state.clone());
        emit(FileBatch {
            epoch,
            path: key,
            state,
            records,
            health,
        });
    }

    fn state_for(&self, path: &Path, created: Option<i64>, size: u64, mtime: i64, offset: u64) -> FileState {
        let prefix_len = offset.min(PREFIX_BYTES);
        FileState {
            created,
            prefix_len: prefix_len as u32,
            prefix_hash: if prefix_len == 0 {
                0
            } else {
                prefix_hash(path, prefix_len).unwrap_or(0)
            },
            size,
            mtime,
            offset,
        }
    }
}

pub(crate) fn list_jsonl(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = fs::read_dir(&d) else { continue };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            let p = entry.path();
            if ft.is_dir() {
                stack.push(p);
            } else if ft.is_file() && p.extension().is_some_and(|e| e == "jsonl") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

// ---------------------------------------------------------------- thread

pub(crate) struct ClaudeThread {
    pub dir: PathBuf,
    pub files: HashMap<String, FileState>,
    pub core: Sender<CoreMsg>,
    pub me: Sender<ClaudeIn>,
    pub rx: Receiver<ClaudeIn>,
    pub stop: Arc<AtomicBool>,
    pub clock: Clock,
    pub diag: Diag,
}

impl ClaudeThread {
    pub(crate) fn spawn(self) -> JoinHandle<()> {
        std::thread::Builder::new()
            .name("usage-claude".into())
            .spawn(move || self.run())
            .expect("spawn claude thread")
    }

    fn run(self) {
        let ClaudeThread {
            dir,
            files,
            core,
            me,
            rx,
            stop,
            clock,
            diag,
        } = self;
        let mut scanner = Scanner::new(files, diag.clone(), clock.clone());
        let mut epoch = 0u64;
        let send = |msg: Update| core.send(CoreMsg::Update(msg)).is_ok();

        'outer: loop {
            let dir_exists = dir.is_dir();
            // Watch before the backfill so changes made during it queue up.
            let _watcher = if dir_exists {
                match start_watcher(&dir, me.clone()) {
                    Ok(w) => Some(w),
                    Err(e) => {
                        diag.log(clock(), "claude", &format!("watcher failed, polling only: {e}"));
                        None
                    }
                }
            } else {
                None
            };

            // ---- backfill
            let list = if dir_exists { list_jsonl(&dir) } else { Vec::new() };
            let total = list.len() as u32;
            let progress = |done_files: u32, done: bool| Update::ClaudeProgress {
                epoch,
                files_done: done_files,
                files_total: total,
                done,
                dir_exists,
            };
            if !send(progress(0, false)) {
                return;
            }
            for (i, path) in list.iter().enumerate() {
                loop {
                    match rx.try_recv() {
                        Ok(ClaudeIn::Shutdown) | Err(TryRecvError::Disconnected) => return,
                        Ok(ClaudeIn::Reset(e)) => {
                            epoch = e;
                            scanner.files.clear();
                            continue 'outer;
                        }
                        // Picked up by the poll right after the backfill.
                        Ok(ClaudeIn::Fs(_) | ClaudeIn::PollNow) => {}
                        Err(TryRecvError::Empty) => break,
                    }
                }
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                let mut ok = true;
                scanner.scan_file(path, epoch, &stop, &mut |b| {
                    ok = send(Update::ClaudeBatch(b));
                    ok
                });
                if !ok || !send(progress(i as u32 + 1, false)) {
                    return;
                }
            }
            if !send(progress(total, true)) {
                return;
            }

            // ---- live
            let mut dirty: HashMap<PathBuf, Instant> = HashMap::new();
            let mut last_poll: Option<Instant> = None;
            loop {
                match rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(ClaudeIn::Fs(paths)) => {
                        let t = Instant::now();
                        for p in paths {
                            dirty.insert(p, t);
                        }
                    }
                    Ok(ClaudeIn::PollNow) => last_poll = None,
                    Ok(ClaudeIn::Reset(e)) => {
                        epoch = e;
                        scanner.files.clear();
                        continue 'outer;
                    }
                    Ok(ClaudeIn::Shutdown) | Err(RecvTimeoutError::Disconnected) => return,
                    Err(RecvTimeoutError::Timeout) => {}
                }
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                let t = Instant::now();
                let ready: Vec<PathBuf> = dirty
                    .iter()
                    .filter(|(_, seen)| t.duration_since(**seen) >= QUIET)
                    .map(|(p, _)| p.clone())
                    .collect();
                for p in ready {
                    dirty.remove(&p);
                    let mut ok = true;
                    scanner.scan_file(&p, epoch, &stop, &mut |b| {
                        ok = send(Update::ClaudeBatch(b));
                        ok
                    });
                    if !ok {
                        return;
                    }
                }
                if last_poll.is_none_or(|lp| t.duration_since(lp) >= POLL) {
                    last_poll = Some(t);
                    if !dir_exists {
                        if dir.is_dir() {
                            continue 'outer;
                        }
                        continue;
                    }
                    for p in list_jsonl(&dir) {
                        let mut ok = true;
                        scanner.scan_file(&p, epoch, &stop, &mut |b| {
                            ok = send(Update::ClaudeBatch(b));
                            ok
                        });
                        if !ok {
                            return;
                        }
                    }
                }
            }
        }
    }
}

fn start_watcher(dir: &Path, me: Sender<ClaudeIn>) -> notify::Result<notify::RecommendedWatcher> {
    use notify::{EventKind, RecursiveMode, Watcher};
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let Ok(event) = event else { return };
        if matches!(event.kind, EventKind::Access(_)) {
            return;
        }
        let paths: Vec<PathBuf> = event
            .paths
            .into_iter()
            .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
            .collect();
        if !paths.is_empty() {
            let _ = me.send(ClaudeIn::Fs(paths));
        }
    })?;
    watcher.watch(dir, RecursiveMode::Recursive)?;
    Ok(watcher)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::Arc;

    fn fixtures() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/claude")
    }

    fn line(id: &str, req: &str, ts: &str, output: u64) -> String {
        format!(
            r#"{{"type":"assistant","timestamp":"{ts}","requestId":"{req}","entrypoint":"cli","cwd":"C:\\dev\\alpha","message":{{"model":"claude-opus-5-5","id":"{id}","content":[{{"type":"text","text":"hello"}}],"usage":{{"input_tokens":10,"output_tokens":{output},"cache_creation_input_tokens":0,"cache_read_input_tokens":100,"speed":"standard","service_tier":"standard"}}}}}}"#
        )
    }

    fn scanner(dir: &Path) -> Scanner {
        Scanner::new(HashMap::new(), Diag::new(dir), Arc::new(|| 1_793_190_000))
    }

    fn scan(s: &mut Scanner, p: &Path) -> Vec<FileBatch> {
        let mut out = Vec::new();
        s.scan_file(p, 0, &AtomicBool::new(false), &mut |b| {
            out.push(b);
            true
        });
        out
    }

    #[test]
    fn parses_fixture_line_fields() {
        let text = fs::read_to_string(fixtures().join("parse/lines.jsonl")).unwrap();
        let outcomes: Vec<LineOutcome> = text.lines().map(|l| parse_line(l.as_bytes(), 0)).collect();
        let LineOutcome::Record(r) = &outcomes[1] else { panic!("{:?}", outcomes[1]) };
        assert_eq!(r.model, "claude-sonnet-4-5-20250929");
        assert_eq!(r.entrypoint, "cli");
        assert_eq!(r.project, "alpha");
        assert_eq!(r.tokens, [1000, 2000, 10000, 1000, 2000]);
        assert_eq!(r.web_search, 2);
        assert_eq!(r.band, 0);
        assert_eq!(r.ts, 1_792_490_400); // 2026-10-20T10:00:00Z
        // user line, nested assistant inside a progress line, synthetic, no usage, broken
        assert_eq!(outcomes[0], LineOutcome::NotAssistant);
        assert_eq!(outcomes[2], LineOutcome::NotAssistant);
        assert!(matches!(outcomes[3], LineOutcome::Synthetic(_)));
        assert!(matches!(outcomes[4], LineOutcome::NoUsage(_)));
        assert!(matches!(outcomes[5], LineOutcome::ParseError(_)));
        // Long context: 150k + 60k + 0 input-side tokens, missing cache split.
        let LineOutcome::Record(lc) = &outcomes[6] else { panic!() };
        assert_eq!(lc.band, 200);
        assert_eq!(lc.tokens[3], 60_000);
        assert_eq!(lc.speed, "fast");
        assert_eq!(lc.project, "beta");
    }

    #[test]
    fn partial_last_line_waits_for_newline() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.jsonl");
        let full = line("m1", "r1", "2026-10-28T10:00:00Z", 5);
        let second = line("m2", "r2", "2026-10-28T10:01:00Z", 7);
        fs::write(&p, format!("{full}\n{}", &second[..40])).unwrap();
        let mut s = scanner(dir.path());
        let b = scan(&mut s, &p);
        let last = b.last().unwrap();
        assert_eq!(b.iter().map(|x| x.records.len()).sum::<usize>(), 1);
        assert_eq!(last.state.offset, full.len() as u64 + 1, "offset must stop before the partial line");

        // The writer finishes the line.
        let mut f = fs::OpenOptions::new().append(true).open(&p).unwrap();
        writeln!(f, "{}", &second[40..]).unwrap();
        drop(f);
        let b = scan(&mut s, &p);
        let recs: Vec<_> = b.iter().flat_map(|x| x.records.iter()).collect();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].tokens[1], 7);
        assert_eq!(b.last().unwrap().state.offset, fs::metadata(&p).unwrap().len());
    }

    #[test]
    fn truncated_or_replaced_file_rereads_from_zero() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.jsonl");
        let a = line("m1", "r1", "2026-10-28T10:00:00Z", 5);
        let b = line("m2", "r2", "2026-10-28T10:01:00Z", 7);
        fs::write(&p, format!("{a}\n{b}\n")).unwrap();
        let mut s = scanner(dir.path());
        assert_eq!(scan(&mut s, &p).iter().map(|x| x.records.len()).sum::<usize>(), 2);

        // Truncated to a shorter, different file.
        let c = line("m3", "r3", "2026-10-28T10:02:00Z", 9);
        fs::write(&p, format!("{c}\n")).unwrap();
        let batches = scan(&mut s, &p);
        let recs: Vec<_> = batches.iter().flat_map(|x| x.records.iter()).collect();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].tokens[1], 9);
        assert_eq!(batches.last().unwrap().state.offset, c.len() as u64 + 1);

        // Replaced by a longer file with a different beginning: also from 0;
        // m1 comes back and the store's dedupe key makes it count once.
        fs::write(&p, format!("{b}\n{a}\n{c}\n{c}\n")).unwrap();
        let recs: Vec<ClaudeRecord> = scan(&mut s, &p).into_iter().flat_map(|x| x.records).collect();
        assert_eq!(recs.len(), 4);
        let mut store = crate::store::Store::open_in_memory();
        let st = s.files.values().next().unwrap().clone();
        let changed = store.apply_claude_batch("s", &st, &recs).unwrap();
        assert_eq!(changed, 3, "duplicate m3 must not count twice");
    }

    #[test]
    fn health_entries_and_diag_without_content() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("h.jsonl");
        let mut text = String::new();
        for i in 0..19 {
            text.push_str(&line(&format!("m{i}"), &format!("r{i}"), "2026-10-28T11:30:00Z", 5));
            text.push('\n');
        }
        for i in 0..6 {
            text.push_str(&format!(
                r#"{{"type":"assistant","timestamp":"2026-10-28T11:31:00Z","requestId":"x{i}","message":{{"id":"y{i}","content":"TOP-SECRET-PROMPT","tokens_v2":{{"in":1}}}}}}"#
            ));
            text.push('\n');
        }
        text.push_str(r#"{"type":"assistant","message":{"content":"TOP-SECRET-BROKEN"#);
        text.push('\n');
        fs::write(&p, text).unwrap();
        let mut s = scanner(dir.path());
        let batches = scan(&mut s, &p);
        let health: Vec<(i64, bool)> = batches.into_iter().flat_map(|b| b.health).collect();
        assert_eq!(health.len(), 25);
        assert_eq!(health.iter().filter(|(_, ok)| !ok).count(), 6);

        let log = fs::read_to_string(dir.path().join("diag.log")).unwrap();
        assert!(log.contains("assistant line without usage"));
        assert!(log.contains("keys=["));
        assert!(log.contains("unparsable line"));
        assert!(log.contains("h.jsonl"));
        assert!(!log.contains("TOP-SECRET"), "diag leaked content:\n{log}");
        assert!(!log.contains("hello"));
    }

    #[test]
    fn unchanged_file_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.jsonl");
        fs::write(&p, format!("{}\n", line("m1", "r1", "2026-10-28T10:00:00Z", 5))).unwrap();
        let mut s = scanner(dir.path());
        assert_eq!(scan(&mut s, &p).len(), 1);
        assert!(scan(&mut s, &p).is_empty());
    }

    #[test]
    fn deleted_file_keeps_state() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.jsonl");
        fs::write(&p, format!("{}\n", line("m1", "r1", "2026-10-28T10:00:00Z", 5))).unwrap();
        let mut s = scanner(dir.path());
        scan(&mut s, &p);
        fs::remove_file(&p).unwrap();
        assert!(scan(&mut s, &p).is_empty());
        assert_eq!(s.files.len(), 1);
    }

    #[test]
    fn project_names() {
        assert_eq!(project_name(Some(r"C:\Users\dev\projects\e-shop-api")), "e-shop-api");
        assert_eq!(project_name(Some("/home/u/proj/")), "proj");
        assert_eq!(project_name(None), "(bez projektu)");
    }
}
