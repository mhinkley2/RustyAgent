//! Newline-delimited JSON transport, per the MCP stdio spec.
//!
//! Each message is one line; embedded newlines are escaped by the JSON encoder,
//! so a message never spans lines. The previous implementation used LSP-style
//! `Content-Length` headers, which no MCP client sends — the server would block
//! forever waiting for a `\r\n\r\n` that never arrived.
//!
//! Nothing but framed messages may be written to stdout. Diagnostics go to
//! stderr; a tracing subscriber installed in a binary using this transport must
//! be pinned to stderr or it will corrupt the stream.

use std::io::{self, BufRead, Write};

use serde_json::Value;

use crate::{
    ctx::McpCtx,
    jsonrpc::{error_response, PARSE_ERROR},
    protocol::handle_message_refreshed,
    registry::McpRegistry,
};

/// Read one message.
///
/// `Ok(None)` is a clean EOF. `Ok(Some(Err(_)))` is a malformed line, which the
/// caller answers with a parse error and then *keeps reading* — one bad line
/// must not take the server down. Only genuine I/O failures surface as `Err`.
pub fn read_message<R: BufRead>(reader: &mut R) -> io::Result<Option<Result<Value, String>>> {
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue; // tolerate blank lines and bare \r\n keep-alives
        }

        return Ok(Some(
            serde_json::from_str::<Value>(trimmed)
                .map_err(|error| format!("Invalid JSON payload: {error}")),
        ));
    }
}

/// Write one message as a single newline-terminated line.
///
/// `to_writer` is compact and escapes newlines inside strings, so the output is
/// guaranteed to contain no raw newline. Never switch this to
/// `to_string_pretty` — it would split one message across many lines.
pub fn write_message<W: Write>(writer: &mut W, body: &Value) -> io::Result<()> {
    serde_json::to_writer(&mut *writer, body)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    writer.write_all(b"\n")?;
    writer.flush()
}

/// Serve MCP over the given reader/writer until EOF or an `exit` notification.
pub async fn serve<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    ctx: McpCtx,
    registry: &McpRegistry,
) -> io::Result<()> {
    while let Some(parsed) = read_message(reader)? {
        let message = match parsed {
            Ok(message) => message,
            Err(error) => {
                // Report and continue; the next line may be perfectly valid.
                write_message(writer, &error_response(Value::Null, PARSE_ERROR, error))?;
                continue;
            }
        };

        if message.get("method").and_then(Value::as_str) == Some("exit") {
            return Ok(());
        }

        if let Some(response) = handle_message_refreshed(&ctx, registry, &message).await {
            write_message(writer, &response)?;
        }
    }

    Ok(())
}
