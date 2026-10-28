//! Feedback ("Zpětná vazba") to the Cloudflare Worker in `feedback/`.
//!
//! The only network path that carries text the user wrote, and it goes to one
//! address: compiled in, or overridden by `SKYLIMIT_FEEDBACK_URL` in the
//! environment of whoever starts the app (for `wrangler dev`). The webview
//! cannot choose the address; it sends only the content, which is checked
//! again here. The Worker owns the GitHub token; the app carries no secret.
//!
//! ```text
//!  dialog ──feedback_send──▶ build_payload (+ version, OS) ──POST──▶ Worker ──▶ private repo issue
//! ```

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

/// Where feedback goes unless the environment says otherwise.
pub const DEFAULT_ENDPOINT: &str = "https://skylimit-feedback.michal-jasek.workers.dev/v1/feedback";
const OVERRIDE_VAR: &str = "SKYLIMIT_FEEDBACK_URL";

/// Same limits as the Worker. The Worker decides; these only spare a request
/// that would obviously be refused.
pub const MAX_MESSAGE: usize = 10_000;
pub const MAX_ATTACHMENT_BASE64: usize = 2 * 1024 * 1024 * 4 / 3 + 4;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackAuthor {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub email: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackAttachment {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    /// Content in base64.
    pub data: String,
}

/// What the dialog sends.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackRequest {
    /// `bug` | `change` | `improvement`.
    pub kind: String,
    pub message: String,
    /// Where in the app the user was, e.g. "Okno aplikace › Měsíc".
    #[serde(default)]
    pub location: String,
    /// `None` = anonymous.
    #[serde(default)]
    pub author: Option<FeedbackAuthor>,
    #[serde(default)]
    pub attachment: Option<FeedbackAttachment>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackContext {
    pub version: String,
    pub os: String,
    pub location: String,
}

/// What reaches the Worker. Contract with `feedback/src/feedback.ts`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackPayload {
    pub kind: String,
    pub message: String,
    pub context: FeedbackContext,
    pub author: Option<FeedbackAuthor>,
    pub attachment: Option<FeedbackAttachment>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackReceipt {
    pub id: String,
}

/// What the dialog needs up front: can it send, and where will it go.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackStatus {
    pub available: bool,
    /// Host the message goes to, shown in the dialog.
    pub host: String,
    pub version: String,
    pub os: String,
    pub error: String,
}

/// Check the dialog's message and add what only Rust knows.
pub fn build_payload(request: FeedbackRequest, version: &str, os: &str) -> Result<FeedbackPayload, String> {
    if !matches!(request.kind.as_str(), "bug" | "change" | "improvement") {
        return Err("Neznámý druh zpětné vazby.".into());
    }
    let message = request.message.trim().to_string();
    if message.is_empty() {
        return Err("Napiš, o co jde. Zpráva je prázdná.".into());
    }
    if message.chars().count() > MAX_MESSAGE {
        return Err("Zpráva je příliš dlouhá.".into());
    }
    if request
        .attachment
        .as_ref()
        .is_some_and(|a| a.data.len() > MAX_ATTACHMENT_BASE64)
    {
        return Err("Příloha je větší než 2 MB.".into());
    }
    // An empty author is the same as anonymous; the Worker would drop it too.
    let author = request
        .author
        .filter(|a| !a.name.trim().is_empty() || !a.email.trim().is_empty());
    Ok(FeedbackPayload {
        kind: request.kind,
        message,
        context: FeedbackContext {
            version: version.to_string(),
            os: os.to_string(),
            location: request.location.trim().to_string(),
        },
        author,
        attachment: request.attachment,
    })
}

/// Where to send: the override, else the compiled-in address. An override
/// must be https, or http to this machine (for `wrangler dev`): text from
/// the app never leaves unencrypted over the network.
pub fn resolve_endpoint(override_url: Option<&str>) -> Result<String, String> {
    let chosen = override_url
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .unwrap_or(DEFAULT_ENDPOINT);
    let local = ["http://127.0.0.1:", "http://localhost:", "http://127.0.0.1/", "http://localhost/"]
        .iter()
        .any(|prefix| chosen.starts_with(prefix));
    let valid = (chosen.starts_with("https://") || local) && !chosen.chars().any(|c| c.is_whitespace() || c.is_control());
    if !valid {
        return Err(format!("{chosen} není adresa, kam se smí zpětná vazba posílat."));
    }
    Ok(chosen.to_string())
}

/// Host part of the address, shown in the dialog.
pub fn endpoint_host(url: &str) -> String {
    url.split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or(url)
        .to_string()
}

fn override_url() -> Option<String> {
    std::env::var(OVERRIDE_VAR).ok()
}

fn os_label() -> String {
    format!("{} ({})", std::env::consts::OS, std::env::consts::ARCH)
}

#[tauri::command]
pub fn feedback_status(app: AppHandle) -> FeedbackStatus {
    let version = app.package_info().version.to_string();
    match resolve_endpoint(override_url().as_deref()) {
        Ok(url) => FeedbackStatus {
            available: true,
            host: endpoint_host(&url),
            version,
            os: os_label(),
            error: String::new(),
        },
        Err(error) => FeedbackStatus {
            available: false,
            host: String::new(),
            version,
            os: os_label(),
            error,
        },
    }
}

/// Blocking HTTP on the blocking pool, so no async worker waits on the network.
#[tauri::command]
pub async fn feedback_send(app: AppHandle, request: FeedbackRequest) -> Result<FeedbackReceipt, String> {
    let version = app.package_info().version.to_string();
    let payload = build_payload(request, &version, &os_label())?;
    let url = resolve_endpoint(override_url().as_deref())?;
    tauri::async_runtime::spawn_blocking(move || send(&url, &version, &payload))
        .await
        .map_err(|e| format!("Odesílání se přerušilo: {e}"))?
}

/// Post the message and read the Worker's answer: `{ ok, id }`, or
/// `{ ok: false, error }` with a sentence fit for the user. Without any
/// answer (offline) a plain message; the text stays in the dialog.
pub(crate) fn send(url: &str, version: &str, payload: &FeedbackPayload) -> Result<FeedbackReceipt, String> {
    // rustls is built without a default crypto provider; install ring once.
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    let client = reqwest::blocking::Client::builder()
        .user_agent(format!("Skylimit/{version}"))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Odesílání se nepodařilo připravit: {e}"))?;
    let body = serde_json::to_vec(payload).map_err(|e| format!("Zprávu se nepodařilo připravit: {e}"))?;
    let response = client
        .post(url)
        .header("x-skylimit-version", version)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .map_err(|_| "Zpětnou vazbu se nepodařilo odeslat. Jsi připojený k internetu?".to_string())?;
    let status = response.status();
    let answer: serde_json::Value = response
        .text()
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(serde_json::Value::Null);
    if status.is_success() && answer["ok"] == serde_json::Value::Bool(true) {
        return Ok(FeedbackReceipt {
            id: answer["id"].as_str().unwrap_or_default().to_string(),
        });
    }
    Err(answer["error"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| format!("Příjem zpětné vazby odpověděl {}.", status.as_u16())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    fn request(json: &str) -> FeedbackRequest {
        serde_json::from_str(json).unwrap()
    }

    fn payload() -> FeedbackPayload {
        build_payload(
            request(r#"{"kind":"improvement","message":"Tmavší widget, prosím.","location":"Okno aplikace › Nastavení"}"#),
            "0.4.0",
            "windows (x86_64)",
        )
        .unwrap()
    }

    #[test]
    fn the_payload_carries_what_the_worker_reads() {
        let p = build_payload(
            request(r#"{"kind":"bug","message":"  Nejde to.  ","location":"Okno aplikace › Měsíc","author":{"name":"Lumír","email":""}}"#),
            "0.4.0",
            "macos (aarch64)",
        )
        .unwrap();
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json["kind"], "bug");
        assert_eq!(json["message"], "Nejde to.");
        assert_eq!(json["context"]["version"], "0.4.0");
        assert_eq!(json["context"]["os"], "macos (aarch64)");
        assert_eq!(json["context"]["location"], "Okno aplikace › Měsíc");
        assert_eq!(json["author"]["name"], "Lumír");
        assert!(json["attachment"].is_null());
    }

    #[test]
    fn an_attachment_keeps_its_type_field_name() {
        let p = build_payload(
            request(r#"{"kind":"bug","message":"x","attachment":{"name":"a.png","type":"image/png","data":"AAAA"}}"#),
            "v",
            "o",
        )
        .unwrap();
        assert_eq!(serde_json::to_value(&p).unwrap()["attachment"]["type"], "image/png");
    }

    #[test]
    fn empty_author_is_anonymous_and_bad_input_is_refused() {
        let anonymous = build_payload(
            request(r#"{"kind":"change","message":"x","author":{"name":" ","email":""}}"#),
            "v",
            "o",
        )
        .unwrap();
        assert!(anonymous.author.is_none());
        assert!(build_payload(request(r#"{"kind":"spam","message":"x"}"#), "v", "o").is_err());
        assert!(build_payload(request(r#"{"kind":"bug","message":"   "}"#), "v", "o").is_err());
        let long = format!(r#"{{"kind":"bug","message":"{}"}}"#, "a".repeat(MAX_MESSAGE + 1));
        assert!(build_payload(request(&long), "v", "o").is_err());
        let big = format!(
            r#"{{"kind":"bug","message":"x","attachment":{{"name":"a.png","type":"image/png","data":"{}"}}}}"#,
            "A".repeat(MAX_ATTACHMENT_BASE64 + 4)
        );
        assert_eq!(build_payload(request(&big), "v", "o").unwrap_err(), "Příloha je větší než 2 MB.");
    }

    #[test]
    fn only_https_or_this_machine() {
        assert_eq!(resolve_endpoint(None).unwrap(), DEFAULT_ENDPOINT);
        assert_eq!(resolve_endpoint(Some("  ")).unwrap(), DEFAULT_ENDPOINT);
        assert!(resolve_endpoint(Some("http://127.0.0.1:8787/v1/feedback")).is_ok());
        assert!(resolve_endpoint(Some("https://example.com/v1/feedback")).is_ok());
        assert!(resolve_endpoint(Some("http://example.com/v1/feedback")).is_err());
        assert!(resolve_endpoint(Some("file:///etc/passwd")).is_err());
        assert!(resolve_endpoint(Some("https://exa mple.com")).is_err());
        assert_eq!(endpoint_host(DEFAULT_ENDPOINT), "skylimit-feedback.michal-jasek.workers.dev");
    }

    /// One-connection server: answers and reports what arrived.
    fn one_shot(status: &str, body: &'static str) -> (String, mpsc::Receiver<(String, String)>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1/feedback", listener.local_addr().unwrap());
        let (sender, receiver) = mpsc::channel();
        let status = status.to_string();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut headers = String::new();
            let mut length = 0usize;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap();
                }
                headers.push_str(&line);
            }
            let mut body_bytes = vec![0; length];
            reader.read_exact(&mut body_bytes).unwrap();
            let reply = format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(reply.as_bytes()).unwrap();
            sender.send((headers, String::from_utf8(body_bytes).unwrap())).unwrap();
        });
        (url, receiver)
    }

    #[test]
    fn sends_the_payload_with_the_app_headers_and_reads_the_receipt() {
        let (url, received) = one_shot("200 OK", r#"{"ok":true,"id":"abcd1234","number":3}"#);
        let receipt = send(&url, "0.4.0", &payload()).unwrap();
        assert_eq!(receipt.id, "abcd1234");
        let (headers, body) = received.recv().unwrap();
        let headers = headers.to_ascii_lowercase();
        assert!(headers.contains("x-skylimit-version: 0.4.0"), "{headers}");
        assert!(headers.contains("user-agent: skylimit/0.4.0"), "{headers}");
        assert!(headers.contains("content-type: application/json"), "{headers}");
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["kind"], "improvement");
        assert_eq!(json["context"]["location"], "Okno aplikace › Nastavení");
        assert!(json["author"].is_null(), "no author means anonymous");
    }

    #[test]
    fn the_workers_own_reason_reaches_the_user() {
        let (url, _received) = one_shot("429 Too Many Requests", r#"{"ok":false,"error":"Zpráv přišlo moc najednou."}"#);
        assert_eq!(send(&url, "0.4.0", &payload()).unwrap_err(), "Zpráv přišlo moc najednou.");
    }

    #[test]
    fn no_server_means_a_plain_message() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1/feedback", listener.local_addr().unwrap());
        drop(listener);
        assert!(send(&url, "0.4.0", &payload()).unwrap_err().contains("připojený k internetu"));
    }
}
