//! Streamable-HTTP transport (POST only — no SSE, so the server is stateless).

use std::{convert::Infallible, net::SocketAddr, sync::Arc, time::Duration};

use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::{
    body::Incoming,
    header::{ALLOW, AUTHORIZATION, CONTENT_TYPE, HOST, ORIGIN, WWW_AUTHENTICATE},
    server::conn::http1,
    service::service_fn,
    Method, Request, Response, StatusCode,
};
use hyper_util::rt::TokioIo;
use serde_json::{json, Value};
use tokio::net::TcpListener as TokioTcpListener;
use tracing::{error, info, warn};

use crate::{
    auth::AuthConfig,
    ctx::McpCtx,
    jsonrpc::{error_response, FORBIDDEN, INVALID_REQUEST, PARSE_ERROR, UNAUTHORIZED},
    protocol::handle_message_refreshed,
    registry::McpRegistry,
    tools::build_registry,
};

pub const MCP_ENDPOINT_PATH: &str = "/mcp";
const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;
/// Give up on the accept loop only after sustained failure.
const MAX_CONSECUTIVE_ACCEPT_FAILURES: u32 = 64;

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

    dispatch_body(&collected, &state).await
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

async fn dispatch_body(
    collected: &[u8],
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

    match handle_message_refreshed(&state.ctx, &state.registry, &message).await {
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

        let response = dispatch_body(body, &state).await;

        let (status, value) = parts(response).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["id"], json!(1));
        assert_eq!(value["result"], json!({}));
    }

    #[tokio::test]
    async fn a_notification_gets_an_empty_202() {
        let state = http_state().await;
        let body = br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;

        let response = dispatch_body(body, &state).await;

        assert_eq!(response.status(), StatusCode::ACCEPTED);
    }

    #[tokio::test]
    async fn malformed_json_is_a_parse_error_with_400() {
        let state = http_state().await;

        let response = dispatch_body(b"{not json}", &state).await;

        let (status, value) = parts(response).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(value["error"]["code"], json!(-32700));
    }

    #[tokio::test]
    async fn a_batch_request_is_rejected_with_an_explanation() {
        let state = http_state().await;

        let response = dispatch_body(b"[{\"jsonrpc\":\"2.0\",\"id\":1}]", &state).await;

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

        let response = dispatch_body(b"\"just a string\"", &state).await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
