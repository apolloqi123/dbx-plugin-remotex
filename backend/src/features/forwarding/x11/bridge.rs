//! Metrics support.

use super::*;

/// Bridges an accepted x11 channel into the local X server endpoint
/// (unix socket first, TCP loopback fallback). The validated setup block
/// (real cookie already substituted) is written first; the relay then
/// continues byte-for-byte in both directions.
pub(crate) async fn bridge_channel(
    channel: russh::Channel<russh::client::Msg>,
    target: &DisplayTarget,
    setup: Vec<u8>,
) -> Result<(), String> {
    let connect_error = |error| format!("X11: cannot reach local display {target:?}: {error}");
    // Unix sockets do not exist on Windows: X servers there listen on TCP
    // (VcXsrv does by default), so the unix branch is compile-gated.
    #[cfg(unix)]
    if let DisplayServer::UnixSocket(path) = &target.server {
        let local = tokio::net::UnixStream::connect(path)
            .await
            .map_err(connect_error)?;
        let mut stream = channel.into_stream();
        stream
            .write_all(&setup)
            .await
            .map_err(|error| format!("X11: cannot write the X setup block: {error}"))?;
        let (up, down) = tokio::io::copy_bidirectional(&mut stream, &mut { local })
            .await
            .map_err(|error| format!("X11: bridge error: {error}"))?;
        tracing_bridge_stats(up, down);
        return Ok(());
    }
    let port = match &target.server {
        DisplayServer::TcpLoopback(port) => *port,
        #[cfg(unix)]
        DisplayServer::UnixSocket(_) => {
            return Err(
                "X11: unix-socket display handled by the unix branch above".to_string(),
            )
        }
        #[cfg(windows)]
        DisplayServer::UnixSocket(_) => {
            return Err("X11: unix sockets are unavailable on Windows; point DISPLAY at a TCP-capable X server (e.g. VcXsrv with 'disable access control' or -listen tcp)".to_string())
        }
    };
    let mut local = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .map_err(connect_error)?;
    let mut stream = channel.into_stream();
    stream
        .write_all(&setup)
        .await
        .map_err(|error| format!("X11: cannot write the X setup block: {error}"))?;
    let (up, down) = tokio::io::copy_bidirectional(&mut stream, &mut local)
        .await
        .map_err(|error| format!("X11: bridge error: {error}"))?;
    tracing_bridge_stats(up, down);
    Ok(())
}

pub(crate) fn tracing_bridge_stats(up: u64, down: u64) {
    if up + down > 0 {
        eprintln!("[x11] bridged {up}↑/{down}↓ bytes");
    }
}
