//! User-supplied environment pair validation shared by preferences and local
//! terminal sessions.

/// Caps for renderer-supplied env, bounded like every other renderer input.
pub(crate) const USER_ENV_MAX_PAIRS: usize = 32;

pub(crate) const USER_ENV_MAX_KEY: usize = 128;

pub(crate) const USER_ENV_MAX_VALUE: usize = 4096;

/// Env the sidecar manages for the workbench: the TERM family defines how the
/// embedded terminal renders and `TERM_PROGRAM` is the plugin's detectability
/// marker. User values for these keys are ignored (case-insensitively).
pub(crate) const USER_ENV_MANAGED_KEYS: [&str; 3] = ["TERM", "COLORTERM", "TERM_PROGRAM"];

/// Validates one user-supplied env pair: non-empty trimmed key within the size
/// caps, no `=` and no control bytes (the spawn layer would reject or garble
/// them), not a managed key, value within the cap. `None` = drop the entry.
pub(crate) fn sanitize_env_pair(name: &str, value: &str) -> Option<(String, String)> {
    let name = name.trim();
    if name.is_empty()
        || name.len() > USER_ENV_MAX_KEY
        || name.contains('=')
        || name.bytes().any(|byte| byte.is_ascii_control())
        || USER_ENV_MANAGED_KEYS
            .iter()
            .any(|managed| managed.eq_ignore_ascii_case(name))
        || value.len() > USER_ENV_MAX_VALUE
    {
        return None;
    }
    Some((name.to_string(), value.to_string()))
}
