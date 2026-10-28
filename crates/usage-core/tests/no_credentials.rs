//! Claude limits are OFF by decision (Anthropic terms forbid third-party apps
//! from collecting or using Claude OAuth tokens) and Codex login stays inside
//! the official CLI. Guard against any code path that would read those files
//! or call the Anthropic usage endpoint.

use std::fs;
use std::path::PathBuf;

#[test]
fn no_code_path_touches_login_files_or_anthropic_api() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let forbidden = [
        ".credentials.json",
        "credentials",
        "api.anthropic.com",
        "oauth/usage",
        "auth.json",
        "accessToken",
        "Authorization",
        "Bearer",
    ];
    let mut checked = 0;
    for entry in fs::read_dir(&src).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "rs") {
            let text = fs::read_to_string(&path).unwrap();
            for needle in forbidden {
                assert!(
                    !text.contains(needle),
                    "{} mentions `{needle}`",
                    path.display()
                );
            }
            checked += 1;
        }
    }
    assert!(checked >= 8, "only {checked} source files checked");
}
