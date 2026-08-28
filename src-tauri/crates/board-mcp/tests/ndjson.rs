//! Newline-delimited JSON framing.
//!
//! The previous transport used LSP-style `Content-Length` headers, which no MCP
//! client sends — the server blocked forever on a terminator that never came.
//! These tests pin the wire format so that cannot regress.

mod common;

use std::io::{BufReader, Cursor};

use board_mcp::transport::stdio::{read_message, serve, write_message};
use common::*;
use serde_json::{json, Value};

fn write_to_string(value: &Value) -> String {
    let mut buffer = Vec::new();
    write_message(&mut buffer, value).expect("write");
    String::from_utf8(buffer).expect("utf-8")
}

#[test]
fn a_message_is_written_as_one_newline_terminated_line() {
    let out = write_to_string(&json!({ "jsonrpc": "2.0", "id": 1, "result": {} }));

    assert!(out.ends_with('\n'));
    assert_eq!(out.matches('\n').count(), 1, "exactly one newline");
    assert!(!out.contains("Content-Length"), "must not be LSP-framed");
}

#[test]
fn a_string_containing_a_newline_still_produces_one_line() {
    // The guarantee that makes NDJSON safe: the encoder escapes the newline
    // rather than emitting it raw. Pretty-printing here would break framing.
    let out = write_to_string(&json!({ "text": "line one\nline two" }));

    assert_eq!(out.matches('\n').count(), 1, "got {out:?}");
    assert!(out.contains(r"line one\nline two"), "got {out:?}");
}

#[test]
fn reading_returns_each_message_in_order() {
    let input = "{\"id\":1}\n{\"id\":2}\n{\"id\":3}\n";
    let mut reader = BufReader::new(Cursor::new(input));

    let mut ids = Vec::new();
    while let Some(parsed) = read_message(&mut reader).expect("read") {
        ids.push(parsed.expect("valid json")["id"].as_i64().unwrap());
    }

    assert_eq!(ids, vec![1, 2, 3]);
}

#[test]
fn blank_lines_and_carriage_returns_are_tolerated() {
    let input = "\n\r\n{\"id\":1}\r\n   \n{\"id\":2}\n";
    let mut reader = BufReader::new(Cursor::new(input));

    let mut ids = Vec::new();
    while let Some(parsed) = read_message(&mut reader).expect("read") {
        ids.push(parsed.expect("valid json")["id"].as_i64().unwrap());
    }

    assert_eq!(ids, vec![1, 2]);
}

#[test]
fn end_of_input_reports_none() {
    let mut reader = BufReader::new(Cursor::new(""));

    assert!(read_message(&mut reader).expect("read").is_none());
}

#[test]
fn a_malformed_line_does_not_stop_the_next_valid_one_from_parsing() {
    // One bad byte must not take the server down.
    let input = "{not json}\n{\"id\":7}\n";
    let mut reader = BufReader::new(Cursor::new(input));

    let first = read_message(&mut reader).expect("read").expect("a message");
    assert!(first.is_err(), "the malformed line should report an error");

    let second = read_message(&mut reader).expect("read").expect("a message");
    assert_eq!(second.expect("valid json")["id"], json!(7));
}

#[test]
fn a_round_trip_survives_the_framing() {
    let original = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": { "content": [{ "type": "text", "text": "a\nb\tc \"quoted\"" }] }
    });

    let encoded = write_to_string(&original);
    let mut reader = BufReader::new(Cursor::new(encoded));
    let decoded = read_message(&mut reader)
        .expect("read")
        .expect("a message")
        .expect("valid json");

    assert_eq!(decoded, original);
}

// -- end-to-end over the transport -------------------------------------------

#[tokio::test]
async fn serve_answers_requests_and_skips_notifications() {
    let ctx = ctx().await;
    let registry = registry();

    let input = concat!(
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n",
        "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\",\"params\":{}}\n",
    );
    let mut reader = BufReader::new(Cursor::new(input));
    let mut output = Vec::new();

    serve(&mut reader, &mut output, ctx, &registry)
        .await
        .expect("serve");

    let lines: Vec<&str> = std::str::from_utf8(&output)
        .expect("utf-8")
        .lines()
        .collect();
    // Two requests answered; the notification produced nothing.
    assert_eq!(lines.len(), 2, "got {lines:?}");
    assert_eq!(
        serde_json::from_str::<Value>(lines[0]).unwrap()["id"],
        json!(1)
    );
    assert_eq!(
        serde_json::from_str::<Value>(lines[1]).unwrap()["id"],
        json!(2)
    );
}

#[tokio::test]
async fn serve_reports_a_parse_error_and_keeps_going() {
    let ctx = ctx().await;
    let registry = registry();

    let input = concat!(
        "{oops\n",
        "{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"ping\",\"params\":{}}\n",
    );
    let mut reader = BufReader::new(Cursor::new(input));
    let mut output = Vec::new();

    serve(&mut reader, &mut output, ctx, &registry)
        .await
        .expect("serve");

    let lines: Vec<&str> = std::str::from_utf8(&output)
        .expect("utf-8")
        .lines()
        .collect();
    assert_eq!(lines.len(), 2);

    let error: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(error["error"]["code"], json!(-32700));

    let pong: Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(pong["id"], json!(9));
}

#[tokio::test]
async fn serve_stops_on_an_exit_notification() {
    let ctx = ctx().await;
    let registry = registry();

    let input = concat!(
        "{\"jsonrpc\":\"2.0\",\"method\":\"exit\"}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\",\"params\":{}}\n",
    );
    let mut reader = BufReader::new(Cursor::new(input));
    let mut output = Vec::new();

    serve(&mut reader, &mut output, ctx, &registry)
        .await
        .expect("serve");

    assert!(output.is_empty(), "nothing after exit should be served");
}

#[tokio::test]
async fn serve_hides_host_only_tools_on_stdio() {
    let ctx = ctx().await;
    let registry = registry();

    let input = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\",\"params\":{}}\n";
    let mut reader = BufReader::new(Cursor::new(input));
    let mut output = Vec::new();

    serve(&mut reader, &mut output, ctx, &registry)
        .await
        .expect("serve");

    let text = String::from_utf8(output).expect("utf-8");
    assert!(text.contains("list_stories"));
    assert!(
        !text.contains("get_agent_runtime_status"),
        "host-only tools must not be advertised over stdio"
    );
}
