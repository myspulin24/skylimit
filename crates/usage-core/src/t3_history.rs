//! Claude history that only T3 Code still remembers. Claude Code deletes its
//! transcripts after 30 days (`cleanupPeriodDays`), so whatever it deleted
//! before Skylimit read it is gone from `claude.rs`. T3 Code's usage view
//! keeps the numbers of every message it ever scanned in
//! `~/.t3/userdata/usage-scan-cache*.json`; this module takes the messages of
//! transcripts that no longer exist from there.
//!
//! ```text
//!  cache files, newest first ─▶ tuples of {p:"claude"} files
//!       ─▶ layout check against messages Skylimit counted itself
//!          (same key: message.id + requestId; ≥ 20 of them, ≥ 95 % agree)
//!       ─▶ messages of deleted transcripts not counted yet ─▶ ClaudeRecord
//!       ─▶ store (dedupe by key, so reruns never double count)
//! ```
//!
//! The cache is T3's internal format, so nothing in it is trusted blindly: a
//! layout that does not match Skylimit's own numbers is skipped whole.
//! Transcripts that still exist are read by `claude.rs`, never from here.
//! Numbers only: the cache holds no conversation text, and only counts reach
//! the diagnostic log. Tuple layout (versions 3 to 9):
//! `[time ms, model index, session index, input, cache read, cache write,
//! output, _, "message.id:requestId", …]`. The cache does not split cache
//! writes into 5 min and 1 h, so they are billed as the cheaper 5 min write,
//! like a transcript without the split; it has no entrypoint either, so
//! the source is `t3-cache`, and the project comes from the folder name.
//!
//! Output is a lower bound: Claude Code writes one message on several lines
//! and the first often carries a partial `output_tokens`. Skylimit keeps the
//! largest (`store.rs`), T3 the first, so its output runs about a quarter
//! lower for SDK sessions. "Agree" therefore means equal input and cache
//! counts and T3's output not above Skylimit's; restored messages keep T3's
//! output as it is, never scaled by a guess.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde_json::Value;

use crate::claude::{fnv1a64, ClaudeRecord};
use crate::pricing::context_band;

const CACHE_PREFIX: &str = "usage-scan-cache";
/// Messages counted by both before the cache's numbers are trusted.
const MIN_OVERLAP: usize = 20;
/// Share of those messages that must agree (see the module note on output).
const MIN_AGREE: f64 = 0.95;
/// Entrypoint of restored messages: the cache does not say where they ran,
/// so the source says where they came back from.
pub(crate) const ENTRYPOINT: &str = "t3-cache";

/// One message of the cache.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Message {
    /// The transcript it came from.
    pub path: String,
    pub ts: i64,
    pub model: String,
    /// input, cache read, cache write, output
    pub tokens: [u64; 4],
    pub key: u64,
}

/// T3's cache files, newest name first (`-v9` before `-v7` before none), with
/// their modification time for change detection.
pub(crate) fn cache_files(dir: &Path) -> Vec<(PathBuf, i64)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<(PathBuf, i64, u32)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let rest = name.strip_prefix(CACHE_PREFIX)?.strip_suffix(".json")?;
            let version = match rest.strip_prefix("-v") {
                Some(v) => v.parse().ok()?,
                None if rest.is_empty() => 0,
                None => return None,
            };
            let mtime = e
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            Some((e.path(), mtime, version))
        })
        .collect();
    out.sort_by(|a, b| b.2.cmp(&a.2));
    out.into_iter().map(|(p, m, _)| (p, m)).collect()
}

fn count(v: &Value) -> Option<u64> {
    let n = v.as_f64()?;
    (n.is_finite() && n >= 0.0).then_some(n as u64)
}

/// Claude messages of one cache file; `None` when it is not the known shape.
pub(crate) fn parse(text: &str) -> Option<Vec<Message>> {
    let doc: Value = serde_json::from_str(text).ok()?;
    let models: Vec<&str> = doc.get("models")?.as_array()?.iter().map(|m| m.as_str().unwrap_or("")).collect();
    let mut out = Vec::new();
    for (path, file) in doc.get("files")?.as_object()? {
        if file.get("p").and_then(Value::as_str) != Some("claude") {
            continue;
        }
        let Some(tuples) = file.get("r").and_then(Value::as_array) else { continue };
        for t in tuples {
            let Some(t) = t.as_array().filter(|t| t.len() >= 9) else { continue };
            let Some(ms) = t[0].as_f64().filter(|ms| *ms > 1e12) else { continue };
            // `<synthetic>`: Claude Code's own placeholder replies, no tokens.
            let Some(model) = t[1]
                .as_u64()
                .and_then(|i| models.get(i as usize))
                .filter(|m| !m.is_empty() && **m != "<synthetic>")
            else {
                continue;
            };
            let (Some(input), Some(read), Some(write), Some(output)) = (count(&t[3]), count(&t[4]), count(&t[5]), count(&t[6]))
            else {
                continue;
            };
            let Some((id, req)) = t[8].as_str().and_then(|k| k.split_once(':')) else { continue };
            if id.is_empty() || req.is_empty() {
                continue;
            }
            out.push(Message {
                path: path.clone(),
                ts: (ms / 1000.0) as i64,
                model: (*model).to_string(),
                tokens: [input, read, write, output],
                key: fnv1a64(&[id.as_bytes(), b"\x1f", req.as_bytes()]),
            });
        }
    }
    Some(out)
}

#[derive(Debug, PartialEq)]
pub(crate) struct Plan {
    pub records: Vec<ClaudeRecord>,
    pub overlap: usize,
    pub agree: usize,
    /// Deleted transcripts the records come from.
    pub files: usize,
}

/// What to add from one cache file. `known` gives Skylimit's counted tokens
/// for a key, `exists` says whether a transcript is still on disk.
pub(crate) fn plan(
    messages: &[Message],
    known: impl Fn(u64) -> Option<[u64; 4]>,
    exists: impl Fn(&str) -> bool,
    project: impl Fn(&str) -> String,
) -> Result<Plan, String> {
    let mut overlap = 0;
    let mut agree = 0;
    let mut missing = Vec::new();
    for m in messages {
        match known(m.key) {
            Some(t) => {
                overlap += 1;
                if t[..3] == m.tokens[..3] && m.tokens[3] <= t[3] {
                    agree += 1;
                }
            }
            None => missing.push(m),
        }
    }
    if overlap < MIN_OVERLAP || (agree as f64) < MIN_AGREE * overlap as f64 {
        return Err(format!("layout not confirmed: {agree} of {overlap} known messages agree"));
    }
    let mut gone: HashMap<&str, bool> = HashMap::new();
    let mut projects: HashMap<&str, String> = HashMap::new();
    let mut seen = HashSet::new();
    let mut files = HashSet::new();
    let mut records = Vec::new();
    for m in missing {
        let deleted = *gone.entry(m.path.as_str()).or_insert_with(|| !exists(&m.path));
        if !deleted || !seen.insert(m.key) {
            continue;
        }
        files.insert(m.path.as_str());
        let folder = Path::new(&m.path)
            .parent()
            .and_then(Path::file_name)
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();
        let project = projects.entry(m.path.as_str()).or_insert_with(|| project(&folder)).clone();
        let [input, read, write, output] = m.tokens;
        records.push(ClaudeRecord {
            ts: m.ts,
            hour: m.ts.div_euclid(3600) * 3600,
            model: m.model.clone(),
            entrypoint: ENTRYPOINT.into(),
            project,
            speed: "standard".into(),
            service_tier: "standard".into(),
            band: context_band(input + write + read),
            tokens: [input, output, read, write, 0],
            web_search: 0,
            key: m.key,
        });
    }
    Ok(Plan {
        records,
        overlap,
        agree,
        files: files.len(),
    })
}

/// Claude Code's folder name for a cwd: every character other than an ASCII
/// letter or digit becomes `-`.
pub(crate) fn encode(path: &str) -> String {
    path.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

/// The project (last cwd segment) behind a transcript folder name. The
/// encoding is lossy (`dev-ai-usage` could be `dev\ai-usage` or
/// `dev\ai\usage`), so the cwd is found again on disk: from the root, the
/// longest child whose encoded name continues the folder name wins. Once no
/// child matches (the folder was deleted), the rest of the name is the answer.
pub(crate) fn project_from_folder(folder: &str) -> String {
    let fallback = || {
        folder
            .rsplit('-')
            .find(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| "(bez projektu)".into())
    };
    let bytes = folder.as_bytes();
    let (mut dir, mut rest) = if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && &folder[1..3] == "--" {
        (PathBuf::from(format!("{}:\\", &folder[..1])), &folder[3..])
    } else if let Some(rest) = folder.strip_prefix('-') {
        (PathBuf::from("/"), rest)
    } else {
        return fallback();
    };
    let mut last: Option<String> = None;
    while !rest.is_empty() {
        // A root that is not there (a cache from another system) says nothing.
        let Ok(entries) = fs::read_dir(&dir) else {
            return if last.is_some() { rest.to_string() } else { fallback() };
        };
        let best = entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let enc = encode(&name);
                let head = rest.get(..enc.len())?;
                let next = rest.as_bytes().get(enc.len());
                (!enc.is_empty() && head.eq_ignore_ascii_case(&enc) && matches!(next, None | Some(b'-')))
                    .then_some((enc.len(), name))
            })
            .max_by_key(|(len, _)| *len);
        let Some((len, name)) = best else { break };
        dir.push(&name);
        rest = rest.get(len + 1..).unwrap_or("");
        last = Some(name);
    }
    if rest.is_empty() {
        last.unwrap_or_else(fallback)
    } else {
        rest.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tuple(ms: u64, model: u32, tokens: [u64; 4], key: &str) -> Value {
        serde_json::json!([ms, model, 0, tokens[0], tokens[1], tokens[2], tokens[3], 0, key, null, 0, null])
    }

    fn cache(files: Value) -> String {
        serde_json::json!({
            "version": 9,
            "models": ["claude-opus-5", "claude-sonnet-5", "<synthetic>"],
            "sessions": ["s1"],
            "files": files,
        })
        .to_string()
    }

    fn key(id: &str, req: &str) -> u64 {
        fnv1a64(&[id.as_bytes(), b"\x1f", req.as_bytes()])
    }

    #[test]
    fn parses_claude_tuples_and_skips_the_rest() {
        let text = cache(serde_json::json!({
            "/p/a/x.jsonl": { "p": "claude", "r": [
                tuple(1_789_768_234_939, 1, [2, 32_450, 21_830, 511], "msg_1:req_1"),
                [1, 2, 3],
                tuple(1_789_768_234_939, 7, [1, 1, 1, 1], "msg_2:req_2"),
                tuple(1_789_768_234_939, 0, [1, 1, 1, 1], "no-request-id"),
                tuple(1_789_768_234_939, 2, [0, 0, 0, 0], "msg_3:req_3"),
            ]},
            "/p/codex.jsonl": { "p": "codex", "r": [tuple(1_789_768_234_939, 0, [1, 1, 1, 1], "a:b")] },
        }));
        let m = parse(&text).unwrap();
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].model, "claude-sonnet-5");
        assert_eq!(m[0].tokens, [2, 32_450, 21_830, 511]);
        assert_eq!(m[0].ts, 1_789_768_234);
        assert_eq!(m[0].key, key("msg_1", "req_1"));
        assert!(parse("[]").is_none());
        assert!(parse("{\"models\": []}").is_none());
    }

    fn messages(n: usize, path: &str, offset: usize) -> Vec<Message> {
        (0..n)
            .map(|i| Message {
                path: path.into(),
                ts: 1_789_768_234 + i as i64,
                model: "claude-opus-5".into(),
                tokens: [1, 1000 + i as u64, 10, 5],
                key: (offset + i) as u64,
            })
            .collect()
    }

    #[test]
    fn imports_only_deleted_transcripts_after_the_layout_check() {
        let mut all = messages(30, "/p/C--dev-alpha/live.jsonl", 0);
        all.extend(messages(5, "/p/C--dev-alpha/live.jsonl", 100));
        all.extend(messages(4, "/p/C--dev-beta/gone.jsonl", 200));
        all.push(all.last().unwrap().clone());
        // T3 keeps the first, partial output of a message: lower agrees.
        let known = |k: u64| (k < 30).then(|| [1, 1000 + k, 10, if k.is_multiple_of(2) { 5 } else { 40 }]);
        let exists = |p: &str| p.contains("live");
        let plan = plan(&all, known, exists, |f| f.rsplit('-').next().unwrap().to_string()).unwrap();
        assert_eq!((plan.overlap, plan.agree, plan.files), (30, 30, 1));
        assert_eq!(plan.records.len(), 4, "live transcripts are read by claude.rs; duplicates count once");
        let r = &plan.records[0];
        assert_eq!(r.project, "beta");
        assert_eq!(r.entrypoint, ENTRYPOINT);
        assert_eq!(r.tokens, [1, 5, 1000, 10, 0]);
        assert_eq!(r.hour % 3600, 0);
    }

    #[test]
    fn a_layout_that_disagrees_or_cannot_be_checked_is_skipped() {
        let all = messages(30, "/p/C--dev-beta/gone.jsonl", 0);
        let wrong = |k: u64| (k < 30).then(|| [1, 10, 1000 + k, 5]);
        assert!(plan(&all, wrong, |_| false, |_| String::new()).is_err());
        let few = |k: u64| (k < 5).then(|| [1, 1000 + k, 10, 5]);
        assert!(plan(&all, few, |_| false, |_| String::new()).is_err());
        let more_output = |k: u64| (k < 30).then(|| [1, 1000 + k, 10, 4]);
        assert!(plan(&all, more_output, |_| false, |_| String::new()).is_err(), "T3 never counts more output");
    }

    #[test]
    fn finds_the_project_folder_on_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let root_s = root.to_string_lossy().trim_start_matches(r"\\?\").to_string();
        fs::create_dir_all(root.join("dev").join("ai-usage")).unwrap();
        fs::create_dir_all(root.join("dev").join("ai")).unwrap();
        fs::create_dir_all(root.join("dev").join(".claude").join("worktrees")).unwrap();
        let folder = |rel: &str| encode(&format!("{root_s}{}{rel}", std::path::MAIN_SEPARATOR));
        assert_eq!(project_from_folder(&folder("dev")), "dev");
        assert_eq!(project_from_folder(&folder("dev/ai-usage")), "ai-usage", "longest match wins");
        assert_eq!(project_from_folder(&folder("dev/.claude/worktrees")), "worktrees");
        assert_eq!(project_from_folder(&folder("dev/gone-project")), "gone-project");
        assert_eq!(project_from_folder("plain-name"), "name");
    }

    #[test]
    fn cache_files_newest_version_first() {
        let tmp = tempfile::tempdir().unwrap();
        for name in ["usage-scan-cache.json", "usage-scan-cache-v9.json", "usage-scan-cache-v10.json", "usage-model-rates.json"] {
            fs::write(tmp.path().join(name), "{}").unwrap();
        }
        let names: Vec<String> = cache_files(tmp.path())
            .into_iter()
            .map(|(p, _)| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["usage-scan-cache-v10.json", "usage-scan-cache-v9.json", "usage-scan-cache.json"]);
    }
}
