use serde::{Deserialize, Serialize};
use std::env;
use std::io::{self, BufRead, Write};
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// JSON-RPC types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct JsonRpcMessage {
    #[serde(default)]
    id: Option<serde_json::Value>,
    method: Option<String>,
    #[serde(default)]
    params: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Serialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

// ---------------------------------------------------------------------------
// LSP types for textDocument sync
// ---------------------------------------------------------------------------

#[allow(dead_code)]
#[derive(Deserialize)]
struct DidOpenParams {
    #[serde(rename = "textDocument")]
    text_document: TextDocumentItem,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct DidChangeParams {
    #[serde(rename = "textDocument")]
    text_document: VersionedTextDocumentIdentifier,
    #[serde(default)]
    content_changes: Vec<TextDocumentContentChangeEvent>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct DidSaveParams {
    #[serde(rename = "textDocument")]
    text_document: TextDocumentIdentifier,
    #[serde(default)]
    text: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct TextDocumentItem {
    uri: String,
    #[serde(rename = "languageId")]
    language_id: String,
    #[serde(default)]
    text: String,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct VersionedTextDocumentIdentifier {
    uri: String,
    #[serde(default)]
    version: i64,
}

#[derive(Deserialize)]
struct TextDocumentIdentifier {
    uri: String,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct TextDocumentContentChangeEvent {
    #[serde(default)]
    text: String,
}

// ---------------------------------------------------------------------------
// Heartbeat payload (matches codetrackr-vscode format)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct HeartbeatPayload {
    project: String,
    file: String,
    lang: String,
    branch: String,
    editor: String,
    os: String,
    duration: u64,
    is_write: bool,
    time: u64,
}

impl HeartbeatPayload {
    fn from_lsp_event(uri: &str, language_id: &str, is_write: bool) -> Self {
        let file_path = uri.strip_prefix("file://").unwrap_or(uri);
        let project = extract_project(file_path);

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        HeartbeatPayload {
            project,
            file: file_path.to_string(),
            lang: language_id.to_string(),
            branch: String::new(),
            editor: "Zed".to_string(),
            os: env::consts::OS.to_string(),
            duration: 60,
            is_write,
            time: now,
        }
    }
}

fn extract_project(path: &str) -> String {
    let components: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if components.len() >= 3 {
        components[components.len() - 2].to_string()
    } else if components.len() >= 2 {
        components[1].to_string()
    } else {
        "Unknown".to_string()
    }
}

// ---------------------------------------------------------------------------
// Heartbeat sender
// ---------------------------------------------------------------------------

struct CodeTrackrClient {
    api_key: String,
    base_url: String,
    last_file: String,
    last_time: u64,
}

impl CodeTrackrClient {
    fn from_env() -> Self {
        CodeTrackrClient {
            api_key: env::var("CODETRACKR_API_KEY").unwrap_or_default(),
            base_url: env::var("CODETRACKR_BASE_URL")
                .unwrap_or_else(|_| "https://codetrackr.fly.dev".to_string()),
            last_file: String::new(),
            last_time: 0,
        }
    }

    fn should_send(&self, uri: &str, is_write: bool) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        is_write || uri != self.last_file || now - self.last_time >= 120
    }

    fn send_heartbeat(&mut self, payload: &HeartbeatPayload) {
        if self.api_key.is_empty() {
            eprintln!("[CodeTrackr] No API key set. Set CODETRACKR_API_KEY env var.");
            return;
        }

        let url = format!(
            "{}/api/v1/heartbeat",
            self.base_url.trim_end_matches('/')
        );

        let body = serde_json::to_string(payload).unwrap_or_default();

        match ureq::post(&url)
            .set("Content-Type", "application/json")
            .set("X-API-Key", &self.api_key)
            .send_string(&body)
        {
            Ok(resp) if resp.status() == 200 || resp.status() == 201 => {
                self.last_file = payload.file.clone();
                self.last_time = payload.time;
                eprintln!(
                    "[CodeTrackr] Heartbeat sent: {} ({})",
                    payload.file, payload.lang
                );
            }
            Ok(resp) => {
                eprintln!(
                    "[CodeTrackr] API error: {} {}",
                    resp.status(),
                    resp.into_string().unwrap_or_default()
                );
            }
            Err(e) => {
                eprintln!("[CodeTrackr] Connection error: {}", e);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// LSP message I/O
// ---------------------------------------------------------------------------

fn read_message(reader: &mut dyn BufRead) -> Option<String> {
    let mut content_length: Option<usize> = None;

    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some(len) = trimmed.strip_prefix("Content-Length: ") {
            content_length = len.trim().parse().ok();
        }
    }

    let len = content_length?;
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).ok()?;
    Some(String::from_utf8(buf).unwrap_or_default())
}

fn send_response(response: &JsonRpcResponse) {
    let body = serde_json::to_string(response).unwrap();
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    write!(handle, "Content-Length: {}\r\n\r\n{}", body.len(), body).ok();
    handle.flush().ok();
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    let mut client = CodeTrackrClient::from_env();
    let mut reader = io::BufReader::new(io::stdin());

    eprintln!("[CodeTrackr] Language Server starting...");

    loop {
        let Some(raw) = read_message(&mut reader) else {
            break;
        };

        let msg: JsonRpcMessage = match serde_json::from_str(&raw) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("[CodeTrackr] Failed to parse message: {}", e);
                continue;
            }
        };

        match msg.method.as_deref() {
            Some("initialize") => {
                let capabilities = serde_json::json!({
                    "capabilities": {
                        "textDocumentSync": {
                            "openClose": true,
                            "change": 1,
                            "save": {
                                "includeText": false
                            }
                        }
                    }
                });

                send_response(&JsonRpcResponse {
                    jsonrpc: "2.0",
                    id: msg.id.unwrap_or(serde_json::Value::Null),
                    result: Some(capabilities),
                    error: None,
                });

                eprintln!("[CodeTrackr] LSP initialized.");
            }

            Some("initialized") => {
                eprintln!("[CodeTrackr] Client initialized, tracking started.");
            }

            Some("textDocument/didOpen") => {
                if let Some(params) = msg.params {
                    if let Ok(p) = serde_json::from_value::<DidOpenParams>(params) {
                        let payload = HeartbeatPayload::from_lsp_event(
                            &p.text_document.uri,
                            &p.text_document.language_id,
                            false,
                        );
                        if client.should_send(&payload.file, false) {
                            client.send_heartbeat(&payload);
                        }
                    }
                }
            }

            Some("textDocument/didChange") => {
                if let Some(params) = msg.params {
                    if let Ok(p) = serde_json::from_value::<DidChangeParams>(params) {
                        let lang = infer_language_from_uri(&p.text_document.uri);
                        let payload = HeartbeatPayload::from_lsp_event(
                            &p.text_document.uri,
                            &lang,
                            false,
                        );
                        if client.should_send(&payload.file, false) {
                            client.send_heartbeat(&payload);
                        }
                    }
                }
            }

            Some("textDocument/didSave") => {
                if let Some(params) = msg.params {
                    if let Ok(p) = serde_json::from_value::<DidSaveParams>(params) {
                        let lang = infer_language_from_uri(&p.text_document.uri);
                        let payload = HeartbeatPayload::from_lsp_event(
                            &p.text_document.uri,
                            &lang,
                            true,
                        );
                        client.send_heartbeat(&payload);
                    }
                }
            }

            Some("shutdown") => {
                send_response(&JsonRpcResponse {
                    jsonrpc: "2.0",
                    id: msg.id.unwrap_or(serde_json::Value::Null),
                    result: Some(serde_json::Value::Null),
                    error: None,
                });
                eprintln!("[CodeTrackr] Shutting down.");
                break;
            }

            Some("exit") => break,

            _ => {
                if let Some(id) = msg.id {
                    if id.is_number() || id.is_string() {
                        send_response(&JsonRpcResponse {
                            jsonrpc: "2.0",
                            id,
                            result: None,
                            error: Some(JsonRpcError {
                                code: -32601,
                                message: "Method not found".to_string(),
                            }),
                        });
                    }
                }
            }
        }
    }
}

/// Infer language from file extension (fallback when didChange/didSave don't include languageId)
fn infer_language_from_uri(uri: &str) -> String {
    let path = uri.strip_prefix("file://").unwrap_or(uri);
    let ext = path.rsplit('.').next().unwrap_or("");
    match ext {
        "rs" => "Rust",
        "ts" | "tsx" => "TypeScript",
        "js" | "jsx" => "JavaScript",
        "py" => "Python",
        "go" => "Go",
        "rb" => "Ruby",
        "java" => "Java",
        "kt" | "kts" => "Kotlin",
        "swift" => "Swift",
        "c" | "h" => "C",
        "cpp" | "hpp" | "cc" | "cxx" => "C++",
        "cs" => "C#",
        "php" => "PHP",
        "vue" => "Vue",
        "svelte" => "Svelte",
        "html" => "HTML",
        "css" | "scss" | "less" => "CSS",
        "json" => "JSON",
        "md" => "Markdown",
        "yaml" | "yml" => "YAML",
        "toml" => "TOML",
        "sh" | "bash" | "zsh" => "Shell",
        "sql" => "SQL",
        "dart" => "Dart",
        "lua" => "Lua",
        "elm" => "Elm",
        "gleam" => "Gleam",
        "clj" | "cljs" | "cljc" => "Clojure",
        "ex" | "exs" => "Elixir",
        "erl" => "Erlang",
        "hs" => "Haskell",
        "nim" => "Nim",
        "r" | "R" => "R",
        "zig" => "Zig",
        _ => ext,
    }
    .to_string()
}
