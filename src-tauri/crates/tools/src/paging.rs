//! Bounding list-shaped tool responses.
//!
//! These read tools answer into an agent's context — Claude Code over MCP, an
//! editor, or one of this app's own runtimes — whose budget the caller cannot
//! see and cannot spend twice. A response that returns "every row" is therefore
//! not a convenience but a defect: one long autonomous run's event log, with
//! the full `tool_input` and `tool_output` of every call in it, is larger than
//! any source file in the repository.
//!
//! Lives beside the tools rather than in `board-mcp` because the bound is not a
//! property of the wire protocol. An internal agent has the same finite context
//! an external one does, and `list_stories` is read by both.
//!
//! Two independent limits apply, for the same reason they do on the file read
//! path (see [`crate::read_cap`]):
//!
//! * a **row** limit, so the reply carries a countable number of records; and
//! * a **byte** budget, because rows are not the same size — a hundred token
//!   events are nothing and a single `tool_output` can be megabytes.
//!
//! A row cap alone would not bound bytes and a byte budget alone would not
//! bound the work, so both are enforced, and both are reported back in the
//! envelope so the caller can page.

use serde::Serialize;
use serde_json::{json, Map, Value};
use crate::read_cap::{floor_char_boundary, optional_positive, MAX_READ_BYTES};

/// Bytes of serialized JSON a single response's payload may carry.
///
/// The same 32 KB the file read path uses: it is this codebase's one number
/// for "more tool output than a turn can carry".
///
/// What this does and does not bound, stated exactly, because the wire form is
/// not the payload form. An MCP result carries the payload **twice** — once as
/// the model-visible `content` text and once as `structuredContent` for
/// programs (`registry::tool_output_to_result`); both are part of the protocol
/// and neither can be dropped. So a page built to this budget yields roughly
/// `2x` this many bytes on the wire, while the *model-visible* text stays at
/// about this figure. Context flooding is what the cap exists to prevent, and
/// that is the model-visible half — but a caller measuring the whole frame
/// should expect twice the number, and tool descriptions should not promise
/// otherwise.
pub const MAX_PAGE_BYTES: usize = MAX_READ_BYTES;

/// What a reader whose value was cut can do about it.
///
/// A truncation marker that stops at "this was cut" tells an agent only that it
/// is missing something. For most capped fields — a tool result, a chat message
/// — there is genuinely no fuller form on this surface, and saying so is the
/// honest end of the sentence.
///
/// A story's `description` is the exception: `get_story` returns it whole, so
/// that marker names the call instead. Hence the parameter.
pub const NO_FULLER_FORM: &str =
    "The full value is not available over MCP — view it in the RustyAgent app.";

/// Bytes any single free-text column may carry.
///
/// Applied before the byte budget, so that one enormous `tool_output` costs a
/// page a bounded amount rather than consuming it whole and starving every
/// other row on it.
pub const MAX_FIELD_BYTES: usize = 4 * 1024;

/// Bytes reserved from [`MAX_PAGE_BYTES`] for everything that is not a row.
///
/// The envelope's keys and numbers, the array brackets, and the `notice` — the
/// largest single part, since it names the tool, the subject and three counts.
/// Reserving generously costs at most one row on a full page; not reserving at
/// all means the advertised budget is not the figure the client receives.
const ENVELOPE_OVERHEAD_BYTES: usize = 512;

/// A validated `offset` / `limit` pair, 1-based like the file read path.
#[derive(Debug, Clone, Copy)]
pub struct PageRequest {
    /// 1-based index of the first row to return.
    pub offset: usize,
    /// Maximum rows to return, already clamped to the tool's documented
    /// ceiling.
    pub limit: usize,
}

/// Parse `offset` and `limit` from a tool's arguments.
///
/// An absent `limit` means `default_limit`, not "everything": the whole point
/// is that a caller who does not think about paging still gets a bounded
/// reply. A `limit` above `max_limit` is clamped rather than rejected, so a
/// caller asking for more than the ceiling gets the ceiling and is told so by
/// the envelope instead of an error it has to handle.
pub fn page_request(
    input: &Value,
    default_limit: usize,
    max_limit: usize,
) -> Result<PageRequest, String> {
    let offset = optional_positive(input, "offset")?.unwrap_or(1);
    let limit = optional_positive(input, "limit")?
        .unwrap_or(default_limit)
        .min(max_limit);
    Ok(PageRequest { offset, limit })
}

/// Cap the named string fields of a serialized row in place.
///
/// Cuts on a UTF-8 character boundary — a row holding a tool result full of
/// box-drawing characters or CJK text is ordinary, and slicing at a fixed byte
/// offset would panic on it. The marker names the tool and the field so the
/// reader can tell a value that was shortened from one that was short, and ends
/// with `remedy` so it also says what to do about it — see [`NO_FULLER_FORM`].
pub fn cap_text_fields(row: &mut Value, fields: &[&str], tool_name: &str, remedy: &str) {
    let Some(object) = row.as_object_mut() else {
        return;
    };
    for field in fields {
        let Some(Value::String(text)) = object.get(*field) else {
            continue;
        };
        if text.len() <= MAX_FIELD_BYTES {
            continue;
        }
        let full = text.len();
        let cut = floor_char_boundary(text, MAX_FIELD_BYTES);
        // The remedy is the caller's to supply: this helper caps tool results,
        // chat messages, directory entries and story descriptions, and only the
        // last of those has another call that returns it whole. A marker that
        // named the wrong thing would send the reader somewhere their value is
        // not.
        let capped = format!(
            "{}\n[{tool_name} FIELD TRUNCATED: '{field}' is {full} bytes; the first {cut} are \
             shown. {remedy}]",
            &text[..cut]
        );
        object.insert((*field).to_string(), Value::String(capped));
    }
}

/// One page of rows, wrapped in an envelope that says what it is.
///
/// The envelope is the whole point: a bare JSON array cannot say "there are
/// four hundred more of these", so an external client that received one would
/// have no way to tell a complete answer from a clipped one. Every response
/// carries `total`, `returned` and `complete`, so the distinction is explicit
/// even when nothing was cut.
///
/// `rows` must already be serialized and field-capped. Returns `Err` when
/// `offset` points past the end.
pub fn page_envelope(
    rows: Vec<Value>,
    request: PageRequest,
    tool_name: &str,
    item_key: &str,
    subject: &str,
) -> Result<Value, String> {
    let total = rows.len();
    let start = request.offset.saturating_sub(1);
    let window: Vec<Value> = rows.into_iter().skip(start).collect();
    page_envelope_of(window, total, request, tool_name, item_key, subject)
}

/// As [`page_envelope`], but taking rows already narrowed to the window that
/// starts at `request.offset`, plus the `total` there were before narrowing.
///
/// Splitting it this way is what lets [`paged_rows`] serialize and field-cap
/// one page rather than the entire list: a caller that already knows the
/// window does not have to materialise four hundred rows to return fifty.
///
/// Public for callers whose rows are not [`Serialize`] and so cannot use
/// [`paged_rows`] — `list_stories` builds its own from `sqlx` rows. Such a
/// caller owns the two steps `paged_rows` would have done for it: narrowing to
/// `request.offset`/`request.limit` before converting anything, and calling
/// [`cap_text_fields`] on the window it produced.
pub fn page_envelope_of(
    rows: Vec<Value>,
    total: usize,
    request: PageRequest,
    tool_name: &str,
    item_key: &str,
    subject: &str,
) -> Result<Value, String> {
    // `.max(1)` so that asking for row 1 of an empty list is an empty page
    // rather than an error: there is no row 1, but wanting it is not a mistake.
    if request.offset > total.max(1) {
        return Err(format!(
            "Parameter 'offset' is {} but {subject} has {total} row(s).",
            request.offset
        ));
    }

    let start = request.offset - 1;
    let mut page: Vec<Value> = Vec::new();
    // The budget is spent on the whole reply, not just the rows in it.
    //
    // Summing only `to_string(&row)` ignores the separating commas, the
    // enclosing brackets, and the envelope's own keys and `notice` — so a page
    // that measured exactly at the limit still went over it on the wire, and
    // the 32 KB this module advertises was never the number a client received.
    let mut bytes = ENVELOPE_OVERHEAD_BYTES;
    for row in rows.into_iter().take(request.limit) {
        // `+ 1` for the comma or closing bracket this row brings with it.
        let size = serde_json::to_string(&row).map(|s| s.len()).unwrap_or(0) + 1;
        // Always emit the first row even when it alone busts the budget.
        // Otherwise a single oversized row would return an empty page whose
        // `next_offset` pointed back at itself, and a paging client would spin
        // there forever.
        if !page.is_empty() && bytes + size > MAX_PAGE_BYTES {
            break;
        }
        bytes += size;
        page.push(row);
    }

    let returned = page.len();
    let consumed = start + returned;
    let next_offset = (consumed < total).then_some(consumed + 1);
    let complete = request.offset == 1 && next_offset.is_none();

    let mut envelope = Map::new();
    envelope.insert(item_key.to_string(), Value::Array(page));
    envelope.insert("offset".into(), json!(request.offset));
    // The limit actually in force, which is not necessarily the one asked for:
    // an over-ceiling `limit` is clamped rather than rejected. Without this the
    // caller could not tell a clamp from a short page, because `returned` is
    // also reduced by the byte budget and by simply running out of rows.
    envelope.insert("limit".into(), json!(request.limit));
    envelope.insert("returned".into(), json!(returned));
    envelope.insert("total".into(), json!(total));
    envelope.insert("complete".into(), json!(complete));
    envelope.insert("next_offset".into(), json!(next_offset));
    if let Some(next) = next_offset {
        envelope.insert(
            "notice".into(),
            json!(format!(
                "[{tool_name} PARTIAL: this is NOT the complete list. {subject} has {total} \
                 {item_key}; this reply carries {}-{consumed}. Call {tool_name} again with \
                 \"offset\": {next} to continue.]",
                request.offset
            )),
        );
    }
    Ok(Value::Object(envelope))
}

/// Serialize, field-cap, and page a list of rows in one step.
///
/// `remedy` ends any truncation marker this produces; pass [`NO_FULLER_FORM`]
/// unless another tool can return the capped field in full.
pub fn paged_rows<T: Serialize>(
    rows: Vec<T>,
    request: PageRequest,
    tool_name: &str,
    item_key: &str,
    subject: &str,
    text_fields: &[&str],
    remedy: &str,
) -> Result<Value, String> {
    let total = rows.len();
    // Narrow to the requested window *before* serializing and field-capping.
    //
    // The rows arrive already fetched — the underlying command is deliberately
    // unpaged, because the app's own detail views read it and want the whole
    // log. Converting all of them to `Value` and capping every text field to
    // return fifty would double the in-memory footprint of the entire run for
    // no benefit, which is the opposite of what capping is for.
    let start = request.offset.saturating_sub(1);

    // Refuse an out-of-range offset up front, on the two numbers it depends on.
    //
    // The saving is smaller than it looks and worth stating honestly: this
    // function owns `rows`, so every element is dropped when it returns either
    // way, and on this path nothing was serialized regardless — `take` yields
    // nothing once `skip` has exhausted the iterator. What it avoids is the
    // element-by-element walk `skip` performs to get there. It is kept mainly
    // because failing on a comparison, before building anything, is the clearer
    // shape for a precondition.
    if request.offset > total.max(1) {
        return Err(format!(
            "Parameter 'offset' is {} but {subject} has {total} row(s).",
            request.offset
        ));
    }

    let window = rows.into_iter().skip(start).take(request.limit);

    let mut values = Vec::with_capacity(request.limit.min(total.saturating_sub(start)));
    for row in window {
        let mut value =
            serde_json::to_value(row).map_err(|error| format!("Failed to serialize: {error}"))?;
        if !text_fields.is_empty() {
            cap_text_fields(&mut value, text_fields, tool_name, remedy);
        }
        values.push(value);
    }
    page_envelope_of(values, total, request, tool_name, item_key, subject)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(offset: usize, limit: usize) -> PageRequest {
        PageRequest { offset, limit }
    }

    fn rows(n: usize) -> Vec<Value> {
        (0..n).map(|i| json!({ "i": i })).collect()
    }

    /// Only the requested window is converted and field-capped.
    ///
    /// The rows arrive already fetched, so serializing all of them to return a
    /// page doubles the in-memory footprint of the whole list — for a long run's
    /// event log, exactly the flooding this module exists to prevent. The
    /// counter is owned by the test rather than a static, so it cannot race
    /// another test in the same binary.
    #[test]
    fn only_the_requested_window_is_serialized() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        struct Counted(usize, Arc<AtomicUsize>);
        impl Serialize for Counted {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                self.1.fetch_add(1, Ordering::SeqCst);
                s.serialize_u64(self.0 as u64)
            }
        }

        let seen = Arc::new(AtomicUsize::new(0));
        let rows: Vec<Counted> =
            (0..1_000).map(|i| Counted(i, Arc::clone(&seen))).collect();

        let envelope =
            paged_rows(rows, req(1, 10), "t", "items", "the list", &[], NO_FULLER_FORM).expect("page");

        assert_eq!(envelope["returned"], json!(10));
        assert_eq!(envelope["total"], json!(1_000));
        assert_eq!(
            seen.load(Ordering::SeqCst),
            10,
            "serialized the whole list to return ten rows"
        );
    }

    /// The same holds partway in: a window at an offset converts only its own
    /// rows, not everything before it.
    #[test]
    fn a_window_at_an_offset_does_not_serialize_what_precedes_it() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        struct Counted(usize, Arc<AtomicUsize>);
        impl Serialize for Counted {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                self.1.fetch_add(1, Ordering::SeqCst);
                s.serialize_u64(self.0 as u64)
            }
        }

        let seen = Arc::new(AtomicUsize::new(0));
        let rows: Vec<Counted> =
            (0..500).map(|i| Counted(i, Arc::clone(&seen))).collect();

        let envelope =
            paged_rows(rows, req(401, 50), "t", "items", "the list", &[], NO_FULLER_FORM).expect("page");

        assert_eq!(envelope["returned"], json!(50));
        assert_eq!(envelope["offset"], json!(401));
        assert_eq!(seen.load(Ordering::SeqCst), 50);
    }

    /// An offset past the end is refused, and nothing is serialized to say so.
    ///
    /// Deliberately not asserting that the rows go undropped: `paged_rows` owns
    /// the `Vec`, so every element is dropped when it returns whichever branch
    /// it takes. What is observable — and what matters — is that the error
    /// names the offset and no row was ever converted.
    #[test]
    fn an_offset_past_the_end_is_refused_before_any_row_is_serialized() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        struct Counted(Arc<AtomicUsize>);
        impl Serialize for Counted {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                self.0.fetch_add(1, Ordering::SeqCst);
                s.serialize_u64(0)
            }
        }

        let serialized = Arc::new(AtomicUsize::new(0));
        let rows: Vec<Counted> = (0..1_000)
            .map(|_| Counted(Arc::clone(&serialized)))
            .collect();

        let error = paged_rows(rows, req(5_000, 50), "t", "items", "the list", &[], NO_FULLER_FORM)
            .expect_err("an offset past the end must fail");

        assert!(error.contains("offset"), "got {error}");
        assert_eq!(serialized.load(Ordering::SeqCst), 0);
    }

    /// The clamped limit is reported, so a caller can tell a ceiling from a
    /// short page. `returned` alone is ambiguous: it also shrinks when the byte
    /// budget cuts the page and when the list simply runs out.
    #[test]
    fn the_envelope_reports_the_limit_actually_in_force() {
        let envelope =
            paged_rows(rows(500), req(1, 200), "t", "items", "the list", &[], NO_FULLER_FORM).expect("page");

        assert_eq!(envelope["limit"], json!(200));
        assert_eq!(envelope["returned"], json!(200));
        assert_eq!(envelope["total"], json!(500));
    }

    #[test]
    fn an_absent_limit_still_bounds_the_reply() {
        let parsed = page_request(&json!({}), 50, 200).expect("defaults");
        assert_eq!(parsed.offset, 1);
        assert_eq!(parsed.limit, 50);
    }

    #[test]
    fn a_limit_above_the_ceiling_is_clamped_not_rejected() {
        let parsed = page_request(&json!({ "limit": 10_000 }), 50, 200).expect("clamped");
        assert_eq!(parsed.limit, 200);
    }

    #[test]
    fn a_zero_offset_is_rejected_rather_than_read_as_one() {
        let error = page_request(&json!({ "offset": 0 }), 50, 200).expect_err("rejected");
        assert!(error.contains("positive integer"), "got {error}");
    }

    #[test]
    fn a_complete_page_says_so_and_offers_no_next_offset() {
        let envelope = page_envelope(rows(3), req(1, 50), "t", "items", "the list").expect("page");
        assert_eq!(envelope["complete"], json!(true));
        assert_eq!(envelope["next_offset"], Value::Null);
        assert_eq!(envelope["total"], json!(3));
        assert_eq!(envelope["returned"], json!(3));
        assert!(envelope.get("notice").is_none());
    }

    #[test]
    fn a_row_limited_page_reports_where_to_resume() {
        let envelope = page_envelope(rows(10), req(1, 4), "t", "items", "the list").expect("page");
        assert_eq!(envelope["returned"], json!(4));
        assert_eq!(envelope["next_offset"], json!(5));
        assert_eq!(envelope["complete"], json!(false));
        assert!(envelope["notice"]
            .as_str()
            .expect("notice")
            .contains("\"offset\": 5"));
        // ...and resuming there really does continue where this page stopped.
        let next = page_envelope(rows(10), req(5, 4), "t", "items", "the list").expect("page");
        assert_eq!(next["items"][0]["i"], json!(4));
    }

    #[test]
    fn the_last_page_is_not_marked_complete_when_it_started_past_row_one() {
        // `complete` means "this reply is the whole list", not "there is
        // nothing after it" — a caller that resumed at row 5 did not receive
        // rows 1-4 and must not be told it holds everything.
        let envelope = page_envelope(rows(6), req(5, 50), "t", "items", "the list").expect("page");
        assert_eq!(envelope["next_offset"], Value::Null);
        assert_eq!(envelope["complete"], json!(false));
    }

    #[test]
    fn the_byte_budget_cuts_a_page_short_before_the_row_limit_does() {
        // Rows of roughly 1 KB: the row limit would allow 200, the byte budget
        // allows about 32.
        let fat: Vec<Value> = (0..200)
            .map(|i| json!({ "i": i, "text": "x".repeat(1024) }))
            .collect();

        let envelope = page_envelope(fat, req(1, 200), "t", "items", "the list").expect("page");

        let returned = envelope["returned"].as_u64().expect("returned") as usize;
        assert!(returned < 200, "the byte budget did not bite: {returned}");
        assert!(returned > 1, "the budget was far too tight: {returned}");
        let serialized = serde_json::to_string(&envelope["items"]).expect("serialize");
        assert!(
            serialized.len() <= MAX_PAGE_BYTES + 1024,
            "the page overran its budget: {}",
            serialized.len()
        );
        assert_eq!(envelope["next_offset"], json!(returned + 1));
    }

    #[test]
    fn a_single_row_larger_than_the_whole_budget_is_still_returned() {
        // Otherwise the page comes back empty with `next_offset` pointing at
        // the row that was skipped, and a paging client loops on it forever.
        let huge = vec![
            json!({ "text": "x".repeat(MAX_PAGE_BYTES * 2) }),
            json!({ "text": "small" }),
        ];

        let envelope = page_envelope(huge, req(1, 50), "t", "items", "the list").expect("page");

        assert_eq!(envelope["returned"], json!(1));
        assert_eq!(envelope["next_offset"], json!(2));
    }

    #[test]
    fn an_offset_past_the_end_is_an_error_naming_the_row_count() {
        let error =
            page_envelope(rows(3), req(9, 50), "t", "items", "the list").expect_err("out of range");
        assert!(error.contains("has 3 row(s)"), "got {error}");
    }

    #[test]
    fn offset_one_of_an_empty_list_is_an_empty_page_not_an_error() {
        let envelope = page_envelope(vec![], req(1, 50), "t", "items", "the list").expect("page");
        assert_eq!(envelope["items"], json!([]));
        assert_eq!(envelope["complete"], json!(true));
    }

    #[test]
    fn a_long_text_field_is_capped_and_says_which_field_was_cut() {
        let mut row = json!({ "content": "y".repeat(MAX_FIELD_BYTES * 2), "role": "user" });

        cap_text_fields(&mut row, &["content", "tool_output"], "get_run_events", NO_FULLER_FORM);

        let content = row["content"].as_str().expect("content");
        assert!(content.starts_with(&"y".repeat(MAX_FIELD_BYTES)));
        assert!(content.contains("[get_run_events FIELD TRUNCATED: 'content' is 8192 bytes"));
        assert_eq!(row["role"], json!("user"), "other fields are untouched");
    }

    #[test]
    fn capping_a_text_field_never_splits_a_codepoint() {
        // MAX_FIELD_BYTES is 4096, which 3 does not divide, so the cut lands
        // inside a "€". Slicing there would panic.
        let mut row = json!({ "content": "\u{20AC}".repeat(4000) });

        cap_text_fields(&mut row, &["content"], "t", NO_FULLER_FORM);

        let content = row["content"].as_str().expect("content");
        let expected = (MAX_FIELD_BYTES / 3) * 3;
        assert!(content.starts_with(&"\u{20AC}".repeat(expected / 3)));
        assert!(content.contains(&format!("the first {expected} are shown")));
    }

    #[test]
    fn a_field_at_exactly_the_cap_is_left_alone() {
        let mut row = json!({ "content": "y".repeat(MAX_FIELD_BYTES) });
        cap_text_fields(&mut row, &["content"], "t", NO_FULLER_FORM);
        assert!(!row["content"].as_str().expect("content").contains("TRUNCATED"));
    }

    #[test]
    fn capping_ignores_absent_and_non_string_fields() {
        let mut row = json!({ "content": Value::Null, "is_error": true });
        cap_text_fields(&mut row, &["content", "tool_output", "is_error"], "t", NO_FULLER_FORM);
        assert_eq!(row, json!({ "content": Value::Null, "is_error": true }));
    }
}
