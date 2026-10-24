//! Tree Download operations for [`SshRuntime`].

use super::*;

mod chunk;
mod prep;
mod read;
mod start;

use read::TreeReadFailure;
