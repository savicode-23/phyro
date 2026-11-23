use std::{
    env, fs, io,
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::PathBuf,
};
use tokio::net::{UnixListener, UnixStream};

pub type Stream = UnixStream;
pub type Client = UnixStream;

fn path(locator: &str) -> io::Result<PathBuf> {
    // SAFETY: geteuid takes no pointers and has no preconditions.
    let uid = unsafe { libc::geteuid() };
    let root = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir);
    let dir = root.join(format!("phyro-mcp-{uid}"));
    match fs::DirBuilder::new().mode(0o700).create(&dir) {
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
        other => other?,
    }
    let info = fs::symlink_metadata(&dir)?;
    if !info.is_dir() || info.uid() != uid || info.mode() & 0o077 != 0 {
        return Err(io::Error::other(
            "Pairing requires a private, user-owned runtime directory.",
        ));
    }
    Ok(dir.join(locator))
}

pub struct Listener(UnixListener, PathBuf);
impl Listener {
    pub(super) fn bind(locator: &str) -> io::Result<Self> {
        let path = path(locator)?;
        Ok(Self(UnixListener::bind(&path)?, path))
    }
    pub(super) async fn accept(self) -> io::Result<Stream> {
        Ok(self.0.accept().await?.0)
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.1);
    }
}

pub async fn connect(locator: &str) -> io::Result<Client> {
    UnixStream::connect(path(locator)?).await
}
