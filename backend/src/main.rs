//! DBX RemoteX plugin sidecar entrypoint.

mod app;
mod core;
mod features;
mod filesystem;
mod platform;
mod sessions;

#[cfg(test)]
mod structure_tests;

fn main() -> std::io::Result<()> {
    app::run()
}
