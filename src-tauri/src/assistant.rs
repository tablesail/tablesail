//! The AI assistant: asks the Anthropic Messages API to write SQL for the open
//! connection. The API key lives in the OS keychain (or `ANTHROPIC_API_KEY`) and
//! never leaves this module; the UI only learns whether one exists.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use dbcore::Error;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Notify;

use crate::keychain;

type Result<T> = std::result::Result<T, Error>;

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";
const DEFAULT_MODEL: &str = "claude-sonnet-5-5";
const MAX_TOKENS: u32 = 2048;
const TIMEOUT: Duration = Duration::from_secs(90);
/// The UI trims its schema listing to about 30k characters; this is the backstop.
const MAX_SCHEMA_CHARS: usize = 60_000;

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AssistantMessage {
    /// `"user"` or `"assistant"`.
    pub role: String,
    pub content: String,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AssistantRequest {
    /// Lets `assistant_cancel` stop this request.
    #[serde(default)]
    pub request_id: Option<String>,
    pub engine: String,
    pub server_version: String,
    pub database: String,
    /// One `schema.table(col, col, ...)` per line.
    pub schema_context: String,
    pub messages: Vec<AssistantMessage>,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AssistantReply {
    pub text: String,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

// --- API key ---------------------------------------------------------------------

/// The keychain value, read once per process: `None` = not read yet,
/// `Some(None)` = read and empty. Avoids a keychain prompt per request.
static KEY_CACHE: Mutex<Option<Option<String>>> = Mutex::new(None);

fn env_key() -> Option<String> {
    std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .map(|k| k.trim().to_owned())
        .filter(|k| !k.is_empty())
}

/// The keychain key, falling back to `ANTHROPIC_API_KEY`.
async fn api_key() -> Result<Option<String>> {
    let cached = lock(&KEY_CACHE).clone();
    let stored = match cached {
        Some(value) => value,
        None => match keychain::assistant_key().await {
            Ok(value) => {
                *lock(&KEY_CACHE) = Some(value.clone());
                value
            }
            // An unavailable keychain shouldn't block someone using the env var.
            Err(e) => return env_key().map(Some).ok_or(e),
        },
    };
    Ok(stored.or_else(env_key))
}

/// Whether a key is available. The key itself is never returned.
#[tauri::command]
pub async fn assistant_key_status() -> Result<bool> {
    Ok(api_key().await?.is_some())
}

/// Stores the Anthropic API key in the OS keychain; an empty string deletes it.
#[tauri::command]
pub async fn assistant_key_set(key: String) -> Result<()> {
    let key = key.trim().to_owned();
    keychain::assistant_key_store(&key).await?;
    *lock(&KEY_CACHE) = Some((!key.is_empty()).then_some(key));
    Ok(())
}

// --- cancellation ----------------------------------------------------------------

static PENDING: LazyLock<Mutex<HashMap<String, Arc<Notify>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Registers a request id for the lifetime of the request.
struct Pending {
    id: Option<String>,
    cancel: Arc<Notify>,
}

impl Pending {
    fn new(id: Option<String>) -> Self {
        let cancel = Arc::new(Notify::new());
        if let Some(id) = &id {
            lock(&PENDING).insert(id.clone(), cancel.clone());
        }
        Pending { id, cancel }
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        if let Some(id) = &self.id {
            lock(&PENDING).remove(id);
        }
    }
}

/// Stops a running `assistant_ask`; it then fails with `kind: "cancelled"`.
#[tauri::command]
pub fn assistant_cancel(request_id: String) {
    if let Some(cancel) = lock(&PENDING).get(&request_id) {
        // Stores a permit, so it works even if the request hasn't started waiting yet.
        cancel.notify_one();
    }
}

// --- the request -----------------------------------------------------------------

fn client() -> Result<&'static reqwest::Client> {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    if let Some(client) = CLIENT.get() {
        return Ok(client);
    }
    // reqwest is built without a bundled TLS provider (the updater's setup); it
    // panics unless one is installed first.
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    let built = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| Error::Other(format!("Could not set up the HTTP client: {e}")))?;
    Ok(CLIENT.get_or_init(|| built))
}

fn engine_name(engine: &str) -> &str {
    match engine {
        "postgres" => "PostgreSQL",
        "mysql" => "MySQL",
        "mariadb" => "MariaDB",
        "sqlite" => "SQLite",
        "mongodb" => "MongoDB",
        other => other,
    }
}

fn dialect_hint(engine: &str) -> &'static str {
    match engine {
        "postgres" => "Quote identifiers with double quotes only when needed.",
        "mysql" | "mariadb" => "Quote identifiers with backticks only when needed.",
        "sqlite" => "Quote identifiers with double quotes only when needed.",
        "mongodb" => {
            "Write mongosh statements (db.collection.find(...), aggregate(...)), not SQL, \
             and tag those code blocks ```js."
        }
        _ => "",
    }
}

fn clip_chars(text: &str, max: usize) -> (&str, bool) {
    match text.char_indices().nth(max) {
        Some((end, _)) => (&text[..end], true),
        None => (text, false),
    }
}

/// "PostgreSQL 16.2", "MariaDB 11.8.2-MariaDB" style: the engine name, plus the
/// version unless that already names the engine.
fn server_label(engine: &str, version: &str) -> String {
    let name = engine_name(engine);
    let version = version.trim();
    if version.is_empty() {
        name.to_owned()
    } else if version.to_lowercase().contains(&name.to_lowercase()) {
        version.to_owned()
    } else {
        format!("{name} {version}")
    }
}

fn system_prompt(req: &AssistantRequest) -> String {
    let server = server_label(&req.engine, &req.server_version);
    let database = req.database.trim();
    let (schema, clipped) = clip_chars(req.schema_context.trim(), MAX_SCHEMA_CHARS);
    let schema = if schema.is_empty() {
        "(The schema has not been loaded yet.)".to_owned()
    } else if clipped {
        format!("{schema}\n(... schema truncated; more tables exist)")
    } else {
        schema.to_owned()
    };
    let database = if database.is_empty() {
        String::new()
    } else {
        format!(", database \"{database}\"")
    };
    let hint = dialect_hint(&req.engine);
    format!(
        "You are an expert database assistant built into TableSoft, a desktop database client. \
The user is connected to {server}{database}.\n\
\n\
How to answer:\n\
- Be concise. Lead with the query or the answer, then add a short explanation only if it helps. \
Plain text with short paragraphs; no headings. Use `inline code` for table and column names.\n\
- Put every runnable statement in its own fenced code block tagged ```sql.\n\
- Write for exactly this server and version. {hint}\n\
- Use only the tables and columns in the schema below. Never invent tables, columns or functions. \
If something you need is not in the schema, say so and ask rather than guessing.\n\
- Prefer read-only queries. Write INSERT, UPDATE, DELETE or DDL only when the user explicitly asks \
to change data or structure, and then say what the statement will change; never write an UPDATE or \
DELETE without a WHERE clause unless asked to affect every row.\n\
- Add a LIMIT to queries that could return a lot of rows.\n\
\n\
Schema (one table per line, as schema.table(columns)):\n\
{schema}"
    )
}

fn request_body(req: &AssistantRequest) -> Result<Value> {
    let messages: Vec<Value> = req
        .messages
        .iter()
        .filter(|m| !m.content.trim().is_empty())
        .map(|m| {
            let role = match m.role.as_str() {
                "user" | "assistant" => m.role.as_str(),
                other => {
                    return Err(Error::InvalidInput(format!(
                        "Unknown message role: {other}"
                    )));
                }
            };
            Ok(json!({ "role": role, "content": m.content }))
        })
        .collect::<Result<_>>()?;
    if messages.last().and_then(|m| m["role"].as_str()) != Some("user") {
        return Err(Error::InvalidInput("Ask a question first.".into()));
    }
    let model = req
        .model
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .unwrap_or(DEFAULT_MODEL);
    Ok(json!({
        "model": model,
        "max_tokens": MAX_TOKENS,
        "system": system_prompt(req),
        "messages": messages,
    }))
}

/// The API's own explanation from an error body, if it sent one.
fn api_message(body: &str) -> Option<String> {
    let parsed: Value = serde_json::from_str(body).ok()?;
    let message = parsed["error"]["message"].as_str()?.trim();
    (!message.is_empty()).then(|| message.to_owned())
}

fn http_error(status: u16, body: &str) -> Error {
    let detail = api_message(body);
    let with_detail = |lead: &str| match &detail {
        Some(d) => format!("{lead} {d}"),
        None => lead.to_owned(),
    };
    Error::Other(match status {
        401 => "Invalid API key. Check it in Settings.".to_owned(),
        403 => with_detail("This API key is not allowed to do that."),
        404 => with_detail("Model not found. Pick another one in Settings."),
        413 => {
            "The conversation and schema are too large for the model. Start a new chat.".to_owned()
        }
        429 => "Rate limited by Anthropic. Wait a moment and try again.".to_owned(),
        529 => "Anthropic's API is overloaded. Try again shortly.".to_owned(),
        500..=599 => format!("Anthropic's API had a server error (HTTP {status}). Try again."),
        400 => with_detail("Anthropic rejected the request."),
        _ => with_detail(&format!("Anthropic API error (HTTP {status}).")),
    })
}

fn network_error(e: &reqwest::Error) -> Error {
    Error::Other(if e.is_timeout() {
        format!(
            "The assistant took longer than {} s to answer. Try again.",
            TIMEOUT.as_secs()
        )
    } else if e.is_connect() {
        "Could not reach api.anthropic.com. Check your internet connection.".to_owned()
    } else {
        format!("Network error talking to Anthropic: {e}")
    })
}

fn parse_reply(body: &str) -> Result<AssistantReply> {
    let parsed: Value = serde_json::from_str(body)
        .map_err(|_| Error::Other("Anthropic sent a reply that is not valid JSON.".into()))?;
    let mut text = String::new();
    for block in parsed["content"].as_array().into_iter().flatten() {
        if block["type"] == "text"
            && let Some(t) = block["text"].as_str()
        {
            text.push_str(t);
        }
    }
    if text.trim().is_empty() {
        return Err(Error::Other("The assistant sent an empty reply.".into()));
    }
    if parsed["stop_reason"] == "max_tokens" {
        text.push_str("\n\n(The reply was cut off because it hit the length limit.)");
    }
    Ok(AssistantReply { text })
}

async fn send(key: String, body: Value) -> Result<AssistantReply> {
    let response = client()?
        .post(API_URL)
        .header("x-api-key", key)
        .header("anthropic-version", API_VERSION)
        .timeout(TIMEOUT)
        .json(&body)
        .send()
        .await
        .map_err(|e| network_error(&e))?;
    let status = response.status();
    let text = response.text().await.map_err(|e| network_error(&e))?;
    if status.is_success() {
        parse_reply(&text)
    } else {
        Err(http_error(status.as_u16(), &text))
    }
}

/// Sends the conversation to the model and returns its reply.
#[tauri::command]
pub async fn assistant_ask(req: AssistantRequest) -> Result<AssistantReply> {
    let body = request_body(&req)?;
    let key = api_key().await?.ok_or_else(|| {
        Error::InvalidInput("Add your Anthropic API key in Settings to use the assistant.".into())
    })?;
    let pending = Pending::new(req.request_id.clone());
    tokio::select! {
        _ = pending.cancel.notified() => Err(Error::Cancelled),
        reply = send(key, body) => reply,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(messages: &[(&str, &str)]) -> AssistantRequest {
        AssistantRequest {
            request_id: None,
            engine: "postgres".into(),
            server_version: "PostgreSQL 16.2".into(),
            database: "shop".into(),
            schema_context: "public.users(id, email)\npublic.orders(id, user_id)".into(),
            messages: messages
                .iter()
                .map(|(role, content)| AssistantMessage {
                    role: (*role).into(),
                    content: (*content).into(),
                })
                .collect(),
            model: None,
        }
    }

    #[test]
    fn prompt_names_the_server_and_lists_the_schema() {
        let prompt = system_prompt(&request(&[("user", "hi")]));
        assert!(prompt.contains("connected to PostgreSQL 16.2, database \"shop\""));
        assert!(prompt.contains("database \"shop\""));
        assert!(prompt.contains("public.users(id, email)"));
        assert!(prompt.contains("```sql"));
    }

    #[test]
    fn http_client_builds_with_the_ring_provider() {
        // Building panics without an installed rustls crypto provider.
        assert!(client().is_ok());
    }

    #[test]
    fn server_label_does_not_repeat_the_engine() {
        assert_eq!(
            server_label("postgres", "PostgreSQL 16.2"),
            "PostgreSQL 16.2"
        );
        assert_eq!(server_label("mysql", "8.4.3"), "MySQL 8.4.3");
        assert_eq!(server_label("mariadb", "11.8.2-MariaDB"), "11.8.2-MariaDB");
        assert_eq!(server_label("mongodb", ""), "MongoDB");
    }

    #[test]
    fn body_uses_the_default_model_and_validates_the_conversation() {
        let body = request_body(&request(&[
            ("user", "count users"),
            ("assistant", "ok"),
            ("user", "more"),
        ]))
        .unwrap();
        assert_eq!(body["model"], DEFAULT_MODEL);
        assert_eq!(body["max_tokens"], MAX_TOKENS);
        assert_eq!(body["messages"].as_array().unwrap().len(), 3);

        assert!(request_body(&request(&[])).is_err());
        assert!(request_body(&request(&[("user", "x"), ("assistant", "y")])).is_err());
        assert!(request_body(&request(&[("system", "x")])).is_err());

        let mut custom = request(&[("user", "x")]);
        custom.model = Some("claude-opus-5-5".into());
        assert_eq!(request_body(&custom).unwrap()["model"], "claude-opus-5-5");
    }

    #[test]
    fn long_schemas_are_cut_and_flagged() {
        let mut req = request(&[("user", "x")]);
        req.schema_context = "t(a)\n".repeat(MAX_SCHEMA_CHARS);
        let prompt = system_prompt(&req);
        assert!(prompt.contains("schema truncated"));
        assert!(prompt.len() < MAX_SCHEMA_CHARS + 2_000);
    }

    #[test]
    fn http_errors_are_readable() {
        let msg = |s, b| http_error(s, b).to_string();
        assert!(msg(401, "{}").starts_with("Invalid API key"));
        assert!(msg(429, "{}").starts_with("Rate limited"));
        assert!(msg(503, "").contains("HTTP 503"));
        assert!(
            msg(400, r#"{"error":{"type":"invalid_request_error","message":"credit balance is too low"}}"#)
                .contains("credit balance is too low")
        );
        assert!(msg(418, "not json").contains("HTTP 418"));
    }

    #[test]
    fn replies_join_text_blocks() {
        let body = r#"{"content":[{"type":"text","text":"Hello "},{"type":"tool_use","id":"x"},{"type":"text","text":"world"}],"stop_reason":"end_turn"}"#;
        assert_eq!(parse_reply(body).unwrap().text, "Hello world");
        assert!(parse_reply(r#"{"content":[]}"#).is_err());
        let cut = r#"{"content":[{"type":"text","text":"abc"}],"stop_reason":"max_tokens"}"#;
        assert!(parse_reply(cut).unwrap().text.contains("cut off"));
    }
}
