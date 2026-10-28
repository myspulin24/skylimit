//! Diagnostic log (`diag_dir/diag.log`).
//!
//! Metadata only (decision D6): file path, offset, line length, parser error
//! category, top-level JSON keys WITHOUT values, and for unknown API JSON the
//! key/type structure. Never message text, tokens or headers. 5 MB cap with
//! one rotation (`diag.log.1`).

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::Value;

pub(crate) const DEFAULT_CAP: u64 = 5 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct Diag {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    path: PathBuf,
    cap: u64,
}

impl Diag {
    pub(crate) fn new(dir: &Path) -> Self {
        Self::with_cap(dir, DEFAULT_CAP)
    }

    pub(crate) fn with_cap(dir: &Path, cap: u64) -> Self {
        let _ = fs::create_dir_all(dir);
        Diag {
            inner: Arc::new(Mutex::new(Inner {
                path: dir.join("diag.log"),
                cap,
            })),
        }
    }

    /// Append one line. Failures are swallowed: diagnostics must never take
    /// the core down.
    pub(crate) fn log(&self, now: i64, area: &str, message: &str) {
        let Ok(inner) = self.inner.lock() else {
            return;
        };
        let stamp = chrono::DateTime::from_timestamp(now, 0)
            .map(|d| d.format("%Y-%m-%dT%H:%M:%SZ").to_string())
            .unwrap_or_else(|| now.to_string());
        // One line per entry, whatever the caller passed.
        let clean: String = message
            .chars()
            .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
            .collect();
        let line = format!("{stamp} [{area}] {clean}\n");

        if let Ok(meta) = fs::metadata(&inner.path) {
            if meta.len() + line.len() as u64 > inner.cap {
                let rotated = inner.path.with_extension("log.1");
                let _ = fs::remove_file(&rotated);
                let _ = fs::rename(&inner.path, &rotated);
            }
        }
        if let Ok(mut f) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&inner.path)
        {
            let _ = f.write_all(line.as_bytes());
        }
    }

    #[cfg(test)]
    pub(crate) fn path(&self) -> PathBuf {
        self.inner.lock().unwrap().path.clone()
    }
}

/// Key/type structure of a JSON value without any values.
/// Arrays show the shape of their first element only.
pub(crate) fn json_shape(value: &Value) -> Value {
    match value {
        Value::Null => Value::String("null".into()),
        Value::Bool(_) => Value::String("bool".into()),
        Value::Number(_) => Value::String("number".into()),
        Value::String(_) => Value::String("string".into()),
        Value::Array(items) => Value::Array(items.first().map(json_shape).into_iter().collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), json_shape(v)))
                .collect(),
        ),
    }
}

/// Top-level keys of a JSON object line, without values. `None` when the line
/// is not a JSON object.
pub(crate) fn top_level_keys(line: &[u8]) -> Option<Vec<String>> {
    let map: serde_json::Map<String, Value> = serde_json::from_slice(line).ok()?;
    Some(map.keys().cloned().collect())
}

/// A serde_json error reduced to category and column. The `Display` form can
/// quote values from the input ("invalid type: string \"...\""), so it is never
/// written to the log.
pub(crate) fn error_category(err: &serde_json::Error) -> String {
    use serde_json::error::Category;
    let cat = match err.classify() {
        Category::Io => "io",
        Category::Syntax => "syntax",
        Category::Data => "data",
        Category::Eof => "eof",
    };
    format!("{cat}@col{}", err.column())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn shape_drops_values() {
        let v = json!({"a": "secret", "b": [{"c": 1}], "d": null, "e": true});
        let s = json_shape(&v);
        assert_eq!(
            s,
            json!({"a": "string", "b": [{"c": "number"}], "d": "null", "e": "bool"})
        );
        assert!(!s.to_string().contains("secret"));
    }

    #[test]
    fn error_category_never_quotes_input() {
        #[derive(serde::Deserialize, Debug)]
        #[allow(dead_code)]
        struct T {
            n: u64,
        }
        let err = serde_json::from_str::<T>(r#"{"n":"TOP-SECRET"}"#).unwrap_err();
        assert!(err.to_string().contains("TOP-SECRET"));
        let cat = error_category(&err);
        assert!(!cat.contains("TOP-SECRET"));
        assert!(cat.starts_with("data@col"));
    }

    #[test]
    fn rotates_at_cap() {
        let dir = tempfile::tempdir().unwrap();
        let diag = Diag::with_cap(dir.path(), 200);
        for i in 0..20 {
            diag.log(0, "t", &format!("entry {i} ..............................."));
        }
        let main = fs::metadata(diag.path()).unwrap().len();
        assert!(main <= 200, "main log over cap: {main}");
        assert!(dir.path().join("diag.log.1").exists());
        // Only one rotation is kept.
        assert!(!dir.path().join("diag.log.2").exists());
    }
}
