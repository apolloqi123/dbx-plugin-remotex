//! Re-export of the startup-command preference sanitizer and runtime plan.
//! The implementation lives in core so the preferences layer can validate the
//! stored shape without depending on the terminal feature.

pub use crate::core::preferences::startup_commands::*;
