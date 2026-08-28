//! The shared context cap for agent-facing reads.
//!
//! Two separate surfaces hand file text to a model: the internal `file_read`
//! tool ([`crate::builtin::file`]) and the outward-facing `read_file` MCP tool
//! in the `board-mcp` crate. Both append their result to somebody's
//! conversation, and both therefore need the same cap, the same `offset` /
//! `limit` semantics and the same truncation marker.
//!
//! They live here rather than being written twice on purpose. An agent that
//! learns one read tool must not be surprised by the other, and two copies of a
//! byte-slicing routine that agree today are exactly how the pair drifts apart
//! tomorrow.
//!
//! ## Why this is not the same thing as a size limit
//!
//! A *memory* guard (for example the 10 MB refusal in
//! `commands::filesystem::read_file_text`) bounds the allocation this process
//! makes. A *context* cap bounds what the consumer is asked to carry. They are
//! independent: a 500 MB file needs the memory guard, because truncating it to
//! 32 KB still means reading 500 MB first; a 9 MB file passes the memory guard
//! untouched and then floods a context window that may only be a few hundred
//! kilobytes wide. Deleting either one leaves a real hole.

use serde_json::Value;

/// Cap on the bytes a single agent-facing read returns.
///
/// Mirrors `shell::MAX_OUTPUT_BYTES`, and for the same reason: tool output is
/// appended to the conversation and re-sent on every subsequent turn, so one
/// unbounded read of a lockfile, a minified bundle or a log can exhaust the
/// context window by itself. The value is deliberately the same 32 KB, so the
/// codebase has a single number for "more tool output than a turn can carry".
pub const MAX_READ_BYTES: usize = 32 * 1024;

/// Split `content` into lines that still carry their original terminator.
///
/// `str::lines` drops the terminator, and re-joining with "\n" would hand back
/// a CRLF file as LF. That is not cosmetic: the text the model reads here is
/// the text it quotes back as `file_edit`'s `old_string`, so a silent CRLF to
/// LF conversion on the way out guarantees the byte-exact match on the way in
/// will fail. `split_inclusive` keeps the bytes exactly as they are on disk.
pub fn lines_with_endings(content: &str) -> Vec<&str> {
    content.split_inclusive('\n').collect()
}

/// Read an optional 1-based positive integer parameter.
///
/// Shared by every paging tool so that "0" and "-1" are rejected identically
/// wherever an `offset` or `limit` is accepted, rather than one surface
/// silently treating 0 as 1.
pub fn optional_positive(input: &Value, key: &str) -> Result<Option<usize>, String> {
    match input.get(key) {
        None | Some(Value::Null) => Ok(None),
        // `try_from` rather than `as`: a value beyond `usize` would wrap, and a
        // line number that silently becomes a small one reads a different part
        // of the file than was asked for. Out of range is rejected like any
        // other invalid value.
        Some(v) => match v.as_u64().and_then(|n| usize::try_from(n).ok()) {
            Some(n) if n >= 1 => Ok(Some(n)),
            _ => Err(format!(
                "Parameter '{key}' must be a positive integer (counting from 1); got {v}."
            )),
        },
    }
}

/// Largest index `<= max` that is a UTF-8 character boundary in `s`.
///
/// Slicing a `str` at a fixed byte offset panics when the offset falls
/// mid-codepoint, which any file containing non-ASCII text can trigger.
pub fn floor_char_boundary(s: &str, max: usize) -> usize {
    let mut cut = max.min(s.len());
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    cut
}

/// Cap `s` at `max` bytes on a character boundary. Returns `None` when `s`
/// already fits.
///
/// The line-unaware form, for text that is not source code — a JSON blob held
/// in a database column, say — where there is no line for the reader to quote
/// back and nothing is gained by retreating to one.
pub fn truncate_at_char_boundary(s: &str, max: usize) -> Option<&str> {
    if s.len() <= max {
        return None;
    }
    Some(&s[..floor_char_boundary(s, max)])
}

/// Cap `body` at `max` bytes, cutting on a line boundary when there is one.
/// Returns `None` when `body` already fits.
///
/// The cut is moved back to the last newline inside the cap deliberately: half
/// a line handed to the model is half a line it will later quote back as an
/// `old_string` that does not exist on disk. A file with no newline at all in
/// its first 32 KB — a minified bundle — has no line boundary to find, so the
/// cut falls back to the nearest character boundary.
pub fn truncate_for_read(body: &str, max: usize) -> Option<&str> {
    let head = truncate_at_char_boundary(body, max)?;
    let cut = match head.rfind('\n') {
        Some(i) => i + 1,
        None => head.len(),
    };
    // Never end on the `\r` of a CRLF pair. With no newline inside the cap —
    // one very long CRLF-terminated first line — the fallback cut can land
    // between the two bytes, producing a lone `\r` that is not a line ending
    // anywhere on disk. That contradicts the byte-identical guarantee this
    // function exists to keep, so back off the orphaned carriage return.
    let cut = if body[..cut].ends_with('\r') && body[cut..].starts_with('\n') {
        cut - 1
    } else {
        cut
    };
    Some(&body[..cut])
}

/// One capped page of a text file: the bytes to show, plus everything a caller
/// needs to tell a reader what it is holding.
#[derive(Debug, Clone)]
pub struct ReadPage {
    /// The body, followed by a truncation or partial marker whenever this is
    /// not the whole file. Byte-identical to the file over the shown range.
    pub text: String,
    /// True when the cap cut the requested range short.
    pub truncated: bool,
    /// True whenever `text` is anything less than the complete file — either
    /// because the cap cut it or because `offset` / `limit` narrowed it.
    pub partial: bool,
    /// 1-based line number of the first line in `text`.
    pub first_line: usize,
    /// 1-based line number of the last line in `text`.
    pub last_line: usize,
    /// Lines in the whole file.
    pub total_lines: usize,
    /// Bytes in the whole file.
    pub total_bytes: usize,
    /// The `offset` to pass back to fetch what follows, or `None` at the end
    /// of the file.
    pub next_offset: Option<usize>,
}

/// Select a 1-based line range of `content`, cap it at [`MAX_READ_BYTES`], and
/// append the marker that tells the reader what it is missing.
///
/// `tool_name` names the tool in the marker so the reader knows which tool to
/// call again; `label` is the path as the caller asked for it, so the marker
/// quotes back a path the caller can actually re-send.
///
/// Returns `Err` only when `offset` points past the end of the file.
pub fn read_page(
    content: &str,
    offset: Option<usize>,
    limit: Option<usize>,
    tool_name: &str,
    label: &str,
) -> Result<ReadPage, String> {
    let lines = lines_with_endings(content);
    let total_lines = lines.len();
    let total_bytes = content.len();

    // Select the requested line range as a byte slice of the original
    // content, so what comes back is byte-identical to what is on disk.
    let first_line = offset.unwrap_or(1);
    // `.max(1)` so that an empty file reads back as empty rather than as an
    // out-of-range error: it has no line 1, but asking for line 1 of it is
    // not a mistake.
    if first_line > total_lines.max(1) {
        return Err(format!(
            "Parameter 'offset' is {first_line} but '{label}' has {total_lines} line(s)."
        ));
    }
    let last_line = match limit {
        Some(n) => (first_line - 1 + n).min(total_lines),
        None => total_lines,
    };
    let start: usize = lines[..first_line - 1].iter().map(|l| l.len()).sum();
    let end: usize =
        start + lines[first_line - 1..last_line].iter().map(|l| l.len()).sum::<usize>();
    let body = &content[start..end];

    let (shown, truncated) = match truncate_for_read(body, MAX_READ_BYTES) {
        Some(head) => (head, true),
        None => (body, false),
    };
    let shown_lines = lines_with_endings(shown).len();
    // An empty file has no lines, so the last line shown is the one before the
    // first — not the first. Without this an empty file reported
    // `last_line: 1, total_lines: 0`, which is not a range that exists.
    let last_shown = if shown_lines == 0 {
        first_line.saturating_sub(1)
    } else {
        first_line + shown_lines - 1
    };
    let whole_file = !truncated && first_line == 1 && last_line == total_lines;

    if whole_file {
        return Ok(ReadPage {
            text: shown.to_string(),
            truncated: false,
            partial: false,
            first_line,
            last_line: last_shown,
            total_lines,
            total_bytes,
            next_offset: None,
        });
    }

    // Anything short of the whole file carries a marker. It is bracketed,
    // prefixed with the tool name and phrased as a statement about the read
    // rather than about the subject matter, so it cannot be mistaken for a
    // line of the file.
    let mut out = String::with_capacity(shown.len() + 256);
    out.push_str(shown);
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    if truncated {
        // What is left to fetch, which is what the reader is about to act
        // on — not `total_bytes - shown.len()`. With an `offset`, `shown`
        // starts at byte `start`, so that form counts the skipped prefix as
        // outstanding and overstates the remainder by exactly the bytes
        // already behind the reader.
        let remaining = total_bytes.saturating_sub(start + shown.len());
        out.push_str(&format!(
            "\n[{tool_name} TRUNCATED: the text above is NOT the complete file. \
             '{label}' is {total_bytes} bytes / {total_lines} lines; this reply \
             carries lines {first_line}-{last_shown} ({} bytes) and {remaining} bytes \
             remain after it. Call {tool_name} again with \"offset\": {} to continue.]",
            shown.len(),
            last_shown + 1,
        ));
    } else {
        out.push_str(&format!(
            "\n[{tool_name} PARTIAL: the text above is lines {first_line}-{last_shown} of \
             {total_lines} in '{label}', not the complete file.]"
        ));
    }

    Ok(ReadPage {
        text: out,
        truncated,
        partial: true,
        first_line,
        last_line: last_shown,
        total_lines,
        total_bytes,
        next_offset: if last_shown < total_lines {
            Some(last_shown + 1)
        } else {
            None
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn floor_char_boundary_retreats_out_of_a_multibyte_codepoint() {
        // "€" is three bytes; only 0 and 3 are boundaries.
        let s = "\u{20AC}\u{20AC}";
        assert_eq!(floor_char_boundary(s, 1), 0);
        assert_eq!(floor_char_boundary(s, 2), 0);
        assert_eq!(floor_char_boundary(s, 3), 3);
        assert_eq!(floor_char_boundary(s, 5), 3);
        assert_eq!(floor_char_boundary(s, 99), 6, "clamps to the string length");
    }

    #[test]
    fn truncate_for_read_returns_none_when_the_body_already_fits() {
        assert!(truncate_for_read("short\n", MAX_READ_BYTES).is_none());
        // Exactly at the cap is not over it.
        let exact = "x".repeat(MAX_READ_BYTES);
        assert!(truncate_for_read(&exact, MAX_READ_BYTES).is_none());
    }

    #[test]
    fn truncate_for_read_retreats_to_the_last_newline() {
        let body = "aaaa\nbbbb\ncccc\n";
        assert_eq!(truncate_for_read(body, 12), Some("aaaa\nbbbb\n"));
    }

    #[test]
    fn truncate_for_read_falls_back_to_a_char_boundary_with_no_newline() {
        // A minified-bundle shape: no newline, multi-byte codepoints straddling
        // the cut. Slicing at a fixed offset would panic here.
        let body = "\u{20AC}".repeat(10);
        assert_eq!(truncate_for_read(&body, 8), Some("\u{20AC}\u{20AC}"));
    }

    #[test]
    fn truncate_for_read_preserves_crlf_terminators() {
        // The cut lands after the "\n" of a CRLF pair, never between the two
        // bytes: a returned "\r" with its "\n" shorn off would be quoted back
        // as text that is not on disk.
        let body = "aaaa\r\nbbbb\r\ncccc\r\n";
        assert_eq!(truncate_for_read(body, 13), Some("aaaa\r\nbbbb\r\n"));
    }

    #[test]
    fn optional_positive_rejects_zero_and_non_integers() {
        assert_eq!(optional_positive(&json!({}), "offset"), Ok(None));
        assert_eq!(optional_positive(&json!({ "offset": null }), "offset"), Ok(None));
        assert_eq!(optional_positive(&json!({ "offset": 3 }), "offset"), Ok(Some(3)));
        assert!(optional_positive(&json!({ "offset": 0 }), "offset").is_err());
        assert!(optional_positive(&json!({ "offset": -1 }), "offset").is_err());
        assert!(optional_positive(&json!({ "offset": "3" }), "offset").is_err());
    }

    #[test]
    fn read_page_returns_a_fitting_file_verbatim_with_no_marker() {
        let page = read_page("alpha\nbravo\n", None, None, "read_file", "a.txt").expect("page");
        assert_eq!(page.text, "alpha\nbravo\n");
        assert!(!page.partial);
        assert!(!page.truncated);
        assert_eq!(page.next_offset, None);
    }

    #[test]
    fn read_page_marker_names_the_calling_tool() {
        let content = "x".repeat(MAX_READ_BYTES * 2);
        let page = read_page(&content, None, None, "read_file", "big.js").expect("page");
        assert!(page.truncated);
        assert!(page.text.contains("[read_file TRUNCATED:"), "got {}", &page.text[page.text.len() - 300..]);
        assert!(!page.text.contains("file_read"), "the marker must name its own tool");
    }

    #[test]
    fn read_page_reports_the_offset_to_resume_from() {
        // 2000 lines of 20 bytes = 40000 bytes, past the 32768 cap.
        let content: String = (0..2000).map(|i| format!("{i:019}\n")).collect();
        assert_eq!(content.len(), 2000 * 20, "line width assumption broke");
        let page = read_page(&content, None, None, "read_file", "big.txt").expect("page");
        let expected_lines = MAX_READ_BYTES / 20;
        assert_eq!(page.last_line, expected_lines);
        assert_eq!(page.next_offset, Some(expected_lines + 1));
        assert!(page
            .text
            .contains(&format!("\"offset\": {}", expected_lines + 1)));
        // Resuming from there really does continue where the first page ended.
        let next = read_page(&content, page.next_offset, None, "read_file", "big.txt")
            .expect("page");
        assert!(next.text.starts_with(&content[expected_lines * 20..][..20]));
    }

    #[test]
    fn read_page_rejects_an_offset_past_the_end() {
        let err = read_page("one\ntwo\n", Some(9), None, "read_file", "a.txt")
            .expect_err("out of range");
        assert!(err.contains("has 2 line(s)"), "got {err}");
    }

    #[test]
    fn read_page_accepts_line_one_of_an_empty_file() {
        let page = read_page("", Some(1), None, "read_file", "empty.txt").expect("page");
        assert_eq!(page.text, "");
        assert!(!page.partial);
    }

    #[test]
    fn read_page_limit_without_truncation_is_partial_not_truncated() {
        let page =
            read_page("a\nb\nc\nd\n", Some(2), Some(2), "read_file", "a.txt").expect("page");
        assert!(page.partial);
        assert!(!page.truncated);
        assert!(page.text.starts_with("b\nc\n"));
        assert!(page.text.contains("[read_file PARTIAL: the text above is lines 2-3 of 4"));
        assert_eq!(page.next_offset, Some(4));
    }

    #[test]
    fn read_page_at_the_final_line_reports_no_next_offset() {
        let page = read_page("a\nb\nc\n", Some(3), None, "read_file", "a.txt").expect("page");
        assert!(page.partial, "it is still not the whole file");
        assert_eq!(page.next_offset, None);
    }

    /// A cut landing between the two bytes of a CRLF must keep neither.
    ///
    /// Reachable when the first newline is past the cap — one very long
    /// CRLF-terminated line — so the newline fallback has nothing to find and
    /// the cut lands on the carriage return. A lone CR is not a line ending
    /// anywhere on disk, and returning one breaks the byte-identical guarantee
    /// the shown range is supposed to keep.
    #[test]
    fn a_cut_between_a_carriage_return_and_its_newline_keeps_neither() {
        let body = format!("{}\r\nsecond line\r\n", "x".repeat(9));

        let shown = truncate_for_read(&body, 10).expect("over the cap");

        assert!(
            !shown.ends_with('\r'),
            "kept an orphaned carriage return: {shown:?}"
        );
        assert_eq!(shown, "x".repeat(9));
    }

    /// An empty file has no line 1, so it cannot be the last line shown.
    #[test]
    fn an_empty_file_reports_a_line_range_that_exists() {
        let page = read_page("", None, None, "file_read", "empty.txt").expect("page");

        assert_eq!(page.total_lines, 0);
        assert_eq!(page.last_line, 0, "reported a line the file does not have");
        assert!(!page.truncated);
    }

    /// An absurd offset fails loudly, never as a wrapped small line number.
    ///
    /// `usize::try_from` replaced an `as usize` cast that would truncate above
    /// `usize::MAX`. On a 64-bit target the two are the same width so nothing
    /// is lost either way — the cast was only lossy on 32-bit, which this app
    /// does not ship. What is worth pinning on every target is the outcome the
    /// cast was risking: a caller asking for an impossible line is told so,
    /// rather than silently handed a different part of the file.
    #[test]
    fn an_absurd_offset_is_refused_rather_than_silently_reinterpreted() {
        let error = read_page("one\ntwo\n", Some(usize::MAX), None, "file_read", "f.txt")
            .expect_err("an offset past the end must fail");

        assert!(error.contains("offset"), "got {error}");
        assert!(error.contains("2 line"), "should name the real length: {error}");
    }
}
