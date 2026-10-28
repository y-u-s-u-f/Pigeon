//! The MCP server's transport: Streamable HTTP on 127.0.0.1, one `POST /mcp`
//! answered with `application/json` (no SSE stream; GET is 405), and the
//! JSON-RPC on top of it. Runs on its own threads; tool calls go to the
//! main thread through `Host`.
//!
//! Dual-era: a request carrying `io.modelcontextprotocol/protocolVersion` in
//! its `_meta` (or a non-legacy `MCP-Protocol-Version` header) is served
//! statelessly as 2026-07-28; `initialize` opens a legacy session.

use std::{
    collections::VecDeque,
    io::{self, Read as _, Write as _},
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

use serde_json::{Value, json};

use super::{McpPrefs, tools};

/// Every version we speak, newest first, for `server/discover` and
/// `UnsupportedProtocolVersionError`.
pub const PROTOCOL_VERSIONS: [&str; 4] = ["2026-07-28", "2025-11-25", "2025-06-18", "2025-03-26"];
/// Per-request `_meta`, no handshake.
const MODERN_VERSIONS: [&str; 1] = ["2026-07-28"];
/// Negotiated by `initialize`. Newest first; the first is offered when a
/// client asks for one we don't know.
const LEGACY_VERSIONS: [&str; 3] = ["2025-11-25", "2025-06-18", "2025-03-26"];
const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const META_CLIENT: &str = "io.modelcontextprotocol/clientInfo";
const META_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
const META_SERVER: &str = "io.modelcontextprotocol/serverInfo";
const HEADER_MISMATCH: i64 = -32020;
const UNSUPPORTED_VERSION: i64 = -32022;
/// `tools/list` and `server/discover` only change with a new build.
const LIST_TTL_MS: u64 = 60 * 60 * 1000;
const INSTRUCTIONS: &str = "Pigeon is the user's Gmail and Google Calendar client. \
    Conversation ids come from search_mail; event ids from list_events. \
    Times are the user's local wall-clock time (ISO 8601, no offset). \
    Each tool needs its switch in Pigeon's Settings › MCP.";
pub const PATH: &str = "/mcp";
const MAX_HEAD: usize = 16 * 1024;
const MAX_BODY: usize = 1024 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(10);
/// A whole request arrives within this, however slowly it trickles.
const READ_DEADLINE: Duration = Duration::from_secs(30);
const MAX_CONNECTIONS: usize = 8;
const ACTIVITY_MAX: usize = 50;
const SESSIONS_MAX: usize = 32;
/// Ports tried after the preferred one before any free port.
const PORT_TRIES: u16 = 10;

/// One tool call, for Settings › MCP › Activity.
#[derive(Clone, Debug)]
pub struct Activity {
    pub at: chrono::DateTime<chrono::Local>,
    pub client: String,
    pub tool: String,
    pub ok: bool,
}

/// What the server threads and the app share.
#[derive(Default)]
pub struct Shared {
    pub token: String,
    pub prefs: McpPrefs,
    pub port: u16,
    /// Session id and the client's name from `initialize`, oldest first.
    sessions: VecDeque<(String, String)>,
    /// Newest first.
    pub activity: VecDeque<Activity>,
}

impl Shared {
    pub fn new(token: String, prefs: McpPrefs) -> Self {
        Self {
            token,
            prefs,
            ..Default::default()
        }
    }

    fn client(&self, session: Option<&str>) -> Option<&str> {
        let session = session?;
        self.sessions
            .iter()
            .find(|(id, _)| id == session)
            .map(|(_, name)| name.as_str())
    }

    fn log(&mut self, client: String, tool: &str, ok: bool) {
        self.activity.push_front(Activity {
            at: chrono::Local::now(),
            client,
            tool: tool.to_string(),
            ok,
        });
        self.activity.truncate(ACTIVITY_MAX);
    }
}

/// Where tool calls run: the app's main thread.
pub trait Host: Send + Sync {
    /// The tool's result as JSON, or why it failed, in plain words.
    fn call(&self, tool: &str, args: Value) -> Result<Value, String>;
    /// The activity log changed.
    fn changed(&self) {}
}

pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(&'static str, String)>,
    pub body: Vec<u8>,
}

impl Response {
    fn empty(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    fn json(status: u16, value: &Value) -> Self {
        Self {
            status,
            headers: vec![("Content-Type", "application/json".into())],
            body: value.to_string().into_bytes(),
        }
    }

    /// A JSON-RPC error with no id, for a refused HTTP request.
    fn refused(status: u16, message: &str) -> Self {
        Self::json(status, &error(Value::Null, -32600, message))
    }
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn result(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

/// An `Origin` a page on this Mac sends: http(s)://localhost or 127.0.0.1,
/// any port. A web page elsewhere (DNS rebinding) sends its own.
fn local_origin(origin: &str) -> bool {
    let Some(rest) = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
    else {
        return false;
    };
    let (host, port) = rest.split_once(':').unwrap_or((rest, ""));
    matches!(host, "127.0.0.1" | "localhost") && port.chars().all(|c| c.is_ascii_digit())
}

/// The `Host` header names this server, not some other name that resolves
/// here (DNS rebinding).
fn local_host(host: &str, port: u16) -> bool {
    host == format!("127.0.0.1:{port}") || host == format!("localhost:{port}")
}

/// Without leaking how much of the token matched.
fn same_token(given: &str, token: &str) -> bool {
    !token.is_empty()
        && given.len() == token.len()
        && given
            .bytes()
            .zip(token.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

/// Answer one HTTP request.
pub fn handle(request: &Request, shared: &Mutex<Shared>, host: &dyn Host) -> Response {
    let (token, port) = {
        let s = shared.lock().unwrap_or_else(|e| e.into_inner());
        (s.token.clone(), s.port)
    };
    if request.path.split('?').next() != Some(PATH) {
        return Response::empty(404);
    }
    if let Some(origin) = request.header("origin")
        && !local_origin(origin)
    {
        return Response::refused(403, "Origin not allowed");
    }
    if !request.header("host").is_some_and(|h| local_host(h, port)) {
        return Response::refused(403, "Host not allowed");
    }
    let bearer = request.header("authorization").and_then(|a| {
        a.strip_prefix("Bearer ")
            .or_else(|| a.strip_prefix("bearer "))
    });
    if !bearer.is_some_and(|b| same_token(b.trim(), &token)) {
        let mut response = Response::refused(401, "Missing or wrong access token");
        response
            .headers
            .push(("WWW-Authenticate", "Bearer realm=\"Pigeon\"".into()));
        return response;
    }
    let session = request.header("mcp-session-id").map(str::to_string);
    let known_session = |id: &str| {
        shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .client(Some(id))
            .is_some()
    };
    match request.method.as_str() {
        "POST" => {}
        "DELETE" => {
            if let Some(id) = &session {
                if !known_session(id) {
                    return Response::refused(404, "Session not found; initialize again");
                }
                shared
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .sessions
                    .retain(|(s, _)| s != id);
            }
            return Response::empty(200);
        }
        _ => {
            let mut response = Response::empty(405);
            response.headers.push(("Allow", "POST, DELETE".into()));
            return response;
        }
    }
    let Ok(message) = serde_json::from_slice::<Value>(&request.body) else {
        return Response::json(400, &error(Value::Null, -32700, "Parse error"));
    };
    if !message.is_object() {
        return Response::json(
            400,
            &error(Value::Null, -32600, "Send one JSON-RPC message"),
        );
    }
    let method = message.get("method").and_then(Value::as_str);
    if method.is_some() && message.get("id").is_some_and(Value::is_null) {
        // A null id is not a notification; only a missing one is.
        return Response::json(
            400,
            &error(Value::Null, -32600, "Invalid Request: id must not be null"),
        );
    }
    let params = message.get("params").cloned().unwrap_or(json!({}));
    let header_version = request.header("mcp-protocol-version");
    let modern = method != Some("initialize")
        && (meta(&params, META_VERSION).is_some()
            || method == Some("server/discover")
            || header_version.is_some_and(|v| !LEGACY_VERSIONS.contains(&v)));
    if modern {
        return serve_modern(request, &message, &params, shared, host);
    }
    // Legacy: the header is absent (2025-03-26) or a legacy version, and
    // `initialize` negotiates in its body whatever the header says.
    if let Some(id) = &session
        && !known_session(id)
    {
        return Response::refused(404, "Session not found; initialize again");
    }
    let Some(method) = method else {
        // A response to a request of ours; we send none.
        return Response::empty(202);
    };
    let Some(id) = message.get("id").cloned().filter(|id| !id.is_null()) else {
        // notifications/initialized, cancelled and the like.
        return Response::empty(202);
    };
    match method {
        "initialize" => initialize(id, &params, shared),
        "ping" => Response::json(200, &result(id, json!({}))),
        "tools/list" => Response::json(200, &result(id, json!({"tools": tools::list()}))),
        "tools/call" => {
            let client = shared
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .client(session.as_deref())
                .unwrap_or("Unknown agent")
                .to_string();
            let response = match call_tool(&params, client, shared, host) {
                Ok(value) => Response::json(200, &result(id, value)),
                Err((code, message)) => Response::json(200, &error(id, code, &message)),
            };
            host.changed();
            response
        }
        _ => Response::json(200, &error(id, -32601, "Method not found")),
    }
}

/// A reserved `io.modelcontextprotocol/*` key of the request's `_meta`.
fn meta<'a>(params: &'a Value, key: &str) -> Option<&'a Value> {
    params.get("_meta")?.get(key)
}

/// A client's self-reported name, safe for the activity log.
fn client_name(info: Option<&Value>) -> String {
    info.and_then(|i| i.get("name"))
        .and_then(Value::as_str)
        .filter(|n| !n.trim().is_empty())
        .unwrap_or("Unknown agent")
        .chars()
        .filter(|c| !c.is_control())
        .take(60)
        .collect()
}

/// `Mcp-Name` and `Mcp-Param-*` values may come as `=?base64?…?=`.
fn header_value(value: &str) -> Option<String> {
    use base64::Engine as _;
    match value
        .strip_prefix("=?base64?")
        .and_then(|v| v.strip_suffix("?="))
    {
        Some(encoded) => base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok()),
        None => Some(value.to_string()),
    }
}

/// A 2026-07-28 request: stateless, its version, client and capabilities in
/// `_meta`, mirrored into headers that must match the body.
fn serve_modern(
    request: &Request,
    message: &Value,
    params: &Value,
    shared: &Mutex<Shared>,
    host: &dyn Host,
) -> Response {
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        return Response::empty(202);
    };
    let Some(id) = message.get("id").cloned().filter(|id| !id.is_null()) else {
        // 2026-07-28 defines no client notifications over HTTP.
        return Response::empty(202);
    };
    let bad = |code: i64, message: &str| Response::json(400, &error(id.clone(), code, message));
    let header = request.header("mcp-protocol-version");
    let body = meta(params, META_VERSION).map(|v| v.as_str());
    if let (Some(header), Some(body)) = (header, body)
        && Some(header) != body
    {
        return bad(
            HEADER_MISMATCH,
            "Header mismatch: MCP-Protocol-Version doesn't match _meta",
        );
    }
    if let Some(requested) = body.flatten().or(header)
        && !MODERN_VERSIONS.contains(&requested)
    {
        let message = if LEGACY_VERSIONS.contains(&requested) {
            "Unsupported protocol version; 2025-11-25 and earlier start with initialize"
        } else {
            "Unsupported protocol version"
        };
        return Response::json(
            400,
            &json!({"jsonrpc": "2.0", "id": id, "error": {
                "code": UNSUPPORTED_VERSION,
                "message": message,
                "data": {"supported": PROTOCOL_VERSIONS, "requested": requested},
            }}),
        );
    }
    let Some(Some(version)) = body else {
        return bad(-32602, &format!("Missing _meta “{META_VERSION}”"));
    };
    if header.is_none() {
        return bad(HEADER_MISMATCH, "Missing MCP-Protocol-Version header");
    }
    if !meta(params, META_CAPABILITIES).is_some_and(Value::is_object) {
        return bad(-32602, &format!("Missing _meta “{META_CAPABILITIES}”"));
    }
    if request.header("mcp-method") != Some(method) {
        return bad(
            HEADER_MISMATCH,
            "Header mismatch: Mcp-Method missing or doesn't match method",
        );
    }
    if method == "tools/call" {
        let name = params.get("name").and_then(Value::as_str);
        let header = request.header("mcp-name").and_then(header_value);
        if header.is_none() || header.as_deref() != name {
            return bad(
                HEADER_MISMATCH,
                "Header mismatch: Mcp-Name missing or doesn't match params.name",
            );
        }
    }
    let complete = |mut value: Value| {
        value["resultType"] = json!("complete");
        value["_meta"] = json!({META_SERVER: server_info()});
        Response::json(200, &result(id.clone(), value))
    };
    match method {
        "server/discover" => complete(json!({
            "supportedVersions": PROTOCOL_VERSIONS,
            "capabilities": {"tools": {"listChanged": false}},
            "instructions": INSTRUCTIONS,
            "ttlMs": LIST_TTL_MS,
            "cacheScope": "public",
        })),
        "tools/list" => complete(json!({
            "tools": tools::list(),
            "ttlMs": LIST_TTL_MS,
            "cacheScope": "public",
        })),
        "tools/call" => {
            let client = client_name(meta(params, META_CLIENT));
            let response = match call_tool(params, client, shared, host) {
                Ok(value) => complete(value),
                Err((code, message)) => Response::json(200, &error(id.clone(), code, &message)),
            };
            host.changed();
            response
        }
        _ => Response::json(
            404,
            &error(
                id.clone(),
                -32601,
                &format!("Method not found in {version}"),
            ),
        ),
    }
}

fn server_info() -> Value {
    json!({"name": "Pigeon", "version": env!("CARGO_PKG_VERSION")})
}

fn initialize(id: Value, params: &Value, shared: &Mutex<Shared>) -> Response {
    let asked = params.get("protocolVersion").and_then(Value::as_str);
    let version = asked
        .filter(|v| LEGACY_VERSIONS.contains(v))
        .unwrap_or(LEGACY_VERSIONS[0]);
    let client = client_name(params.get("clientInfo"));
    let session = crate::backend::auth::random_token(24);
    {
        let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
        if s.sessions.len() >= SESSIONS_MAX {
            s.sessions.pop_front();
        }
        s.sessions.push_back((session.clone(), client));
    }
    let mut response = Response::json(
        200,
        &result(
            id,
            json!({
                "protocolVersion": version,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": server_info(),
                "instructions": INSTRUCTIONS,
            }),
        ),
    );
    response.headers.push(("Mcp-Session-Id", session));
    response
}

/// The `tools/call` result, or a JSON-RPC error code and message.
fn call_tool(
    params: &Value,
    client: String,
    shared: &Mutex<Shared>,
    host: &dyn Host,
) -> Result<Value, (i64, String)> {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let Some(tool) = tools::find(name) else {
        return Err((-32602, format!("Unknown tool: {name}")));
    };
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    let allowed = {
        let s = shared.lock().unwrap_or_else(|e| e.into_inner());
        s.prefs.enabled && s.prefs.allows(tool.capability)
    };
    let outcome = if !allowed {
        Err(format!(
            "{} is off. Ask the user to turn on “{}” in Pigeon's Settings › MCP.",
            tool.capability.title(),
            tool.capability.title()
        ))
    } else if !args.is_object() {
        Err("arguments must be an object".into())
    } else {
        host.call(name, args)
    };
    crate::logging::info(&format!(
        "mcp: {client} called {name}: {}",
        if outcome.is_ok() { "ok" } else { "error" }
    ));
    shared
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .log(client, name, outcome.is_ok());
    let (text, is_error) = match outcome {
        Ok(Value::String(text)) => (text, false),
        Ok(value) => (value.to_string(), false),
        Err(message) => (message, true),
    };
    Ok(json!({"content": [{"type": "text", "text": text}], "isError": is_error}))
}

// The socket.

/// The listening server; dropping it stops it.
pub struct Server {
    port: u16,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    /// Listen on 127.0.0.1: `preferred`, the next few ports if it's taken,
    /// else any free one.
    pub fn start(
        shared: Arc<Mutex<Shared>>,
        host: Arc<dyn Host>,
        preferred: u16,
    ) -> io::Result<Self> {
        let listener = (0..PORT_TRIES)
            .filter_map(|n| preferred.checked_add(n))
            .chain([0])
            .find_map(|port| TcpListener::bind((Ipv4Addr::LOCALHOST, port)).ok())
            .ok_or_else(|| io::Error::other("no free port"))?;
        let port = listener.local_addr()?.port();
        shared.lock().unwrap_or_else(|e| e.into_inner()).port = port;
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = std::thread::Builder::new()
            .name("mcp-server".into())
            .spawn(move || accept(listener, shared, host, stopping))?;
        Ok(Self {
            port,
            stop,
            thread: Some(thread),
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the accept.
        let _ = TcpStream::connect_timeout(
            &SocketAddr::from((Ipv4Addr::LOCALHOST, self.port)),
            Duration::from_millis(200),
        );
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn accept(
    listener: TcpListener,
    shared: Arc<Mutex<Shared>>,
    host: Arc<dyn Host>,
    stop: Arc<AtomicBool>,
) {
    let open = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming() {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let Ok(mut stream) = stream else { continue };
        if open.load(Ordering::SeqCst) >= MAX_CONNECTIONS {
            let _ = write_response(&mut stream, &Response::empty(503));
            continue;
        }
        let slot = Slot::take(&open);
        let (shared, host) = (shared.clone(), host.clone());
        // A thread that doesn't start drops its closure, and the slot with it.
        let spawned = std::thread::Builder::new()
            .name("mcp-connection".into())
            .spawn(move || {
                let _slot = slot;
                serve(stream, &shared, &*host);
            });
        if spawned.is_err() {
            crate::logging::info("mcp: couldn't start a connection thread");
        }
    }
}

/// One of `MAX_CONNECTIONS`, given back when dropped, a panic included.
struct Slot(Arc<AtomicUsize>);

impl Slot {
    fn take(open: &Arc<AtomicUsize>) -> Self {
        open.fetch_add(1, Ordering::SeqCst);
        Self(open.clone())
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// One request per connection, then close.
fn serve(mut stream: TcpStream, shared: &Mutex<Shared>, host: &dyn Host) {
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    let response = match read_request(&mut stream) {
        Ok(request) => handle(&request, shared, host),
        Err(status) => Response::empty(status),
    };
    let _ = write_response(&mut stream, &response);
}

/// The request, or the status to refuse it with.
fn read_request(stream: &mut TcpStream) -> Result<Request, u16> {
    let started = std::time::Instant::now();
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let read = |stream: &mut TcpStream, chunk: &mut [u8]| {
        if started.elapsed() > READ_DEADLINE {
            return Err(408u16);
        }
        stream.read(chunk).map_err(|_| 408u16)
    };
    let (head_len, method, path, headers) = loop {
        let n = read(stream, &mut chunk)?;
        if n == 0 {
            return Err(400);
        }
        buf.extend_from_slice(&chunk[..n]);
        let mut slots = [httparse::EMPTY_HEADER; 64];
        let mut parsed = httparse::Request::new(&mut slots);
        match parsed.parse(&buf) {
            Ok(httparse::Status::Complete(len)) => {
                let headers = parsed
                    .headers
                    .iter()
                    .map(|h| {
                        (
                            h.name.to_string(),
                            String::from_utf8_lossy(h.value).trim().to_string(),
                        )
                    })
                    .collect::<Vec<_>>();
                break (
                    len,
                    parsed.method.unwrap_or("").to_string(),
                    parsed.path.unwrap_or("").to_string(),
                    headers,
                );
            }
            Ok(httparse::Status::Partial) if buf.len() > MAX_HEAD => return Err(431),
            Ok(httparse::Status::Partial) => {}
            Err(_) => return Err(400),
        }
    };
    let header = |name: &str| {
        headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    };
    if header("transfer-encoding").is_some() {
        return Err(411);
    }
    let length = match header("content-length") {
        Some(v) => v.parse::<usize>().map_err(|_| 400u16)?,
        None => 0,
    };
    if length > MAX_BODY {
        return Err(413);
    }
    let mut body = buf.split_off(head_len);
    while body.len() < length {
        let n = read(stream, &mut chunk)?;
        if n == 0 {
            return Err(400);
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(length);
    Ok(Request {
        method,
        path,
        headers,
        body,
    })
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        411 => "Length Required",
        413 => "Payload Too Large",
        431 => "Request Header Fields Too Large",
        503 => "Service Unavailable",
        _ => "",
    }
}

fn write_response(stream: &mut TcpStream, response: &Response) -> io::Result<()> {
    let mut out = format!(
        "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n",
        response.status,
        reason(response.status),
        response.body.len()
    );
    for (name, value) in &response.headers {
        out.push_str(&format!("{name}: {value}\r\n"));
    }
    out.push_str("\r\n");
    stream.write_all(out.as_bytes())?;
    stream.write_all(&response.body)?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::Capability;

    const TOKEN: &str = "secret-token";
    const PORT: u16 = 6427;

    struct Echo;

    impl Host for Echo {
        fn call(&self, tool: &str, args: Value) -> Result<Value, String> {
            match tool {
                "archive" => Err("No conversation with id x".into()),
                _ => Ok(json!({"tool": tool, "args": args})),
            }
        }
    }

    fn shared(prefs: McpPrefs) -> Mutex<Shared> {
        let mut s = Shared::new(TOKEN.into(), prefs);
        s.port = PORT;
        Mutex::new(s)
    }

    fn on() -> McpPrefs {
        McpPrefs {
            enabled: true,
            ..Default::default()
        }
    }

    fn post(body: Value) -> Request {
        Request {
            method: "POST".into(),
            path: PATH.into(),
            headers: vec![
                ("Host".into(), format!("127.0.0.1:{PORT}")),
                ("Authorization".into(), format!("Bearer {TOKEN}")),
                ("Content-Type".into(), "application/json".into()),
            ],
            body: body.to_string().into_bytes(),
        }
    }

    fn rpc(method: &str, params: Value) -> Request {
        post(json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
    }

    fn body(response: &Response) -> Value {
        serde_json::from_slice(&response.body).unwrap()
    }

    fn header<'a>(response: &'a Response, name: &str) -> Option<&'a str> {
        response
            .headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    #[test]
    fn initialize_negotiates_and_opens_a_session() {
        let shared = shared(on());
        let response = handle(
            &rpc(
                "initialize",
                json!({"protocolVersion": "2025-06-18", "clientInfo": {"name": "Claude Code"}}),
            ),
            &shared,
            &Echo,
        );
        assert_eq!(response.status, 200);
        let v = body(&response);
        assert_eq!(v["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(v["result"]["serverInfo"]["name"], "Pigeon");
        assert!(v["result"]["capabilities"]["tools"].is_object());
        let session = header(&response, "mcp-session-id").unwrap().to_string();
        // An unknown version gets ours.
        let v = body(&handle(
            &rpc("initialize", json!({"protocolVersion": "1999-01-01"})),
            &shared,
            &Echo,
        ));
        assert_eq!(v["result"]["protocolVersion"], LEGACY_VERSIONS[0]);
        assert!(v["result"].get("resultType").is_none());
        // The session names the client in the activity log.
        let mut request = rpc("tools/call", json!({"name": "list_labels"}));
        request.headers.push(("Mcp-Session-Id".into(), session));
        handle(&request, &shared, &Echo);
        let s = shared.lock().unwrap();
        assert_eq!(s.activity[0].client, "Claude Code");
        assert_eq!(s.activity[0].tool, "list_labels");
        assert!(s.activity[0].ok);
    }

    #[test]
    fn notifications_ping_and_unknown_methods() {
        let shared = shared(on());
        let note = post(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        let response = handle(&note, &shared, &Echo);
        assert_eq!(response.status, 202);
        assert!(response.body.is_empty());
        let v = body(&handle(&rpc("ping", json!({})), &shared, &Echo));
        assert_eq!(v["result"], json!({}));
        let v = body(&handle(&rpc("resources/list", json!({})), &shared, &Echo));
        assert_eq!(v["error"]["code"], -32601);
        let mut bad = post(json!({}));
        bad.body = b"{not json".to_vec();
        let response = handle(&bad, &shared, &Echo);
        assert_eq!(response.status, 400);
        assert_eq!(body(&response)["error"]["code"], -32700);
    }

    #[test]
    fn tools_list_names_every_tool_and_its_switch() {
        let shared = shared(on());
        let v = body(&handle(&rpc("tools/list", json!({})), &shared, &Echo));
        let listed = v["result"]["tools"].as_array().unwrap();
        assert_eq!(listed.len(), tools::TOOLS.len());
        let search = listed.iter().find(|t| t["name"] == "search_mail").unwrap();
        assert_eq!(search["inputSchema"]["type"], "object");
        assert!(
            search["description"]
                .as_str()
                .unwrap()
                .contains("Read mail")
        );
        let send = listed.iter().find(|t| t["name"] == "send_mail").unwrap();
        assert_eq!(send["annotations"]["readOnlyHint"], false);
    }

    #[test]
    fn tool_calls_check_their_switch() {
        // Read mail is on by default, Send mail off.
        let shared = shared(on());
        let v = body(&handle(
            &rpc(
                "tools/call",
                json!({"name": "search_mail", "arguments": {"query": "x"}}),
            ),
            &shared,
            &Echo,
        ));
        assert_eq!(v["result"]["isError"], false);
        assert!(
            v["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("search_mail")
        );
        let v = body(&handle(
            &rpc("tools/call", json!({"name": "send_mail", "arguments": {}})),
            &shared,
            &Echo,
        ));
        assert_eq!(v["result"]["isError"], true);
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains(Capability::SendMail.title()), "{text}");
        assert!(!shared.lock().unwrap().activity[0].ok);
        // The whole server off: nothing runs.
        let off = self::shared(McpPrefs::default());
        let v = body(&handle(
            &rpc("tools/call", json!({"name": "list_labels"})),
            &off,
            &Echo,
        ));
        assert_eq!(v["result"]["isError"], true);
    }

    #[test]
    fn tool_errors_and_unknown_tools() {
        let shared = shared(McpPrefs {
            organize_mail: true,
            ..on()
        });
        let v = body(&handle(
            &rpc(
                "tools/call",
                json!({"name": "archive", "arguments": {"ids": ["x"]}}),
            ),
            &shared,
            &Echo,
        ));
        assert_eq!(v["result"]["isError"], true);
        assert_eq!(
            v["result"]["content"][0]["text"],
            "No conversation with id x"
        );
        let v = body(&handle(
            &rpc("tools/call", json!({"name": "format_disk"})),
            &shared,
            &Echo,
        ));
        assert_eq!(v["error"]["code"], -32602);
    }

    #[test]
    fn refuses_bad_tokens_origins_hosts_and_methods() {
        let shared = shared(on());
        let mut request = rpc("ping", json!({}));
        request.headers.retain(|(n, _)| n != "Authorization");
        assert_eq!(handle(&request, &shared, &Echo).status, 401);
        request
            .headers
            .push(("Authorization".into(), "Bearer secret-tokem".into()));
        let response = handle(&request, &shared, &Echo);
        assert_eq!(response.status, 401);
        assert!(header(&response, "www-authenticate").is_some());

        let mut request = rpc("ping", json!({}));
        request
            .headers
            .push(("Origin".into(), "https://evil.example".into()));
        assert_eq!(handle(&request, &shared, &Echo).status, 403);
        let mut request = rpc("ping", json!({}));
        request
            .headers
            .push(("Origin".into(), "http://localhost:6274".into()));
        assert_eq!(handle(&request, &shared, &Echo).status, 200);

        let mut request = rpc("ping", json!({}));
        request.headers[0].1 = format!("evil.example:{PORT}");
        assert_eq!(handle(&request, &shared, &Echo).status, 403);

        let mut request = rpc("ping", json!({}));
        request.method = "GET".into();
        assert_eq!(handle(&request, &shared, &Echo).status, 405);
        let mut request = rpc("ping", json!({}));
        request.path = "/other".into();
        assert_eq!(handle(&request, &shared, &Echo).status, 404);

        let mut request = rpc("ping", json!({}));
        request
            .headers
            .push(("Mcp-Session-Id".into(), "gone".into()));
        assert_eq!(handle(&request, &shared, &Echo).status, 404);
        let mut request = rpc("ping", json!({}));
        request
            .headers
            .push(("MCP-Protocol-Version".into(), "2024-01-01".into()));
        let response = handle(&request, &shared, &Echo);
        assert_eq!(response.status, 400);
        assert_eq!(body(&response)["error"]["code"], UNSUPPORTED_VERSION);
        // `initialize` negotiates in its body, whatever the header says.
        let mut request = rpc("initialize", json!({"protocolVersion": "2026-07-28"}));
        request
            .headers
            .push(("MCP-Protocol-Version".into(), "2026-07-28".into()));
        let response = handle(&request, &shared, &Echo);
        assert_eq!(response.status, 200);
        assert_eq!(
            body(&response)["result"]["protocolVersion"],
            LEGACY_VERSIONS[0]
        );
    }

    /// A 2026-07-28 request with its `_meta` and mirrored headers.
    fn modern(method: &str, mut params: Value) -> Request {
        params["_meta"] = json!({
            META_VERSION: "2026-07-28",
            META_CLIENT: {"name": "Codex", "version": "1.0"},
            META_CAPABILITIES: {},
        });
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string);
        let mut request = rpc(method, params);
        request.headers.extend([
            ("MCP-Protocol-Version".into(), "2026-07-28".into()),
            ("Mcp-Method".into(), method.into()),
        ]);
        if let Some(name) = name {
            request.headers.push(("Mcp-Name".into(), name));
        }
        request
    }

    fn set_header(request: &mut Request, name: &str, value: &str) {
        request
            .headers
            .retain(|(n, _)| !n.eq_ignore_ascii_case(name));
        request.headers.push((name.into(), value.into()));
    }

    #[test]
    fn server_discover_lists_versions_and_capabilities() {
        let shared = shared(on());
        let response = handle(&modern("server/discover", json!({})), &shared, &Echo);
        assert_eq!(response.status, 200);
        assert!(header(&response, "mcp-session-id").is_none());
        let r = &body(&response)["result"];
        assert_eq!(r["resultType"], "complete");
        assert_eq!(r["supportedVersions"], json!(PROTOCOL_VERSIONS));
        assert_eq!(r["supportedVersions"][0], "2026-07-28");
        assert!(r["capabilities"]["tools"].is_object());
        assert_eq!(r["_meta"][META_SERVER]["name"], "Pigeon");
        assert!(r["instructions"].as_str().unwrap().contains("Pigeon"));
        assert!(r["ttlMs"].as_u64().is_some());
        assert_eq!(r["cacheScope"], "public");
        // Without any `_meta`, it's a malformed modern request.
        let response = handle(&rpc("server/discover", json!({})), &shared, &Echo);
        assert_eq!(response.status, 400);
        assert_eq!(body(&response)["error"]["code"], -32602);
    }

    #[test]
    fn modern_tools_list_and_call_without_a_session() {
        let shared = shared(on());
        let response = handle(&modern("tools/list", json!({})), &shared, &Echo);
        assert_eq!(response.status, 200);
        let r = &body(&response)["result"];
        assert_eq!(r["resultType"], "complete");
        assert_eq!(r["tools"].as_array().unwrap().len(), tools::TOOLS.len());
        assert!(r["ttlMs"].as_u64().is_some());
        assert_eq!(r["cacheScope"], "public");
        assert_eq!(r["_meta"][META_SERVER]["name"], "Pigeon");

        // A stale legacy session id is ignored, not a 404.
        let mut request = modern(
            "tools/call",
            json!({"name": "search_mail", "arguments": {"query": "x"}}),
        );
        request
            .headers
            .push(("Mcp-Session-Id".into(), "gone".into()));
        let response = handle(&request, &shared, &Echo);
        assert_eq!(response.status, 200);
        assert!(header(&response, "mcp-session-id").is_none());
        let r = &body(&response)["result"];
        assert_eq!(r["resultType"], "complete");
        assert_eq!(r["isError"], false);
        assert!(
            r["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("search_mail")
        );
        // The client's name comes from this request's `_meta`.
        let s = shared.lock().unwrap();
        assert_eq!(s.activity[0].client, "Codex");
        assert_eq!(s.activity[0].tool, "search_mail");
        drop(s);

        // Switched off: still a tool error, now with resultType.
        let r = body(&handle(
            &modern("tools/call", json!({"name": "send_mail", "arguments": {}})),
            &shared,
            &Echo,
        ));
        assert_eq!(r["result"]["resultType"], "complete");
        assert_eq!(r["result"]["isError"], true);

        // A base64-encoded Mcp-Name is decoded before comparing.
        let mut request = modern("tools/call", json!({"name": "list_labels"}));
        set_header(&mut request, "Mcp-Name", "=?base64?bGlzdF9sYWJlbHM=?=");
        assert_eq!(handle(&request, &shared, &Echo).status, 200);

        // `ping` is gone in 2026-07-28: 404 with -32601.
        let mut request = modern("ping", json!({}));
        set_header(&mut request, "Mcp-Method", "ping");
        let response = handle(&request, &shared, &Echo);
        assert_eq!(response.status, 404);
        assert_eq!(body(&response)["error"]["code"], -32601);
    }

    #[test]
    fn unsupported_versions_list_the_supported_ones() {
        let shared = shared(on());
        let mut request = modern("tools/list", json!({}));
        let mut params = json!({});
        params["_meta"] = json!({META_VERSION: "1900-01-01", META_CAPABILITIES: {}});
        request.body = json!({"jsonrpc": "2.0", "id": 9, "method": "tools/list", "params": params})
            .to_string()
            .into_bytes();
        set_header(&mut request, "MCP-Protocol-Version", "1900-01-01");
        let response = handle(&request, &shared, &Echo);
        assert_eq!(response.status, 400);
        let v = body(&response);
        assert_eq!(v["id"], 9);
        assert_eq!(v["error"]["code"], UNSUPPORTED_VERSION);
        assert_eq!(v["error"]["data"]["supported"], json!(PROTOCOL_VERSIONS));
        assert_eq!(v["error"]["data"]["requested"], "1900-01-01");
    }

    #[test]
    fn modern_requests_naming_a_legacy_version_point_to_initialize() {
        let shared = shared(on());
        let mut request = modern("tools/list", json!({}));
        let mut params = json!({});
        params["_meta"] = json!({META_VERSION: "2025-06-18", META_CAPABILITIES: {}});
        request.body = json!({"jsonrpc": "2.0", "id": 4, "method": "tools/list", "params": params})
            .to_string()
            .into_bytes();
        set_header(&mut request, "MCP-Protocol-Version", "2025-06-18");
        let response = handle(&request, &shared, &Echo);
        assert_eq!(response.status, 400);
        let v = body(&response);
        assert_eq!(v["id"], 4);
        assert_eq!(v["error"]["code"], UNSUPPORTED_VERSION);
        let supported = v["error"]["data"]["supported"].as_array().unwrap();
        assert!(supported.contains(&json!("2025-06-18")));
        assert_eq!(v["error"]["data"]["requested"], "2025-06-18");
        assert!(
            v["error"]["message"]
                .as_str()
                .unwrap()
                .contains("initialize")
        );
    }

    #[test]
    fn a_null_id_is_an_invalid_request_not_a_notification() {
        let shared = shared(on());
        let legacy = post(json!({"jsonrpc": "2.0", "id": null, "method": "tools/list"}));
        let mut modern_null = modern("tools/list", json!({}));
        let mut v: Value = serde_json::from_slice(&modern_null.body).unwrap();
        v["id"] = Value::Null;
        modern_null.body = v.to_string().into_bytes();
        for request in [legacy, modern_null] {
            let response = handle(&request, &shared, &Echo);
            assert_eq!(response.status, 400);
            let v = body(&response);
            assert_eq!(v["id"], Value::Null);
            assert_eq!(v["error"]["code"], -32600);
        }
        // A missing id is still a notification.
        let note = post(json!({"jsonrpc": "2.0", "method": "notifications/cancelled"}));
        assert_eq!(handle(&note, &shared, &Echo).status, 202);
    }

    #[test]
    fn headers_must_match_the_body() {
        let shared = shared(on());
        let call = || modern("tools/call", json!({"name": "list_labels"}));
        let mismatch = |request: Request| {
            let response = handle(&request, &shared, &Echo);
            assert_eq!(response.status, 400);
            let v = body(&response);
            assert_eq!(v["error"]["code"], HEADER_MISMATCH, "{v}");
            assert_eq!(v["id"], 1);
        };
        let mut request = call();
        set_header(&mut request, "MCP-Protocol-Version", "2025-11-25");
        mismatch(request);
        let mut request = call();
        request
            .headers
            .retain(|(n, _)| !n.eq_ignore_ascii_case("mcp-protocol-version"));
        mismatch(request);
        let mut request = call();
        set_header(&mut request, "Mcp-Method", "tools/list");
        mismatch(request);
        let mut request = call();
        request.headers.retain(|(n, _)| n != "Mcp-Method");
        mismatch(request);
        let mut request = call();
        set_header(&mut request, "Mcp-Name", "send_mail");
        mismatch(request);
        let mut request = call();
        request.headers.retain(|(n, _)| n != "Mcp-Name");
        mismatch(request);
        // Nothing ran.
        assert!(shared.lock().unwrap().activity.is_empty());

        // Missing clientCapabilities is a malformed request: -32602.
        let mut request = call();
        let mut v: Value = serde_json::from_slice(&request.body).unwrap();
        v["params"]["_meta"]
            .as_object_mut()
            .unwrap()
            .remove(META_CAPABILITIES);
        request.body = v.to_string().into_bytes();
        let response = handle(&request, &shared, &Echo);
        assert_eq!(response.status, 400);
        assert_eq!(body(&response)["error"]["code"], -32602);
    }

    #[test]
    fn legacy_initialize_flow_still_works() {
        let shared = shared(on());
        let mut request = rpc(
            "initialize",
            json!({"protocolVersion": "2025-11-25", "clientInfo": {"name": "Cursor"}}),
        );
        set_header(&mut request, "MCP-Protocol-Version", "2025-11-25");
        let response = handle(&request, &shared, &Echo);
        assert_eq!(response.status, 200);
        assert_eq!(body(&response)["result"]["protocolVersion"], "2025-11-25");
        let session = header(&response, "mcp-session-id").unwrap().to_string();
        let legacy = |method: &str, params: Value| {
            let mut request = rpc(method, params);
            request.headers.extend([
                ("MCP-Protocol-Version".into(), "2025-11-25".into()),
                ("Mcp-Session-Id".into(), session.clone()),
            ]);
            request
        };
        let note = {
            let mut r = legacy("x", json!({}));
            r.body = json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
                .to_string()
                .into_bytes();
            r
        };
        assert_eq!(handle(&note, &shared, &Echo).status, 202);
        let v = body(&handle(&legacy("ping", json!({})), &shared, &Echo));
        assert_eq!(v["result"], json!({}));
        let v = body(&handle(&legacy("tools/list", json!({})), &shared, &Echo));
        assert!(v["result"].get("resultType").is_none());
        assert!(v["result"].get("ttlMs").is_none());
        assert_eq!(
            v["result"]["tools"].as_array().unwrap().len(),
            tools::TOOLS.len()
        );
        let v = body(&handle(
            &legacy("tools/call", json!({"name": "list_labels"})),
            &shared,
            &Echo,
        ));
        assert_eq!(v["result"]["isError"], false);
        assert!(v["result"].get("resultType").is_none());
        assert_eq!(shared.lock().unwrap().activity[0].client, "Cursor");
        // No header at all is 2025-03-26.
        let v = body(&handle(&rpc("tools/list", json!({})), &shared, &Echo));
        assert!(v["result"]["tools"].is_array());
        // DELETE ends the session; it's unknown after.
        let mut delete = legacy("ping", json!({}));
        delete.method = "DELETE".into();
        assert_eq!(handle(&delete, &shared, &Echo).status, 200);
        assert_eq!(
            handle(&legacy("ping", json!({})), &shared, &Echo).status,
            404
        );
    }

    #[test]
    fn slots_come_back_even_after_a_panic() {
        let open = Arc::new(AtomicUsize::new(0));
        let slot = Slot::take(&open);
        assert_eq!(open.load(Ordering::SeqCst), 1);
        let _ = std::thread::spawn(move || {
            let _slot = slot;
            panic!("serve panicked");
        })
        .join();
        assert_eq!(open.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn origins() {
        assert!(local_origin("http://127.0.0.1:8080"));
        assert!(local_origin("http://localhost"));
        assert!(!local_origin("http://localhost.evil.com"));
        assert!(!local_origin("http://127.0.0.1.nip.io:80"));
        assert!(!local_origin("null"));
    }

    #[test]
    fn serves_over_a_real_socket() {
        let shared = Arc::new(shared(on()));
        let server = Server::start(shared.clone(), Arc::new(Echo), 0).unwrap();
        let port = server.port();
        assert_eq!(shared.lock().unwrap().port, port);
        let body = json!({"jsonrpc": "2.0", "id": 7, "method": "ping"}).to_string();
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        write!(
            stream,
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {TOKEN}\r\n\
             Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let mut answer = String::new();
        stream.read_to_string(&mut answer).unwrap();
        assert!(answer.starts_with("HTTP/1.1 200 OK"), "{answer}");
        let json = answer.split("\r\n\r\n").nth(1).unwrap();
        let v: Value = serde_json::from_str(json).unwrap();
        assert_eq!(v["id"], 7);
        assert_eq!(v["result"], json!({}));
        drop(server);
        assert!(TcpStream::connect((Ipv4Addr::LOCALHOST, port)).is_err());
    }
}
