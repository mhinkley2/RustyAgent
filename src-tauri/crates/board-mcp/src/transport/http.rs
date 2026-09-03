//! Streamable-HTTP transport (POST only — no SSE, so the server is stateless).

use std::{convert::Infallible, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::{
    body::Incoming,
    header::{ALLOW, AUTHORIZATION, CONTENT_TYPE, HOST, ORIGIN, WWW_AUTHENTICATE},
    server::conn::http1,
    service::service_fn,
    HeaderMap, Method, Request, Response, StatusCode,
};
use hyper_util::rt::TokioIo;
use serde_json::{json, Value};
use tokio::net::TcpListener as TokioTcpListener;
use tracing::{error, info, warn};

use crate::{
    auth::AuthConfig,
    ctx::{McpCtx, PinScope},
    jsonrpc::{
        error_response, FORBIDDEN, INVALID_PARAMS, INVALID_REQUEST, PARSE_ERROR, UNAUTHORIZED,
    },
    protocol::handle_message_refreshed,
    registry::McpRegistry,
    tools::build_registry,
    WORKSPACE_HEADER, WORKSPACE_QUERY_KEY,
};

pub const MCP_ENDPOINT_PATH: &str = "/mcp";
const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;
/// Give up on the accept loop only after sustained failure.
const MAX_CONSECUTIVE_ACCEPT_FAILURES: u32 = 64;

/// One server, every client.
///
/// `ctx` is the *unscoped* context — the app's active workspace, which is what
/// a client that names no project gets. A client that does name one is served a
/// clone confined to it, resolved per request in [`scope_for`]. Nothing
/// per-client is stored here, which is what lets one process replace the
/// one-stdio-process-per-editor-window arrangement without growing a session
/// table to leak.
pub struct HttpState {
    pub ctx: McpCtx,
    pub registry: McpRegistry,
    pub auth: AuthConfig,
}

/// A bound but not-yet-serving MCP HTTP server.
///
/// Binding is separate from serving so the caller can spawn the accept loop on
/// whichever runtime it owns. This crate must not depend on Tauri, and the
/// desktop app's setup hook runs *outside* a Tokio runtime context — calling
/// `tokio::spawn` here would panic with "there is no reactor running".
pub struct BoundServer {
    listener: std::net::TcpListener,
    addr: SocketAddr,
    state: Arc<HttpState>,
}

impl BoundServer {
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Run the accept loop. Spawn this on your runtime; it never returns
    /// normally.
    pub async fn serve(self) {
        // Registering with the reactor has to happen inside the runtime, which
        // is why it is here rather than in `bind`.
        let listener = match TokioTcpListener::from_std(self.listener) {
            Ok(listener) => listener,
            Err(error) => {
                error!("Failed to register the MCP HTTP listener with the reactor: {error}");
                return;
            }
        };

        let mut consecutive_failures: u32 = 0;

        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    consecutive_failures = 0;
                    let io = TokioIo::new(stream);
                    let state = self.state.clone();

                    tokio::spawn(async move {
                        let service =
                            service_fn(move |request| handle_request(request, state.clone()));
                        if let Err(error) =
                            http1::Builder::new().serve_connection(io, service).await
                        {
                            warn!("MCP HTTP connection failed: {error}");
                        }
                    });
                }
                Err(error) => {
                    // Transient conditions (EMFILE, ECONNABORTED) recover on
                    // their own; back off rather than killing the server for
                    // the rest of the process lifetime.
                    consecutive_failures += 1;
                    warn!(
                        "MCP HTTP accept failed ({consecutive_failures}/\
                         {MAX_CONSECUTIVE_ACCEPT_FAILURES}): {error}"
                    );
                    if consecutive_failures >= MAX_CONSECUTIVE_ACCEPT_FAILURES {
                        error!("MCP HTTP accept loop giving up after sustained failures");
                        return;
                    }
                    let backoff = 50 * u64::from(consecutive_failures.min(20));
                    tokio::time::sleep(Duration::from_millis(backoff)).await;
                }
            }
        }
    }
}

/// Claim the port.
///
/// Synchronous and runtime-free, so a port conflict surfaces to the caller
/// immediately rather than inside a detached task where it could only be
/// logged.
pub fn bind(state: HttpState, addr: SocketAddr) -> Result<BoundServer, String> {
    let listener = std::net::TcpListener::bind(addr)
        .map_err(|error| format!("Failed to bind MCP HTTP server to {addr}: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("Failed to set MCP HTTP listener non-blocking: {error}"))?;

    // Report the address actually bound, not the one requested — they differ
    // when the caller asks for port 0.
    let addr = listener.local_addr().unwrap_or(addr);

    info!("MCP HTTP server listening at http://{addr}{MCP_ENDPOINT_PATH}");

    Ok(BoundServer {
        listener,
        addr,
        state: Arc::new(state),
    })
}

/// Build a registry and context for an out-of-app HTTP server.
pub fn state(ctx: McpCtx, auth: AuthConfig) -> HttpState {
    HttpState {
        ctx,
        registry: build_registry(),
        auth,
    }
}

pub(crate) async fn handle_request(
    request: Request<Incoming>,
    state: Arc<HttpState>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    Ok(respond(request, state).await)
}

async fn respond(request: Request<Incoming>, state: Arc<HttpState>) -> Response<Full<Bytes>> {
    let (parts, body) = request.into_parts();

    // Gate on auth before reading the body, so an unauthenticated caller can
    // neither map the endpoint nor make us buffer their upload.
    if let Some(rejection) = check_access(&parts.headers, &state) {
        return rejection;
    }
    if let Some(rejection) = check_route(&parts.method, parts.uri.path()) {
        return rejection;
    }

    // Reject an oversized body before reading it, when the sender declares one.
    if let Some(declared) = parts
        .headers
        .get(hyper::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
    {
        if declared > MAX_BODY_BYTES {
            return too_large();
        }
    }

    let collected = match Limited::new(body, MAX_BODY_BYTES).collect().await {
        Ok(body) => body.to_bytes(),
        Err(_) => return too_large(),
    };

    dispatch_body(&collected, &parts.headers, parts.uri.query(), &state).await
}

/// Authentication and origin checks. `Some(response)` means "rejected".
///
/// Split out from [`respond`] so it can be exercised without constructing a
/// hyper `Incoming` body, which is not publicly constructible.
fn check_access(
    headers: &hyper::HeaderMap,
    state: &HttpState,
) -> Option<Response<Full<Bytes>>> {
    let origin = headers.get(ORIGIN).and_then(|value| value.to_str().ok());
    let host = headers.get(HOST).and_then(|value| value.to_str().ok());

    if !state.auth.check_origin(origin, host) {
        return Some(json_error(
            StatusCode::FORBIDDEN,
            FORBIDDEN,
            "Forbidden: cross-origin request rejected",
        ));
    }

    let authorization = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    if !state.auth.check_bearer(authorization) {
        // One body for missing, malformed, and wrong — no oracle.
        let mut response = json_error(
            StatusCode::UNAUTHORIZED,
            UNAUTHORIZED,
            "Unauthorized: missing or invalid bearer token",
        );
        response.headers_mut().insert(
            WWW_AUTHENTICATE,
            hyper::header::HeaderValue::from_static("Bearer realm=\"rustyagent-board-mcp\""),
        );
        return Some(response);
    }

    None
}

/// Path and method checks. `Some(response)` means "rejected".
fn check_route(method: &Method, path: &str) -> Option<Response<Full<Bytes>>> {
    if path != MCP_ENDPOINT_PATH {
        return Some(empty(StatusCode::NOT_FOUND));
    }
    if method != Method::POST {
        let mut response = empty(StatusCode::METHOD_NOT_ALLOWED);
        response
            .headers_mut()
            .insert(ALLOW, hyper::header::HeaderValue::from_static("POST"));
        return Some(response);
    }
    None
}

/// The project a request named, before it is checked against the database.
///
/// The header wins; the query parameter is there for a client that can
/// template a URL but not a header value. Both are read because which one a
/// given editor can produce is a property of that editor, not of this server.
///
/// A present-but-unreadable header is an `Err` rather than a shrug. Ignoring it
/// would attach that client to whatever project the app happens to have open,
/// which is the exact failure the header exists to prevent — and it would do so
/// silently, which is worse than refusing.
fn requested_workspace(headers: &HeaderMap, query: Option<&str>) -> Result<Option<String>, String> {
    if let Some(value) = headers.get(WORKSPACE_HEADER) {
        let text = value.to_str().map_err(|_| {
            format!(
                "{WORKSPACE_HEADER} is not readable as text. Send the workspace path as \
                 ASCII, or name it with the '{WORKSPACE_QUERY_KEY}' query parameter instead."
            )
        })?;
        // Blank is absent: a client that templated a variable which resolved to
        // nothing meant "no scope", not "the workspace named empty string".
        if !text.trim().is_empty() {
            return Ok(Some(text.trim().to_string()));
        }
    }

    let Some(query) = query else {
        return Ok(None);
    };
    Ok(form_urlencoded::parse(query.as_bytes())
        .find(|(key, _)| key == WORKSPACE_QUERY_KEY)
        .map(|(_, value)| value.trim().to_string())
        .filter(|value| !value.is_empty()))
}

/// Confine this request to the project it named, if it named one.
///
/// Resolved against workspaces the user has already opened and never
/// registering a new one — the same rule the stdio pin follows, and for the
/// same reason: a client that could register could point itself at any
/// directory and then read it through `read_file`.
///
/// `db::find_workspace_by_path` normalizes the Windows extended-length prefix
/// before it matches, so a client sending the `\\?\` form resolves to the same
/// row as one sending the plain path. Getting that wrong would silently hand a
/// client a different board.
async fn scope_for(
    headers: &HeaderMap,
    query: Option<&str>,
    state: &HttpState,
) -> Result<McpCtx, String> {
    let requested = requested_workspace(headers, query)?;
    let Some(requested) = requested else {
        // No project named: follow the app's active workspace, which is what
        // the app's own webview wants and what every pre-existing config gets.
        return Ok(state.ctx.clone());
    };

    match db::find_workspace_by_path(&state.ctx.db, &PathBuf::from(&requested)).await {
        Some(workspace) => Ok(state.ctx.clone().pinned_to(
            PathBuf::from(&workspace.path),
            Some(workspace.id),
            PinScope::Request,
        )),
        // Refused on this request only. The server is already running for every
        // other client, so it cannot refuse at startup the way the stdio binary
        // does — the failure has to land on the request that carried the bad
        // value, and nowhere else.
        None => Err(format!(
            "'{requested}' is not a workspace this RustyAgent has opened. Open the folder \
             in the app once to register it, or drop {WORKSPACE_HEADER} to follow the \
             app's active workspace."
        )),
    }
}

async fn dispatch_body(
    collected: &[u8],
    headers: &HeaderMap,
    query: Option<&str>,
    state: &HttpState,
) -> Response<Full<Bytes>> {
    let message: Value = match serde_json::from_slice(collected) {
        Ok(value) => value,
        Err(error) => {
            return json_body(
                StatusCode::BAD_REQUEST,
                error_response(
                    Value::Null,
                    PARSE_ERROR,
                    format!("Invalid JSON payload: {error}"),
                ),
            )
        }
    };

    if message.is_array() {
        return json_body(
            StatusCode::BAD_REQUEST,
            error_response(
                Value::Null,
                INVALID_REQUEST,
                "Batch requests are not supported; send one JSON-RPC message per POST.",
            ),
        );
    }
    if !message.is_object() {
        return json_body(
            StatusCode::BAD_REQUEST,
            error_response(
                Value::Null,
                INVALID_REQUEST,
                "Request body must be a JSON-RPC object",
            ),
        );
    }

    // After the body is parsed, so the refusal can echo the request's id. A
    // notification carrying a bad scope is still answered: silently dropping it
    // would leave the client believing it wrote to a board it never reached.
    let ctx = match scope_for(headers, query, state).await {
        Ok(ctx) => ctx,
        Err(error) => {
            let id = message.get("id").cloned().unwrap_or(Value::Null);
            return json_body(
                StatusCode::BAD_REQUEST,
                error_response(id, INVALID_PARAMS, error),
            );
        }
    };

    match handle_message_refreshed(&ctx, &state.registry, &message).await {
        // A notification gets an empty 202, not a JSON-RPC envelope.
        None => empty(StatusCode::ACCEPTED),
        Some(response) => json_body(StatusCode::OK, response),
    }
}

fn too_large() -> Response<Full<Bytes>> {
    json_body(
        StatusCode::PAYLOAD_TOO_LARGE,
        error_response(
            Value::Null,
            INVALID_REQUEST,
            format!("Request body exceeds {} MiB", MAX_BODY_BYTES / 1024 / 1024),
        ),
    )
}

fn json_error(status: StatusCode, code: i32, message: &str) -> Response<Full<Bytes>> {
    json_body(status, error_response(Value::Null, code, message))
}

fn json_body(status: StatusCode, body: Value) -> Response<Full<Bytes>> {
    let bytes = serde_json::to_vec(&body).unwrap_or_else(|_| {
        serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "id": null,
            "error": { "code": -32603, "message": "Failed to serialize response" }
        }))
        .expect("static JSON always serializes")
    });

    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json")
        .body(Full::new(Bytes::from(bytes)))
        .expect("valid response")
}

fn empty(status: StatusCode) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .body(Full::new(Bytes::new()))
        .expect("valid response")
}

#[cfg(test)]
mod tests {
    //! Transport-level tests.
    //!
    //! In-crate rather than under `tests/` so `check_access`, `check_route`, and
    //! `dispatch_body` can stay private — they are internal plumbing, not part
    //! of this crate's API. `respond` is split into them precisely because
    //! hyper's `Incoming` body cannot be constructed outside hyper, so the full
    //! request path is not directly callable from a test.

    use super::*;
    use crate::McpCtx;
    use hyper::header::{AUTHORIZATION, HOST, ORIGIN};
    use hyper::HeaderMap;
    use std::path::PathBuf;

    const TOKEN: &str = "test-token-value";

    async fn http_state() -> HttpState {
        state(
            McpCtx::new(db::testing::make_test_pool().await),
            AuthConfig {
                token: Some(TOKEN.to_string()),
                port: 8765,
            },
        )
    }

    fn headers(pairs: &[(hyper::header::HeaderName, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(name.clone(), value.parse().expect("valid header"));
        }
        map
    }

    fn authorized() -> HeaderMap {
        headers(&[
            (AUTHORIZATION, &format!("Bearer {TOKEN}")),
            (HOST, "127.0.0.1:8765"),
        ])
    }

    /// Status plus decoded JSON body.
    async fn parts(response: Response<Full<Bytes>>) -> (StatusCode, Value) {
        let status = response.status();
        let bytes = response.into_body().collect().await.expect("collect").to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    // -- binding -----------------------------------------------------------------

    /// Build a state without a runtime, for the sync binding tests below.
    fn blocking_state() -> HttpState {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let db = runtime.block_on(db::testing::make_test_pool());
        state(
            McpCtx::new(db),
            AuthConfig {
                token: Some(TOKEN.to_string()),
                port: 0,
            },
        )
    }

    #[test]
    fn binding_does_not_require_a_running_runtime() {
        // Regression: `bind` used to call `tokio::spawn`, which panics with
        // "there is no reactor running" when called from Tauri's setup hook.
        // This is a plain #[test] on purpose — no #[tokio::test] — so it fails
        // if a runtime dependency creeps back into the synchronous path.
        let addr = SocketAddr::from(([127, 0, 0, 1], 0));

        let server = bind(blocking_state(), addr).expect("bind should succeed");

        // Port 0 means "any free port"; the OS picked a real one.
        assert_ne!(server.addr().port(), 0);
    }

    #[test]
    fn binding_a_port_already_in_use_reports_an_error_rather_than_panicking() {
        // What makes the desktop app's non-fatal-bind path possible.
        let occupied = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("occupy a port");
        let addr = occupied.local_addr().expect("addr");

        let result = bind(blocking_state(), addr);

        let error = result.err().expect("binding a taken port must fail");
        assert!(error.contains("Failed to bind"), "got {error}");
    }

    // -- authentication ----------------------------------------------------------

    #[tokio::test]
    async fn a_missing_token_is_rejected_with_401_and_a_challenge() {
        let state = http_state().await;

        let response = check_access(&headers(&[(HOST, "127.0.0.1:8765")]), &state)
            .expect("should be rejected");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            response
                .headers()
                .get(hyper::header::WWW_AUTHENTICATE)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer realm=\"rustyagent-board-mcp\"")
        );
        let (_, body) = parts(response).await;
        assert_eq!(body["error"]["code"], json!(-32001));
    }

    #[tokio::test]
    async fn missing_malformed_and_wrong_tokens_return_identical_bodies() {
        // No oracle: an attacker must not be able to tell "absent" from "wrong".
        let state = http_state().await;

        let mut bodies = Vec::new();
        for header_map in [
            headers(&[(HOST, "127.0.0.1:8765")]),
            headers(&[(AUTHORIZATION, "Basic abc"), (HOST, "127.0.0.1:8765")]),
            headers(&[
                (AUTHORIZATION, "Bearer wrong-token-value"),
                (HOST, "127.0.0.1:8765"),
            ]),
        ] {
            let response = check_access(&header_map, &state).expect("should be rejected");
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            bodies.push(parts(response).await.1);
        }

        assert_eq!(bodies[0], bodies[1]);
        assert_eq!(bodies[1], bodies[2]);
    }

    #[tokio::test]
    async fn a_correct_token_passes() {
        let state = http_state().await;

        assert!(check_access(&authorized(), &state).is_none());
    }

    // -- origin and host ---------------------------------------------------------

    #[tokio::test]
    async fn a_foreign_origin_is_rejected_with_403() {
        let state = http_state().await;
        let mut headers = authorized();
        headers.insert(ORIGIN, "http://evil.com".parse().unwrap());

        let response = check_access(&headers, &state).expect("should be rejected");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(parts(response).await.1["error"]["code"], json!(-32002));
    }

    #[tokio::test]
    async fn localhost_and_tauri_origins_pass() {
        let state = http_state().await;

        for origin in [
            "http://localhost:1420",
            "http://127.0.0.1:8765",
            "tauri://localhost",
            "https://tauri.localhost",
        ] {
            let mut headers = authorized();
            headers.insert(ORIGIN, origin.parse().unwrap());
            assert!(
                check_access(&headers, &state).is_none(),
                "{origin} should pass"
            );
        }
    }

    #[tokio::test]
    async fn a_rebound_host_header_is_rejected() {
        // The DNS-rebinding guard: the request reaches 127.0.0.1 but carries the
        // attacker's hostname.
        let state = http_state().await;
        let headers = headers(&[
            (AUTHORIZATION, &format!("Bearer {TOKEN}")),
            (HOST, "attacker.example"),
        ]);

        let response = check_access(&headers, &state).expect("should be rejected");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn origin_is_checked_before_the_token_so_a_page_cannot_probe() {
        let state = http_state().await;
        let headers = headers(&[(ORIGIN, "http://evil.com"), (HOST, "127.0.0.1:8765")]);

        let response = check_access(&headers, &state).expect("should be rejected");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    // -- routing -----------------------------------------------------------------

    #[test]
    fn a_get_to_the_endpoint_is_405_with_an_allow_header() {
        let response = check_route(&Method::GET, "/mcp").expect("should be rejected");

        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(
            response
                .headers()
                .get(hyper::header::ALLOW)
                .and_then(|value| value.to_str().ok()),
            Some("POST")
        );
    }

    #[test]
    fn another_path_is_404() {
        let response = check_route(&Method::POST, "/other").expect("should be rejected");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn a_post_to_the_endpoint_is_routed() {
        assert!(check_route(&Method::POST, "/mcp").is_none());
    }

    // -- body handling -----------------------------------------------------------

    #[tokio::test]
    async fn a_valid_request_is_dispatched() {
        let state = http_state().await;
        let body = br#"{"jsonrpc":"2.0","id":1,"method":"ping","params":{}}"#;

        let response = dispatch_body(body, &HeaderMap::new(), None, &state).await;

        let (status, value) = parts(response).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["id"], json!(1));
        assert_eq!(value["result"], json!({}));
    }

    #[tokio::test]
    async fn a_notification_gets_an_empty_202() {
        let state = http_state().await;
        let body = br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;

        let response = dispatch_body(body, &HeaderMap::new(), None, &state).await;

        assert_eq!(response.status(), StatusCode::ACCEPTED);
    }

    #[tokio::test]
    async fn malformed_json_is_a_parse_error_with_400() {
        let state = http_state().await;

        let response = dispatch_body(b"{not json}", &HeaderMap::new(), None, &state).await;

        let (status, value) = parts(response).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(value["error"]["code"], json!(-32700));
    }

    #[tokio::test]
    async fn a_batch_request_is_rejected_with_an_explanation() {
        let state = http_state().await;

        let response = dispatch_body(
            b"[{\"jsonrpc\":\"2.0\",\"id\":1}]",
            &HeaderMap::new(),
            None,
            &state,
        )
        .await;

        let (status, value) = parts(response).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(value["error"]["code"], json!(-32600));
        assert!(
            value["error"]["message"]
                .as_str()
                .unwrap_or_default()
                .contains("Batch"),
            "the message should say batching is unsupported"
        );
    }

    #[tokio::test]
    async fn a_non_object_body_is_rejected() {
        let state = http_state().await;

        let response = dispatch_body(b"\"just a string\"", &HeaderMap::new(), None, &state).await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    // -- per-request workspace scope ---------------------------------------------

    // These drive two clients through one `HttpState`, which is the whole claim
    // of this transport: one process, one port, one context, and still a
    // different board per caller.

    /// Two registered projects on one board, with directories that exist.
    struct TwoProjects {
        state: HttpState,
        a: PathBuf,
        b: PathBuf,
        _root: tempfile::TempDir,
    }

    async fn two_projects() -> TwoProjects {
        let root = tempfile::tempdir().expect("temp dir");
        let a = root.path().join("project-a");
        let b = root.path().join("project-b");
        std::fs::create_dir_all(&a).expect("create project-a");
        std::fs::create_dir_all(&b).expect("create project-b");

        let db = db::testing::make_test_pool().await;
        db::testing::seed_workspace(&db, "ws-a", &a.to_string_lossy()).await;
        db::testing::seed_workspace(&db, "ws-b", &b.to_string_lossy()).await;

        let state = state(
            McpCtx::new(db),
            AuthConfig {
                token: Some(TOKEN.to_string()),
                port: 8765,
            },
        );

        TwoProjects {
            state,
            a,
            b,
            _root: root,
        }
    }

    fn workspace_header(value: &str) -> HeaderMap {
        let mut map = HeaderMap::new();
        map.insert(
            hyper::header::HeaderName::from_static("x-rustyagent-workspace"),
            value.parse().expect("valid header"),
        );
        map
    }

    fn scoped(path: &std::path::Path) -> HeaderMap {
        workspace_header(&path.to_string_lossy())
    }

    /// The structured payload of one `tools/call`, through the full body path.
    async fn call_tool(
        state: &HttpState,
        headers: &HeaderMap,
        query: Option<&str>,
        name: &str,
        arguments: Value,
    ) -> Value {
        let body = json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": name, "arguments": arguments },
        })
        .to_string();

        let (status, value) =
            parts(dispatch_body(body.as_bytes(), headers, query, state).await).await;
        assert_eq!(status, StatusCode::OK, "got {value}");
        value
    }

    /// The JSON a tool answered with, decoded out of the MCP text envelope.
    fn payload_of(response: &Value) -> Value {
        let text = response["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default();
        serde_json::from_str(text).unwrap_or(Value::Null)
    }

    async fn active_workspace(
        state: &HttpState,
        headers: &HeaderMap,
        query: Option<&str>,
    ) -> Value {
        payload_of(&call_tool(state, headers, query, "get_active_workspace", json!({})).await)
    }

    #[tokio::test]
    async fn two_clients_on_one_server_read_their_own_boards() {
        let p = two_projects().await;

        let a = active_workspace(&p.state, &scoped(&p.a), None).await;
        let b = active_workspace(&p.state, &scoped(&p.b), None).await;

        assert_eq!(a["workspace"]["id"], json!("ws-a"));
        assert_eq!(b["workspace"]["id"], json!("ws-b"));
    }

    /// Make one project the app's active workspace, deterministically.
    ///
    /// Not by seeding order: `last_opened_at` has millisecond resolution and
    /// two seeds land in the same millisecond often enough to matter, at which
    /// point the ordering tiebreak decides and the test flakes.
    async fn make_active(db: &db::DbPool, id: &str) {
        sqlx::query("UPDATE workspaces SET last_opened_at = ? WHERE id = ?")
            .bind("2099-01-01T00:00:00.000Z")
            .bind(id)
            .execute(db)
            .await
            .expect("promote a workspace");
    }

    #[tokio::test]
    async fn a_client_that_names_no_workspace_follows_the_app() {
        // The pre-existing behaviour, and what the app's own webview needs.
        let p = two_projects().await;
        make_active(&p.state.ctx.db, "ws-b").await;

        let payload = active_workspace(&p.state, &HeaderMap::new(), None).await;

        assert_eq!(payload["workspace"]["id"], json!("ws-b"));
    }

    #[tokio::test]
    async fn a_scoped_client_does_not_follow_another_clients_switch() {
        let p = two_projects().await;

        // An unscoped client moves the app to project A...
        let switched = call_tool(
            &p.state,
            &HeaderMap::new(),
            None,
            "use_workspace",
            json!({ "path": p.a.to_string_lossy() }),
        )
        .await;
        assert_eq!(payload_of(&switched)["workspace"]["id"], json!("ws-a"));

        // ...and the client scoped to B is unmoved.
        let payload = active_workspace(&p.state, &scoped(&p.b), None).await;

        assert_eq!(payload["workspace"]["id"], json!("ws-b"));
    }

    #[tokio::test]
    async fn a_scoped_client_cannot_switch_workspaces() {
        let p = two_projects().await;

        let response = call_tool(
            &p.state,
            &scoped(&p.a),
            None,
            "use_workspace",
            json!({ "path": p.b.to_string_lossy() }),
        )
        .await;

        assert!(
            response["result"]["isError"].as_bool().unwrap_or(false),
            "got {response}"
        );
        let text = response["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default();
        assert!(text.contains("cannot switch workspaces"), "got {text}");
        // Named per mechanism: an HTTP client cannot act on advice about an
        // environment variable it never read.
        assert!(
            text.contains(WORKSPACE_HEADER),
            "the refusal should name the header: {text}"
        );
    }

    #[tokio::test]
    async fn a_scoped_client_sees_only_its_own_workspace() {
        let p = two_projects().await;

        let response = call_tool(&p.state, &scoped(&p.a), None, "list_workspaces", json!({})).await;

        let listed = payload_of(&response);
        let listed = listed["workspaces"].as_array().expect("array");
        assert_eq!(listed.len(), 1, "a confined client has no use for the others");
        assert_eq!(listed[0]["id"], json!("ws-a"));
    }

    #[tokio::test]
    async fn a_scoped_clients_file_tools_stop_at_its_own_project() {
        // The scope is not only a board filter: `read_file` resolves against
        // the same `workspace_root`, so a client scoped to A cannot read B.
        let p = two_projects().await;
        std::fs::write(p.a.join("mine.txt"), "a").expect("write a");
        std::fs::write(p.b.join("theirs.txt"), "b").expect("write b");
        // The app is looking at B, so a root re-derived from the database
        // would be B's — which is the bug this asserts against.
        make_active(&p.state.ctx.db, "ws-b").await;

        let mine = call_tool(
            &p.state,
            &scoped(&p.a),
            None,
            "read_file",
            json!({ "path": p.a.join("mine.txt").to_string_lossy() }),
        )
        .await;
        assert!(
            !mine["result"]["isError"].as_bool().unwrap_or(false),
            "a scoped client must still read its own files: {mine}"
        );

        let theirs = call_tool(
            &p.state,
            &scoped(&p.a),
            None,
            "read_file",
            json!({ "path": p.b.join("theirs.txt").to_string_lossy() }),
        )
        .await;
        assert!(
            theirs["result"]["isError"].as_bool().unwrap_or(false),
            "the other project's file should be out of reach: {theirs}"
        );
    }

    #[tokio::test]
    async fn a_scoped_client_still_reaches_the_live_state_tools() {
        // The reason to prefer this transport at all: these are hidden on stdio,
        // and scoping must not cost a client access to them.
        let p = two_projects().await;
        let body = br#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#;

        let (_, value) = parts(dispatch_body(body, &scoped(&p.a), None, &p.state).await).await;

        // No `HostBridge` in this fixture, so the host-only tools are absent —
        // what matters is that the scope does not change the answer.
        let (_, unscoped) =
            parts(dispatch_body(body, &HeaderMap::new(), None, &p.state).await).await;
        assert_eq!(value["result"]["tools"], unscoped["result"]["tools"]);
    }

    #[tokio::test]
    async fn an_unregistered_workspace_is_refused_on_that_request_alone() {
        let p = two_projects().await;
        let stranger = p.a.parent().expect("parent").join("not-a-workspace");

        let body = br#"{"jsonrpc":"2.0","id":7,"method":"ping","params":{}}"#;
        let (status, value) =
            parts(dispatch_body(body, &scoped(&stranger), None, &p.state).await).await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(value["error"]["code"], json!(-32602));
        assert_eq!(
            value["id"],
            json!(7),
            "the refusal should echo the request id"
        );
        let message = value["error"]["message"].as_str().unwrap_or_default();
        assert!(
            message.contains(&stranger.to_string_lossy().to_string()),
            "the refusal should name the folder: {message}"
        );

        // And only that request: the server is still serving everyone else.
        let other = active_workspace(&p.state, &scoped(&p.b), None).await;
        assert_eq!(other["workspace"]["id"], json!("ws-b"));
    }

    #[tokio::test]
    async fn the_query_parameter_scopes_a_client_that_cannot_template_a_header() {
        let p = two_projects().await;
        let query = form_urlencoded::Serializer::new(String::new())
            .append_pair(WORKSPACE_QUERY_KEY, &p.a.to_string_lossy())
            .finish();

        let payload = active_workspace(&p.state, &HeaderMap::new(), Some(&query)).await;

        assert_eq!(payload["workspace"]["id"], json!("ws-a"));
    }

    #[tokio::test]
    async fn initialize_names_the_board_the_client_is_attached_to() {
        let p = two_projects().await;
        let body = br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;

        let (_, value) = parts(dispatch_body(body, &scoped(&p.a), None, &p.state).await).await;

        let instructions = value["result"]["instructions"].as_str().unwrap_or_default();
        assert!(
            instructions.contains(&p.a.to_string_lossy().to_string()),
            "got {instructions}"
        );
        assert!(instructions.contains("confined"), "got {instructions}");
    }

    /// The header names the right folder in the wrong case, and still lands.
    ///
    /// This is the shape of the real report: `${workspaceFolder}` is whatever
    /// casing the editor holds, the row is whatever casing the app canonicalized
    /// when the user opened the folder, and on Windows those can differ while
    /// naming one directory. Refusing that told the user their own open project
    /// was not a workspace this RustyAgent had opened.
    #[tokio::test]
    async fn a_header_that_shouts_the_path_still_finds_the_project() {
        let p = two_projects().await;

        let shouted = workspace_header(&p.a.to_string_lossy().to_uppercase());
        let payload = active_workspace(&p.state, &shouted, None).await;

        // On a case-sensitive filesystem the shouted path is a different
        // directory, and the refusal it gets there is the correct answer.
        if cfg!(any(windows, target_os = "macos")) {
            assert_eq!(payload["workspace"]["id"], json!("ws-a"), "got {payload}");
        }
    }

    /// A templated path that arrives with a trailing separator still lands.
    #[tokio::test]
    async fn a_trailing_separator_in_the_header_still_finds_the_project() {
        let p = two_projects().await;

        let with_slash = workspace_header(&format!("{}/", p.b.to_string_lossy()));
        let payload = active_workspace(&p.state, &with_slash, None).await;

        assert_eq!(payload["workspace"]["id"], json!("ws-b"), "got {payload}");
    }

    #[test]
    fn the_header_wins_over_the_query_parameter() {
        // Both present is a misconfiguration rather than an attack, but it needs
        // one defined answer, and the header is the documented mechanism.
        let headers = workspace_header("C:/from-the-header");

        let requested = requested_workspace(&headers, Some("workspace=C%3A%2Ffrom-the-query"));

        assert_eq!(requested, Ok(Some("C:/from-the-header".to_string())));
    }

    #[test]
    fn a_blank_header_reads_as_absent() {
        // A client templating a variable that resolved to nothing meant "no
        // scope", not "the workspace named empty string".
        assert_eq!(requested_workspace(&workspace_header("   "), None), Ok(None));
    }

    #[test]
    fn a_percent_encoded_windows_path_survives_the_query() {
        // Why this parses the query rather than reading it raw: a real path
        // carries a drive colon, backslashes, and often a space.
        let requested = requested_workspace(
            &HeaderMap::new(),
            Some("workspace=C%3A%5CUsers%5Cmitch%5CMy%20Projects%5Cboard"),
        );

        assert_eq!(
            requested,
            Ok(Some(r"C:\Users\mitch\My Projects\board".to_string()))
        );
    }
}
