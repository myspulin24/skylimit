//! SQLite history (decision D4: `%LOCALAPPDATA%\skylimit-data\history.sqlite`,
//! the path comes from `Config::db_path`).
//!
//! Only the core thread owns this connection (WAL). Aggregates hold TOKENS,
//! never money: prices are applied at query time, so a new price list reprices
//! history without rereading the 1.6 GB of logs.

use std::collections::HashMap;
use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

use crate::claude::{ClaudeRecord, FileState};
use crate::fx::FxDay;
use crate::pricing::TokenKind;

pub(crate) struct Store {
    conn: Connection,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct UsageRow {
    pub hour: i64,
    pub provider: String,
    pub model: String,
    pub entrypoint: String,
    pub project: String,
    pub speed: String,
    pub service_tier: String,
    pub band: u32,
    pub kind: TokenKind,
    pub tokens: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WebRow {
    pub hour: i64,
    pub model: String,
    pub entrypoint: String,
    pub project: String,
    pub requests: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LimitRow {
    pub win: String,
    pub percent: Option<f64>,
    pub resets_at: Option<i64>,
    pub ts: i64,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta(
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
-- Read position per JSONL file plus what identifies the file.
CREATE TABLE IF NOT EXISTS files(
    path TEXT PRIMARY KEY,
    created INTEGER,
    prefix_len INTEGER NOT NULL,
    prefix_hash INTEGER NOT NULL,
    size INTEGER NOT NULL,
    mtime INTEGER NOT NULL,
    offset INTEGER NOT NULL
);
-- Dedupe: 64-bit hash of message.id + requestId. Claude Code repeats a
-- message on several lines and the first one often carries a partial
-- output_tokens, so the counted values are kept and a later, larger value
-- adds only the difference (max per kind).
CREATE TABLE IF NOT EXISTS seen_messages(
    hash INTEGER PRIMARY KEY,
    hour INTEGER NOT NULL,
    model TEXT NOT NULL,
    entrypoint TEXT NOT NULL,
    project TEXT NOT NULL,
    speed TEXT NOT NULL,
    service_tier TEXT NOT NULL,
    long_context INTEGER NOT NULL,
    input INTEGER NOT NULL,
    output INTEGER NOT NULL,
    cache_read INTEGER NOT NULL,
    cache_write_5m INTEGER NOT NULL,
    cache_write_1h INTEGER NOT NULL,
    web_search INTEGER NOT NULL
);
-- Hourly token aggregates keyed by every price dimension. long_context holds
-- the input-side band in thousands (0, 100, 200, 272). The primary key's
-- leading columns (hour, provider) are the (hour, provider) index.
CREATE TABLE IF NOT EXISTS usage_hourly(
    hour INTEGER NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    entrypoint TEXT NOT NULL,
    project TEXT NOT NULL,
    speed TEXT NOT NULL,
    service_tier TEXT NOT NULL,
    long_context INTEGER NOT NULL,
    token_kind TEXT NOT NULL,
    tokens INTEGER NOT NULL,
    PRIMARY KEY(hour, provider, model, entrypoint, project, speed, service_tier, long_context, token_kind)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS web_search_hourly(
    hour INTEGER NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    entrypoint TEXT NOT NULL,
    project TEXT NOT NULL,
    requests INTEGER NOT NULL,
    PRIMARY KEY(hour, provider, model, entrypoint, project)
) WITHOUT ROWID;
-- Authoritative daily totals from account/usage/read (upsert by date).
CREATE TABLE IF NOT EXISTS codex_daily(
    date TEXT PRIMARY KEY,
    tokens INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS fx_rates(
    date TEXT PRIMARY KEY,
    czk_per_usd REAL NOT NULL,
    czk_per_eur REAL NOT NULL,
    fetched_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS milestones(
    id TEXT PRIMARY KEY,
    at INTEGER NOT NULL,
    shown INTEGER NOT NULL
);
-- Limit history for the full app chart, pruned after 90 days.
CREATE TABLE IF NOT EXISTS limit_snapshots(
    provider TEXT NOT NULL,
    win TEXT NOT NULL,
    ts INTEGER NOT NULL,
    percent REAL,
    resets_at INTEGER,
    PRIMARY KEY(provider, win, ts)
) WITHOUT ROWID;
"#;

const ALL_TABLES: [&str; 9] = [
    "meta",
    "files",
    "seen_messages",
    "usage_hourly",
    "web_search_hourly",
    "codex_daily",
    "fx_rates",
    "milestones",
    "limit_snapshots",
];

impl Store {
    pub(crate) fn open(path: &Path) -> rusqlite::Result<Store> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let _mode: String = conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))?;
        conn.execute_batch("PRAGMA synchronous=NORMAL;")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Store { conn })
    }

    pub(crate) fn open_in_memory() -> Store {
        let conn = Connection::open_in_memory().expect("in-memory sqlite");
        conn.execute_batch(SCHEMA).expect("schema");
        Store { conn }
    }

    #[cfg(test)]
    pub(crate) fn journal_mode(&self) -> String {
        self.conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap_or_default()
    }

    pub(crate) fn load_files(&self) -> HashMap<String, FileState> {
        let mut out = HashMap::new();
        let Ok(mut stmt) = self
            .conn
            .prepare("SELECT path, created, prefix_len, prefix_hash, size, mtime, offset FROM files")
        else {
            return out;
        };
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                FileState {
                    created: r.get(1)?,
                    prefix_len: r.get::<_, i64>(2)? as u32,
                    prefix_hash: r.get::<_, i64>(3)? as u64,
                    size: r.get::<_, i64>(4)? as u64,
                    mtime: r.get(5)?,
                    offset: r.get::<_, i64>(6)? as u64,
                },
            ))
        });
        if let Ok(rows) = rows {
            for (path, state) in rows.flatten() {
                out.insert(path, state);
            }
        }
        out
    }

    /// Persist one file's new position and its records in one transaction.
    /// Returns how many records changed the aggregates.
    pub(crate) fn apply_claude_batch(
        &mut self,
        path: &str,
        state: &FileState,
        records: &[ClaudeRecord],
    ) -> rusqlite::Result<usize> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO files(path, created, prefix_len, prefix_hash, size, mtime, offset)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(path) DO UPDATE SET created=excluded.created, prefix_len=excluded.prefix_len,
               prefix_hash=excluded.prefix_hash, size=excluded.size, mtime=excluded.mtime, offset=excluded.offset",
            params![
                path,
                state.created,
                state.prefix_len as i64,
                state.prefix_hash as i64,
                state.size as i64,
                state.mtime,
                state.offset as i64
            ],
        )?;
        let mut changed = 0;
        for rec in records {
            if apply_record(&tx, rec)? {
                changed += 1;
            }
        }
        tx.commit()?;
        Ok(changed)
    }

    /// Records that belong to no transcript file (history restored from T3
    /// Code's cache), with the same dedupe as a file batch.
    pub(crate) fn apply_records(&mut self, records: &[ClaudeRecord]) -> rusqlite::Result<usize> {
        let tx = self.conn.transaction()?;
        let mut changed = 0;
        for rec in records {
            if apply_record(&tx, rec)? {
                changed += 1;
            }
        }
        tx.commit()?;
        Ok(changed)
    }

    /// Counted tokens of one message by its dedupe key: input, cache read,
    /// cache write (5 min + 1 h), output.
    pub(crate) fn seen_tokens(&self, key: u64) -> Option<[u64; 4]> {
        self.conn
            .prepare_cached(
                "SELECT input, cache_read, cache_write_5m + cache_write_1h, output FROM seen_messages WHERE hash = ?1",
            )
            .ok()?
            .query_row([key as i64], |r| {
                Ok([
                    r.get::<_, i64>(0)?.max(0) as u64,
                    r.get::<_, i64>(1)?.max(0) as u64,
                    r.get::<_, i64>(2)?.max(0) as u64,
                    r.get::<_, i64>(3)?.max(0) as u64,
                ])
            })
            .optional()
            .ok()
            .flatten()
    }

    /// The oldest hour with a provider's tokens.
    pub(crate) fn first_usage_hour(&self, provider: &str) -> Option<i64> {
        self.conn
            .query_row("SELECT MIN(hour) FROM usage_hourly WHERE provider = ?1", [provider], |r| {
                r.get::<_, Option<i64>>(0)
            })
            .ok()
            .flatten()
    }

    pub(crate) fn usage_rows(&self, start_hour: i64, end_hour: i64) -> rusqlite::Result<Vec<UsageRow>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT hour, provider, model, entrypoint, project, speed, service_tier, long_context, token_kind, tokens
             FROM usage_hourly WHERE hour >= ?1 AND hour < ?2",
        )?;
        let rows = stmt.query_map(params![start_hour, end_hour], |r| {
            Ok((
                UsageRow {
                    hour: r.get(0)?,
                    provider: r.get(1)?,
                    model: r.get(2)?,
                    entrypoint: r.get(3)?,
                    project: r.get(4)?,
                    speed: r.get(5)?,
                    service_tier: r.get(6)?,
                    band: r.get::<_, i64>(7)? as u32,
                    kind: TokenKind::Input,
                    tokens: r.get::<_, i64>(9)?.max(0) as u64,
                },
                r.get::<_, String>(8)?,
            ))
        })?;
        Ok(rows
            .flatten()
            .filter_map(|(mut row, kind)| {
                row.kind = TokenKind::parse(&kind)?;
                Some(row)
            })
            .collect())
    }

    pub(crate) fn web_rows(&self, start_hour: i64, end_hour: i64) -> rusqlite::Result<Vec<WebRow>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT hour, model, entrypoint, project, requests FROM web_search_hourly
             WHERE hour >= ?1 AND hour < ?2",
        )?;
        let rows = stmt.query_map(params![start_hour, end_hour], |r| {
            Ok(WebRow {
                hour: r.get(0)?,
                model: r.get(1)?,
                entrypoint: r.get(2)?,
                project: r.get(3)?,
                requests: r.get::<_, i64>(4)?.max(0) as u64,
            })
        })?;
        Ok(rows.flatten().collect())
    }

    /// Total tokens per hour for a provider over the whole history.
    pub(crate) fn hourly_totals(&self, provider: &str) -> rusqlite::Result<Vec<(i64, u64)>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT hour, SUM(tokens) FROM usage_hourly WHERE provider = ?1 GROUP BY hour ORDER BY hour",
        )?;
        let rows = stmt.query_map([provider], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?.max(0) as u64))
        })?;
        Ok(rows.flatten().collect())
    }

    pub(crate) fn upsert_codex_buckets(&mut self, buckets: &[(String, u64)], now: i64) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO codex_daily(date, tokens, updated_at) VALUES(?1, ?2, ?3)
                 ON CONFLICT(date) DO UPDATE SET tokens=excluded.tokens, updated_at=excluded.updated_at",
            )?;
            for (date, tokens) in buckets {
                stmt.execute(params![date, *tokens as i64, now])?;
            }
        }
        tx.commit()
    }

    pub(crate) fn codex_buckets(&self) -> rusqlite::Result<Vec<(String, u64)>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT date, tokens FROM codex_daily ORDER BY date")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?.max(0) as u64))
        })?;
        Ok(rows.flatten().collect())
    }

    pub(crate) fn put_fx(&self, day: &FxDay, now: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO fx_rates(date, czk_per_usd, czk_per_eur, fetched_at) VALUES(?1, ?2, ?3, ?4)
             ON CONFLICT(date) DO UPDATE SET czk_per_usd=excluded.czk_per_usd,
               czk_per_eur=excluded.czk_per_eur, fetched_at=excluded.fetched_at",
            params![day.date, day.czk_per_usd, day.czk_per_eur, now],
        )?;
        Ok(())
    }

    pub(crate) fn latest_fx(&self) -> Option<FxDay> {
        self.conn
            .query_row(
                "SELECT date, czk_per_usd, czk_per_eur FROM fx_rates ORDER BY date DESC LIMIT 1",
                [],
                |r| {
                    Ok(FxDay {
                        date: r.get(0)?,
                        czk_per_usd: r.get(1)?,
                        czk_per_eur: r.get(2)?,
                    })
                },
            )
            .optional()
            .ok()
            .flatten()
    }

    pub(crate) fn meta_get(&self, key: &str) -> Option<String> {
        self.conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()
            .ok()
            .flatten()
    }

    pub(crate) fn meta_set(&self, key: &str, value: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO meta(key, value) VALUES(?1, ?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub(crate) fn milestone_known(&self, id: &str) -> bool {
        self.conn
            .query_row("SELECT 1 FROM milestones WHERE id = ?1", [id], |_| Ok(()))
            .optional()
            .ok()
            .flatten()
            .is_some()
    }

    pub(crate) fn mark_milestone(&self, id: &str, at: i64, shown: bool) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO milestones(id, at, shown) VALUES(?1, ?2, ?3)",
            params![id, at, shown as i64],
        )?;
        Ok(())
    }

    pub(crate) fn add_limit_snapshot(
        &self,
        ts: i64,
        provider: &str,
        win: &str,
        percent: Option<f64>,
        resets_at: Option<i64>,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO limit_snapshots(provider, win, ts, percent, resets_at) VALUES(?1, ?2, ?3, ?4, ?5)",
            params![provider, win, ts, percent, resets_at],
        )?;
        Ok(())
    }

    /// Rows of the newest 5h and weekly snapshot of a provider (one per
    /// window). Other windows (Claude's Fable) keep their own times.
    pub(crate) fn latest_limit_snapshot(&self, provider: &str) -> Vec<LimitRow> {
        let Ok(mut stmt) = self.conn.prepare(
            "SELECT win, percent, resets_at, ts FROM limit_snapshots
             WHERE provider = ?1 AND win IN ('5h', 'week') AND ts = (
                 SELECT MAX(ts) FROM limit_snapshots WHERE provider = ?1 AND win IN ('5h', 'week'))",
        ) else {
            return Vec::new();
        };
        let rows = stmt.query_map([provider], |r| {
            Ok(LimitRow {
                win: r.get(0)?,
                percent: r.get(1)?,
                resets_at: r.get(2)?,
                ts: r.get(3)?,
            })
        });
        match rows {
            Ok(rows) => rows.flatten().collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Newest row of one window of a provider.
    pub(crate) fn latest_limit_row(&self, provider: &str, win: &str) -> Option<LimitRow> {
        self.conn
            .query_row(
                "SELECT win, percent, resets_at, ts FROM limit_snapshots
                 WHERE provider = ?1 AND win = ?2 ORDER BY ts DESC LIMIT 1",
                params![provider, win],
                |r| {
                    Ok(LimitRow {
                        win: r.get(0)?,
                        percent: r.get(1)?,
                        resets_at: r.get(2)?,
                        ts: r.get(3)?,
                    })
                },
            )
            .ok()
    }

    pub(crate) fn prune_limit_snapshots(&self, before: i64) -> rusqlite::Result<usize> {
        self.conn
            .execute("DELETE FROM limit_snapshots WHERE ts < ?1", [before])
    }

    /// "Smazat historii": wipe every table.
    pub(crate) fn clear_all(&mut self) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        for table in ALL_TABLES {
            tx.execute(&format!("DELETE FROM {table}"), [])?;
        }
        tx.commit()
    }

    #[cfg(test)]
    pub(crate) fn count(&self, table: &str) -> i64 {
        self.conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }
}

fn apply_record(tx: &rusqlite::Transaction<'_>, rec: &ClaudeRecord) -> rusqlite::Result<bool> {
    let hash = rec.key as i64;
    let new_vals = [
        rec.tokens[0] as i64,
        rec.tokens[1] as i64,
        rec.tokens[2] as i64,
        rec.tokens[3] as i64,
        rec.tokens[4] as i64,
        rec.web_search as i64,
    ];
    let existing = tx
        .prepare_cached(
            "SELECT hour, model, entrypoint, project, speed, service_tier, long_context,
                    input, output, cache_read, cache_write_5m, cache_write_1h, web_search
             FROM seen_messages WHERE hash = ?1",
        )?
        .query_row([hash], |r| {
            Ok((
                (
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, i64>(6)?,
                ),
                [
                    r.get::<_, i64>(7)?,
                    r.get::<_, i64>(8)?,
                    r.get::<_, i64>(9)?,
                    r.get::<_, i64>(10)?,
                    r.get::<_, i64>(11)?,
                    r.get::<_, i64>(12)?,
                ],
            ))
        })
        .optional()?;

    let (dims, deltas) = match existing {
        None => {
            tx.prepare_cached(
                "INSERT INTO seen_messages(hash, hour, model, entrypoint, project, speed, service_tier, long_context,
                   input, output, cache_read, cache_write_5m, cache_write_1h, web_search)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            )?
            .execute(params![
                hash,
                rec.hour,
                rec.model,
                rec.entrypoint,
                rec.project,
                rec.speed,
                rec.service_tier,
                rec.band as i64,
                new_vals[0],
                new_vals[1],
                new_vals[2],
                new_vals[3],
                new_vals[4],
                new_vals[5]
            ])?;
            (
                (
                    rec.hour,
                    rec.model.clone(),
                    rec.entrypoint.clone(),
                    rec.project.clone(),
                    rec.speed.clone(),
                    rec.service_tier.clone(),
                    rec.band as i64,
                ),
                new_vals,
            )
        }
        Some((dims, old)) => {
            let mut deltas = [0i64; 6];
            for i in 0..6 {
                deltas[i] = (new_vals[i] - old[i]).max(0);
            }
            if deltas.iter().all(|d| *d == 0) {
                return Ok(false);
            }
            tx.prepare_cached(
                "UPDATE seen_messages SET input = MAX(input, ?2), output = MAX(output, ?3),
                   cache_read = MAX(cache_read, ?4), cache_write_5m = MAX(cache_write_5m, ?5),
                   cache_write_1h = MAX(cache_write_1h, ?6), web_search = MAX(web_search, ?7)
                 WHERE hash = ?1",
            )?
            .execute(params![
                hash,
                new_vals[0],
                new_vals[1],
                new_vals[2],
                new_vals[3],
                new_vals[4],
                new_vals[5]
            ])?;
            (dims, deltas)
        }
    };

    let (hour, model, entrypoint, project, speed, tier, band) = dims;
    let mut add = tx.prepare_cached(
        "INSERT INTO usage_hourly(hour, provider, model, entrypoint, project, speed, service_tier, long_context, token_kind, tokens)
         VALUES(?1, 'claude', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(hour, provider, model, entrypoint, project, speed, service_tier, long_context, token_kind)
         DO UPDATE SET tokens = tokens + excluded.tokens",
    )?;
    let mut any = false;
    for (i, kind) in TokenKind::ALL.iter().enumerate() {
        if deltas[i] > 0 {
            add.execute(params![hour, model, entrypoint, project, speed, tier, band, kind.as_str(), deltas[i]])?;
            any = true;
        }
    }
    if deltas[5] > 0 {
        tx.prepare_cached(
            "INSERT INTO web_search_hourly(hour, provider, model, entrypoint, project, requests)
             VALUES(?1, 'claude', ?2, ?3, ?4, ?5)
             ON CONFLICT(hour, provider, model, entrypoint, project) DO UPDATE SET requests = requests + excluded.requests",
        )?
        .execute(params![hour, model, entrypoint, project, deltas[5]])?;
        any = true;
    }
    Ok(any)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(key: u64, output: u64) -> ClaudeRecord {
        ClaudeRecord {
            ts: 3600 * 10 + 5,
            hour: 3600 * 10,
            model: "claude-opus-5-5".into(),
            entrypoint: "cli".into(),
            project: "alpha".into(),
            speed: "standard".into(),
            service_tier: "standard".into(),
            band: 0,
            tokens: [10, output, 100, 5, 0],
            web_search: 0,
            key,
        }
    }

    fn state(offset: u64) -> FileState {
        FileState {
            created: Some(1),
            prefix_len: 0,
            prefix_hash: 0,
            size: offset,
            mtime: 1,
            offset,
        }
    }

    fn output_total(store: &Store) -> u64 {
        store
            .usage_rows(0, i64::MAX)
            .unwrap()
            .iter()
            .filter(|r| r.kind == TokenKind::Output)
            .map(|r| r.tokens)
            .sum()
    }

    #[test]
    fn wal_mode_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("nested").join("history.sqlite")).unwrap();
        assert_eq!(store.journal_mode().to_lowercase(), "wal");
    }

    #[test]
    fn dedupe_keeps_max_per_kind() {
        let mut store = Store::open_in_memory();
        // First line of a streamed message: partial output.
        assert_eq!(store.apply_claude_batch("a", &state(10), &[rec(7, 4)]).unwrap(), 1);
        // Same message again (later line, final output) adds only the difference.
        assert_eq!(store.apply_claude_batch("a", &state(20), &[rec(7, 400)]).unwrap(), 1);
        // Exact repeat (e.g. reread after truncation) changes nothing.
        assert_eq!(store.apply_claude_batch("a", &state(30), &[rec(7, 400)]).unwrap(), 0);
        // Smaller value never subtracts.
        assert_eq!(store.apply_claude_batch("a", &state(40), &[rec(7, 3)]).unwrap(), 0);
        assert_eq!(output_total(&store), 400);
        assert_eq!(store.load_files()["a"].offset, 40);
    }

    #[test]
    fn clear_all_wipes_every_table() {
        let mut store = Store::open_in_memory();
        store.apply_claude_batch("a", &state(10), &[rec(1, 5)]).unwrap();
        store.upsert_codex_buckets(&[("2026-10-01".into(), 5)], 1).unwrap();
        store.put_fx(&FxDay::fallback(), 1).unwrap();
        store.mark_milestone("m", 1, false).unwrap();
        store.add_limit_snapshot(1, "codex", "5h", Some(1.0), None).unwrap();
        store.meta_set("k", "v").unwrap();
        store.clear_all().unwrap();
        for t in ALL_TABLES {
            assert_eq!(store.count(t), 0, "{t} not empty");
        }
    }

    #[test]
    fn limit_snapshots_prune_and_latest() {
        let store = Store::open_in_memory();
        store.add_limit_snapshot(100, "codex", "5h", Some(10.0), Some(500)).unwrap();
        store.add_limit_snapshot(200, "codex", "5h", Some(20.0), Some(500)).unwrap();
        store.add_limit_snapshot(200, "codex", "week", Some(5.0), Some(900)).unwrap();
        let latest = store.latest_limit_snapshot("codex");
        assert_eq!(latest.len(), 2);
        assert!(latest.iter().all(|r| r.ts == 200));
        assert_eq!(store.prune_limit_snapshots(150).unwrap(), 1);
        assert_eq!(store.count("limit_snapshots"), 2);
    }

    #[test]
    fn a_newer_fable_row_keeps_the_main_snapshot_and_has_its_own_latest() {
        let store = Store::open_in_memory();
        store.add_limit_snapshot(200, "claude", "5h", Some(20.0), Some(500)).unwrap();
        store.add_limit_snapshot(200, "claude", "week", Some(5.0), Some(900)).unwrap();
        store.add_limit_snapshot(250, "claude", "fable", Some(3.0), Some(900)).unwrap();
        store.add_limit_snapshot(300, "claude", "fable", Some(4.0), Some(900)).unwrap();
        let main = store.latest_limit_snapshot("claude");
        assert_eq!(main.len(), 2);
        assert!(main.iter().all(|r| r.ts == 200));
        let fable = store.latest_limit_row("claude", "fable").unwrap();
        assert_eq!((fable.ts, fable.percent), (300, Some(4.0)));
        assert!(store.latest_limit_row("codex", "fable").is_none());
    }

    #[test]
    fn codex_buckets_upsert_by_date() {
        let mut store = Store::open_in_memory();
        store.upsert_codex_buckets(&[("2026-10-01".into(), 5), ("2026-10-02".into(), 7)], 1).unwrap();
        store.upsert_codex_buckets(&[("2026-10-02".into(), 9)], 2).unwrap();
        assert_eq!(
            store.codex_buckets().unwrap(),
            vec![("2026-10-01".to_string(), 5), ("2026-10-02".to_string(), 9)]
        );
    }
}
