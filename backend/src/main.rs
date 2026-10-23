//! DBX SSHX plugin sidecar entrypoint.

mod app;
mod core;
mod features;
mod filesystem;
mod platform;
mod sessions;

fn main() -> std::io::Result<()> {
    app::run()
}
