//! An in-process MCP server over HTTP.
//!
//! This is how the conductor gets hands: Divixi registers its app API as
//! MCP tools, listens on a random localhost port with a random bearer
//! token, and hands the URL to the conductor's ACP session in
//! `session/new` → `mcpServers`. Tool calls land directly in this process,
//! so `spawn_worker` can open a worker and wait for its report without any
//! extra process or IPC.
//!
//! Only the Streamable HTTP transport's request/response half is
//! implemented: JSON-RPC over `POST`, JSON back. That is enough for tools;
//! server-initiated messages (the `GET` stream) are not needed here.

use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// MCP protocol revision this server speaks.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

type ToolFuture = Pin<Box<dyn Future<Output = Result<Value, String>> + Send>>;
type ToolHandler = Arc<dyn Fn(Value) -> ToolFuture + Send + Sync>;

/// One tool the agent can call.
#[derive(Clone)]
pub struct Tool {
    pub name: String,
    pub description: String,
    /// JSON Schema for the arguments object.
    pub input_schema: Value,
    handler: ToolHandler,
}

impl Tool {
    /// Build a tool from an async closure. `Ok(value)` is returned to the
    /// agent as text (strings verbatim, other values as JSON); `Err(msg)`
    /// becomes an `isError` result the agent can read and recover from.
    pub fn new<F, Fut>(name: &str, description: &str, input_schema: Value, f: F) -> Self
    where
        F: Fn(Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Value, String>> + Send + 'static,
    {
        Self {
            name: name.to_string(),
            description: description.to_string(),
            input_schema,
            handler: Arc::new(move |args| Box::pin(f(args))),
        }
    }
}

/// The running server.
pub struct McpServer {
    addr: SocketAddr,
    token: String,
    task: tokio::task::JoinHandle<()>,
}

struct Shared {
    name: String,
    token: String,
    tools: HashMap<String, Tool>,
    order: Vec<String>,
}

impl McpServer {
    /// Bind `127.0.0.1:0` and serve `tools`. `name` is what the agent sees
    /// as the server's name.
    pub async fn start(name: &str, tools: Vec<Tool>) -> anyhow::Result<Self> {
        let token = random_token();
        let order: Vec<String> = tools.iter().map(|t| t.name.clone()).collect();
        let shared = Arc::new(Shared {
            name: name.to_string(),
            token: token.clone(),
            tools: tools.into_iter().map(|t| (t.name.clone(), t)).collect(),
            order,
        });

        let app = Router::new()
            .route("/mcp", post(handle_post).get(handle_get).delete(handle_delete))
            .with_state(shared);

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
        let addr = listener.local_addr()?;
        let task = tokio::spawn(async move {
            if let Err(err) = axum::serve(listener, app).await {
                tracing::error!(%err, "mcp server stopped");
            }
        });
        tracing::info!(%addr, "mcp server listening");
        Ok(Self { addr, token, task })
    }

    /// The endpoint URL to hand to agents.
    pub fn url(&self) -> String {
        format!("http://{}/mcp", self.addr)
    }

    /// The `Authorization` header value agents must send.
    pub fn auth_header(&self) -> (String, String) {
        ("Authorization".to_string(), format!("Bearer {}", self.token))
    }
}

impl Drop for McpServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn random_token() -> String {
    use rand::Rng;
    let mut bytes = [0u8; 24];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Deserialize)]
struct Request {
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Serialize)]
struct RpcError {
    code: i64,
    message: String,
}

fn ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn err(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": RpcError { code, message: message.into() } })
}

fn authorized(shared: &Shared, headers: &HeaderMap) -> bool {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.strip_prefix("Bearer ").map(|t| t == shared.token).unwrap_or(false))
        .unwrap_or(false)
}

async fn handle_get() -> Response {
    // No server-initiated stream. Clients treat 405 as "not supported".
    StatusCode::METHOD_NOT_ALLOWED.into_response()
}

async fn handle_delete() -> Response {
    StatusCode::NO_CONTENT.into_response()
}

async fn handle_post(State(shared): State<Arc<Shared>>, headers: HeaderMap, body: Bytes) -> Response {
    if !authorized(&shared, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let parsed: Result<Value, _> = serde_json::from_slice(&body);
    let Ok(value) = parsed else {
        return json_response(err(Value::Null, -32700, "parse error"));
    };

    // A batch is answered as a batch; a lone notification with 202.
    match value {
        Value::Array(items) => {
            let mut out = Vec::new();
            for item in items {
                if let Some(resp) = dispatch(&shared, item).await {
                    out.push(resp);
                }
            }
            if out.is_empty() {
                StatusCode::ACCEPTED.into_response()
            } else {
                json_response(Value::Array(out))
            }
        }
        item => match dispatch(&shared, item).await {
            Some(resp) => json_response(resp),
            None => StatusCode::ACCEPTED.into_response(),
        },
    }
}

fn json_response(value: Value) -> Response {
    ([(header::CONTENT_TYPE, "application/json")], value.to_string()).into_response()
}

/// Handle one JSON-RPC message. Notifications return `None`.
async fn dispatch(shared: &Shared, value: Value) -> Option<Value> {
    let req: Request = match serde_json::from_value(value) {
        Ok(r) => r,
        Err(e) => return Some(err(Value::Null, -32600, format!("invalid request: {e}"))),
    };
    let id = req.id.clone();
    let is_notification = id.is_none() || req.method.starts_with("notifications/");
    let id = id.unwrap_or(Value::Null);

    let result: Result<Value, (i64, String)> = match req.method.as_str() {
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": shared.name, "version": env!("CARGO_PKG_VERSION") },
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({
            "tools": shared.order.iter().filter_map(|n| shared.tools.get(n)).map(|t| json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": t.input_schema,
            })).collect::<Vec<_>>()
        })),
        "tools/call" => {
            let name = req.params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = req.params.get("arguments").cloned().unwrap_or(json!({}));
            match shared.tools.get(name) {
                None => Err((-32602, format!("unknown tool {name}"))),
                Some(tool) => {
                    tracing::info!(tool = name, "mcp tool call");
                    let outcome = (tool.handler)(args).await;
                    Ok(match outcome {
                        Ok(v) => json!({ "content": [{ "type": "text", "text": as_text(&v) }], "isError": false }),
                        Err(msg) => json!({ "content": [{ "type": "text", "text": msg }], "isError": true }),
                    })
                }
            }
        }
        m if m.starts_with("notifications/") => return None,
        other => Err((-32601, format!("method not found: {other}"))),
    };

    if is_notification {
        return None;
    }
    Some(match result {
        Ok(v) => ok(id, v),
        Err((code, msg)) => err(id, code, msg),
    })
}

fn as_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn call(server: &McpServer, body: Value) -> (StatusCode, Value) {
        let client = reqwest_lite::post(&server.url(), &server.auth_header(), body.to_string()).await;
        client
    }

    /// A tiny HTTP client on raw tokio so tests do not need reqwest here.
    mod reqwest_lite {
        use super::*;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        pub async fn post(url: &str, auth: &(String, String), body: String) -> (StatusCode, Value) {
            let rest = url.strip_prefix("http://").unwrap();
            let (host, path) = rest.split_once('/').unwrap();
            let mut stream = tokio::net::TcpStream::connect(host).await.unwrap();
            let req = format!(
                "POST /{path} HTTP/1.1\r\nHost: {host}\r\n{}: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                auth.0,
                auth.1,
                body.len()
            );
            stream.write_all(req.as_bytes()).await.unwrap();
            let mut buf = Vec::new();
            stream.read_to_end(&mut buf).await.unwrap();
            let text = String::from_utf8_lossy(&buf);
            let status: u16 = text.split_whitespace().nth(1).unwrap().parse().unwrap();
            let body = text.split("\r\n\r\n").nth(1).unwrap_or("");
            let value = serde_json::from_str(body).unwrap_or(Value::Null);
            (StatusCode::from_u16(status).unwrap(), value)
        }
    }

    #[tokio::test]
    async fn lists_and_calls_tools() {
        let tool = Tool::new(
            "echo",
            "Echo the input",
            json!({ "type": "object", "properties": { "text": { "type": "string" } }, "required": ["text"] }),
            |args| async move {
                let text = args.get("text").and_then(Value::as_str).unwrap_or("").to_string();
                Ok(Value::String(format!("echo: {text}")))
            },
        );
        let server = McpServer::start("orchestra", vec![tool]).await.unwrap();

        let (st, init) = call(&server, json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}})).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(init["result"]["serverInfo"]["name"], "orchestra");

        let (st, list) = call(&server, json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(list["result"]["tools"][0]["name"], "echo");

        let (_, resp) = call(
            &server,
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"echo","arguments":{"text":"hi"}}}),
        )
        .await;
        assert_eq!(resp["result"]["content"][0]["text"], "echo: hi");
        assert_eq!(resp["result"]["isError"], false);

        let (_, missing) = call(&server, json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"nope"}})).await;
        assert_eq!(missing["error"]["code"], -32602);

        let (st, _) = call(&server, json!({"jsonrpc":"2.0","method":"notifications/initialized"})).await;
        assert_eq!(st, StatusCode::ACCEPTED);
    }

    #[tokio::test]
    async fn rejects_without_token() {
        let server = McpServer::start("orchestra", vec![]).await.unwrap();
        let bad = ("Authorization".to_string(), "Bearer wrong".to_string());
        let (st, _) = reqwest_lite::post(&server.url(), &bad, "{}".to_string()).await;
        assert_eq!(st, StatusCode::UNAUTHORIZED);
    }
}
