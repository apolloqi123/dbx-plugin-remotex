//! Shell quoting helpers shared by remote command builders.

/// Single-quote shell escaping for embedding a path in a remote command.
pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}
