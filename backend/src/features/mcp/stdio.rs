//! MCP stdio layer.

use super::*;

/// Single input-line ceiling (`DBX_SSH_MCP_STDIO_MAX_LINE`, bytes), the
/// family contract shared with files (`DBX_FILES_MCP_STDIO_MAX_LINE`) /
/// ldap / kafka: the request side has no schema-level size cap (a large
/// inline payload is a legitimate line), but the reader must stay bounded —
/// an unbounded line is an unbounded allocation from any writer on the other
/// end of the pipe. Default 16 MiB comfortably fits every legitimate inline
/// payload (the smoke's 8 MiB hostile-line case included); tests/smoke lower
/// it to exercise the over-limit path cheaply.
pub(crate) const DEFAULT_STDIO_MAX_LINE_BYTES: usize = 16 * 1024 * 1024;

pub(crate) fn stdio_max_line_bytes() -> usize {
    std::env::var("DBX_SSH_MCP_STDIO_MAX_LINE")
        .ok()
        .and_then(|raw| raw.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_STDIO_MAX_LINE_BYTES)
}

/// `-32700` answer for one over-limit input line (null id). The line has
/// already been consumed through its trailing newline by the `read_until`
/// loop, so the session continues with the next request.
pub(crate) fn over_limit_line_response(bytes: usize, limit: usize) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": null,
        "error": {
            "code": -32700,
            "message": format!(
                "Parse error: request line of {bytes} bytes exceeds the \
                 {limit}-byte limit (DBX_SSH_MCP_STDIO_MAX_LINE)"
            ),
        },
    })
}

/// One line read from the stdio peer, bounded by `max_line`.
pub(crate) enum StdioLineRead {
    /// EOF before any byte of this line.
    Eof,
    /// A line within the ceiling (raw bytes, newline included, in `raw`).
    Line,
    /// The ceiling filled without a newline: the remainder of the line has
    /// been drained and discarded, `raw` holds only the drained tail.
    OverLimit,
}

/// `read_until` without the unbounded-allocation hazard: a peer writing a
/// newline-less flood used to be buffered whole before the limit check. The
/// `Take` wrapper caps each read at `max_line + 1` bytes; a full cap without
/// a newline means over-limit, and the rest of the line is consumed (and
/// thrown away) so the session can continue with the next request.
pub(crate) fn read_bounded_line(
    reader: &mut impl BufRead,
    max_line: usize,
    raw: &mut Vec<u8>,
) -> io::Result<StdioLineRead> {
    raw.clear();
    let mut limited = std::io::Read::take(&mut *reader, max_line as u64 + 1);
    let read = limited.read_until(b'\n', raw)?;
    if read == 0 {
        return Ok(StdioLineRead::Eof);
    }
    if raw.len() == max_line + 1 && !raw.ends_with(b"\n") {
        loop {
            raw.clear();
            let consumed = reader.read_until(b'\n', raw)?;
            if consumed == 0 || raw.ends_with(b"\n") {
                break;
            }
        }
        return Ok(StdioLineRead::OverLimit);
    }
    Ok(StdioLineRead::Line)
}

pub fn run_mcp_stdio(data_dir: PathBuf) -> io::Result<()> {
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|error| io::Error::other(format!("Failed to create async runtime: {error}")))?;
    let state = Arc::new(McpState::new(data_dir));
    let stdin = io::stdin();
    // Spawned handlers may finish out of order; the mutex keeps each
    // JSON-RPC line intact and id-based correlation makes ordering
    // irrelevant to callers.
    let stdout = Arc::new(std::sync::Mutex::new(io::stdout()));
    // Handles of every spawned request, drained before exit so a task that
    // is mid-connection (or mid-command) isn't cancelled when stdin closes.
    let mut in_flight = Vec::new();
    // Single-line ceiling: `read_until` (not `lines()`) so an over-limit
    // line is consumed WHOLE through its newline — the -32700 reply goes out
    // and the next request still parses. Lossy decoding turns invalid UTF-8
    // into U+FFFD, which fails JSON parsing below into the same -32700 path
    // (the InvalidData arm in `classify_stdio_line` stays as the defensive
    // fallback for that error shape).
    let max_line = stdio_max_line_bytes();
    let mut raw: Vec<u8> = Vec::new();
    loop {
        match read_bounded_line(&mut stdin.lock(), max_line, &mut raw)? {
            StdioLineRead::Eof => break,
            StdioLineRead::OverLimit => {
                write_response(&stdout, over_limit_line_response(max_line + 1, max_line))?;
                continue;
            }
            StdioLineRead::Line => {}
        }
        // Lossy decoding turns invalid UTF-8 into U+FFFD, which fails JSON
        // parsing below into the same -32700 path (the InvalidData arm in
        // `classify_stdio_line` stays as the defensive fallback for that
        // error shape).
        let text = String::from_utf8_lossy(&raw);
        let text = text.trim_end_matches(['\n', '\r']).to_string();
        if text.len() > max_line {
            write_response(&stdout, over_limit_line_response(text.len(), max_line))?;
            continue;
        }
        // Spawn every request instead of block_on: one long tool call (a
        // slow ssh_exec, an sftp transfer) must not stall ping, tools/list,
        // or calls for other connections behind it. The host may already
        // have abandoned THIS call; its handler still runs to completion
        // and replies into the pipe.
        match classify_stdio_line(Ok(text))? {
            StdioLine::Silent => continue,
            StdioLine::Reply(response) => write_response(&stdout, response)?,
            StdioLine::Request(request) => {
                let state = Arc::clone(&state);
                let stdout = Arc::clone(&stdout);
                in_flight.push(runtime.spawn(async move {
                    if let Some(response) = state.dispatch(request).await {
                        let _ = write_response(&stdout, response);
                    }
                }));
            }
        }
    }
    // stdin is closed: drain in-flight handlers (bounded, as a runaway
    // handler must not pin the process forever) before the runtime drops.
    // The timeout future is built INSIDE block_on: tokio timers capture
    // Handle::current() at construction, which needs the runtime context.
    let drain = async {
        for handle in in_flight {
            let _ = handle.await;
        }
    };
    let _ = runtime.block_on(async { tokio::time::timeout(Duration::from_secs(300), drain).await });
    Ok(())
}

pub(crate) fn write_response(
    stdout: &std::sync::Mutex<io::Stdout>,
    response: Value,
) -> io::Result<()> {
    let mut guard = stdout
        .lock()
        .map_err(|poisoned| io::Error::other(poisoned.to_string()))?;
    writeln!(guard, "{response}")?;
    guard.flush()
}

/// Transport-tier outcome of one stdio line (reliability round 5): keep the
/// process serving no matter what a hostile client writes into the pipe.
#[derive(Debug)]
pub(crate) enum StdioLine {
    /// Blank line or CRLF-only line: nothing to answer, no state change.
    Silent,
    /// A ready transport-tier reply (-32700 parse error) that must go out
    /// before the next line is read.
    Reply(Value),
    /// A parseable JSON value for the async JSON-RPC dispatcher.
    Request(Value),
}

/// Classifies one stdin line. The `read_until` loop decodes lossily, so an
/// invalid-UTF-8 line arrives as a String with U+FFFD bytes and fails JSON
/// parsing into the same -32700 path; the `InvalidData` arm below stays as
/// the defensive fallback for the `BufRead::lines`-style error shape (and is
/// unit-tested). Real I/O errors still abort.
pub(crate) fn classify_stdio_line(line: io::Result<String>) -> io::Result<StdioLine> {
    let line = match line {
        Ok(line) => line,
        Err(error) if error.kind() == io::ErrorKind::InvalidData => {
            return Ok(StdioLine::Reply(json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": { "code": -32700, "message": format!("Parse error: {error}") },
            })));
        }
        Err(error) => return Err(error),
    };
    if line.trim().is_empty() {
        // Trailing `\r` from a CRLF peer and empty keepalive lines land here.
        return Ok(StdioLine::Silent);
    }
    match serde_json::from_str(&line) {
        Ok(request) => Ok(StdioLine::Request(request)),
        Err(error) => Ok(StdioLine::Reply(json!({
            "jsonrpc": "2.0",
            "id": null,
            "error": { "code": -32700, "message": format!("Parse error: {error}") },
        }))),
    }
}
