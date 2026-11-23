//! Local transport shared by the stdio bridge and the consenting editor.
use serde::{de::DeserializeOwned, Serialize};
use std::{io, time::Duration};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as platform;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as platform;

pub use platform::{Listener, Stream};
pub const PAIRING_LIFETIME: Duration = Duration::from_secs(120);
pub const MAX_REQUEST: usize = 1024 * 1024;
pub const MAX_RESPONSE: usize = 8 * 1024 * 1024;

pub fn token() -> io::Result<String> {
    let mut bytes = [0; 12];
    getrandom::fill(&mut bytes).map_err(|e| io::Error::other(e.to_string()))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn locator(code: &str) -> io::Result<&str> {
    if code.len() != 24 || !code.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(io::Error::other(
            "Enter the pairing code from the assistant.",
        ));
    }
    Ok(&code[..8])
}

pub fn listen(code: &str) -> io::Result<Listener> {
    Listener::bind(locator(code)?)
}

// Only the locator appears in the endpoint name. Verify the secret before sending
// the acknowledgement or any editor data, even if the original endpoint expired.
pub async fn pair(code: &str) -> io::Result<platform::Client> {
    let mut stream = platform::connect(locator(code)?).await?;
    let proof: String = receive(&mut stream, 128).await?;
    if proof != code[8..] {
        return Err(io::Error::other("Pairing code did not match."));
    }
    send(&mut stream, &true, 16).await?;
    Ok(stream)
}

pub async fn accept(listener: Listener, code: &str) -> io::Result<Stream> {
    let mut stream = listener.accept().await?;
    send(&mut stream, &&code[8..], 128).await?;
    if !receive::<bool>(&mut stream, 16).await? {
        return Err(io::Error::other("Pairing declined."));
    }
    Ok(stream)
}

pub async fn receive<T: DeserializeOwned>(
    stream: &mut (impl AsyncRead + Unpin),
    limit: usize,
) -> io::Result<T> {
    let size = stream.read_u32().await? as usize;
    if size > limit {
        return Err(io::Error::other("Local message is too large."));
    }
    let mut bytes = vec![0; size];
    stream.read_exact(&mut bytes).await?;
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}

pub async fn send(
    stream: &mut (impl AsyncWrite + Unpin),
    value: &impl Serialize,
    limit: usize,
) -> io::Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > limit {
        return Err(io::Error::other("Local message is too large."));
    }
    stream.write_u32(bytes.len() as u32).await?;
    stream.write_all(&bytes).await?;
    stream.flush().await
}
