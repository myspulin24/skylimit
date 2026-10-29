//! PATH for the Codex CLI on macOS and Linux.
//!
//! An app started from Finder, the Dock, a login item or the relaunch after
//! an update gets launchd's bare PATH (`/usr/bin:/bin:/usr/sbin:/sbin`), not
//! the terminal's. Codex usually comes from npm (Node through nvm, Homebrew,
//! Volta, …) or Homebrew, and those directories are added in `~/.zshrc` or
//! `~/.bashrc`, which a login shell alone (`zsh -lc`) does not read. So the
//! user's shell is asked once, interactively, for its PATH (the idea behind
//! Tauri's fix-path-env), and common install directories are added in case
//! it does not answer.
//!
//! ```text
//!  $SHELL -i -l -c 'printf "\n<mark>%s\n" "$PATH"'   (3 s at most, stdin /dev/null)
//!      ─▶ text after the last mark ─▶ shell PATH : inherited PATH : known dirs
//! ```

// Only the tests use the helpers on Windows, where PATH needs no fixing.
#![cfg_attr(not(unix), allow(dead_code))]

use std::path::{Path, PathBuf};

const MARK: &str = "__SKYLIMIT_PATH__";

/// The PATH to give the Codex CLI, or `None` to keep the inherited one.
#[cfg(unix)]
pub(crate) fn for_codex(shell: &str, diag: &crate::diag::Diag, now: i64) -> Option<String> {
    let from_shell = ask_shell(shell, std::time::Duration::from_secs(3));
    let inherited = std::env::var("PATH").unwrap_or_default();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let known = home.as_deref().map(known_dirs).unwrap_or_default();
    let merged = merge([from_shell.as_deref().unwrap_or(""), &inherited], &known);
    diag.log(
        now,
        "codex",
        &format!(
            "PATH for codex: login shell {}, {} entries",
            if from_shell.is_some() { "answered" } else { "did not answer" },
            merged.split(':').count()
        ),
    );
    Some(merged)
}

#[cfg(not(unix))]
pub(crate) fn for_codex(_shell: &str, _diag: &crate::diag::Diag, _now: i64) -> Option<String> {
    None
}

/// The user's interactive login shell prints its PATH between markers, so
/// whatever its startup files print (greetings, prompts) is ignored.
#[cfg(unix)]
fn ask_shell(shell: &str, timeout: std::time::Duration) -> Option<String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::Instant;

    let fish = Path::new(shell).file_name().is_some_and(|n| n == "fish");
    let script = if fish {
        format!("printf '\\n{MARK}%s\\n' (string join : $PATH)")
    } else {
        format!("printf '\\n{MARK}%s\\n' \"$PATH\"")
    };
    let mut child = Command::new(shell)
        .args(["-i", "-l", "-c", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        text
    });
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < timeout => std::thread::sleep(std::time::Duration::from_millis(20)),
            _ => {
                // A startup file that waits for something: give up on it.
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    parse(&reader.join().ok()?)
}

fn parse(output: &str) -> Option<String> {
    output
        .lines()
        .rev()
        .find_map(|line| line.split_once(MARK).map(|(_, path)| path.trim().to_owned()))
        .filter(|path| path.contains('/'))
}

/// Where Codex and Node usually live, if those folders exist.
fn known_dirs(home: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = [
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        home.join(".local/bin"),
        home.join(".npm-global/bin"),
        home.join(".volta/bin"),
        home.join(".bun/bin"),
    ]
    .into_iter()
    .collect();
    // nvm: every installed Node that has codex, newest name first.
    if let Ok(entries) = std::fs::read_dir(home.join(".nvm/versions/node")) {
        let mut nvm: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path().join("bin"))
            .filter(|bin| bin.join("codex").exists())
            .collect();
        nvm.sort();
        nvm.reverse();
        dirs.extend(nvm);
    }
    dirs.retain(|d| d.is_dir());
    dirs
}

/// Entries in order of first appearance, empty ones dropped.
fn merge<'a>(paths: impl IntoIterator<Item = &'a str>, extra: &[PathBuf]) -> String {
    let mut seen: Vec<String> = Vec::new();
    let extra: Vec<String> = extra.iter().map(|d| d.to_string_lossy().into_owned()).collect();
    for entry in paths
        .into_iter()
        .flat_map(|p| p.split(':'))
        .map(str::to_owned)
        .chain(extra)
    {
        if !entry.is_empty() && !seen.contains(&entry) {
            seen.push(entry);
        }
    }
    seen.join(":")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_the_marked_line_and_ignores_what_startup_files_print() {
        let out = format!("Welcome back!\n\x1b[1m prompt \x1b[0m\n\n{MARK}/Users/a/.nvm/versions/node/v22/bin:/opt/homebrew/bin:/usr/bin\n");
        assert_eq!(parse(&out).as_deref(), Some("/Users/a/.nvm/versions/node/v22/bin:/opt/homebrew/bin:/usr/bin"));
        assert_eq!(parse("no marker here"), None);
        assert_eq!(parse(&format!("{MARK}\n")), None, "an empty PATH is no answer");
    }

    #[test]
    fn merges_in_order_without_duplicates() {
        let merged = merge(
            ["/opt/homebrew/bin:/usr/bin", "/usr/bin:/bin::/usr/sbin"],
            &[PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/Users/a/.volta/bin")],
        );
        assert_eq!(merged, "/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/Users/a/.volta/bin");
    }

    #[test]
    fn finds_codex_installed_through_nvm() {
        let home = tempfile::tempdir().unwrap();
        for v in ["v20.1.0", "v22.3.0"] {
            std::fs::create_dir_all(home.path().join(".nvm/versions/node").join(v).join("bin")).unwrap();
        }
        let with_codex = home.path().join(".nvm/versions/node/v22.3.0/bin");
        std::fs::write(with_codex.join("codex"), "").unwrap();
        let dirs = known_dirs(home.path());
        assert!(dirs.contains(&with_codex));
        assert!(!dirs.iter().any(|d| d.to_string_lossy().contains("v20.1.0")), "Node without codex is skipped");
    }

    /// sh everywhere, zsh as on a Mac, bash as on most Linux desktops.
    #[cfg(unix)]
    #[test]
    fn asks_real_shells() {
        for shell in ["/bin/sh", "/bin/zsh", "/bin/bash"] {
            if !Path::new(shell).exists() {
                continue;
            }
            let path = ask_shell(shell, std::time::Duration::from_secs(10)).unwrap_or_else(|| panic!("{shell} answers"));
            assert!(path.split(':').any(|p| p == "/bin" || p == "/usr/bin"), "{shell}: {path}");
        }
    }
}
