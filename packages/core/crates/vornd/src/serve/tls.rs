//! The certificate vornd presents to other machines, made once and kept in
//! the data directory, so a paired phone's pin holds across restarts.

use std::io;
use std::path::Path;

use tokio_rustls::TlsAcceptor;
use vorn_reach::tls::{self, Identity};

const CERT_FILE: &str = "tls-cert.der";
const KEY_FILE: &str = "tls-key.der";

/// What another machine's connections are served with.
#[derive(Clone)]
pub struct Tls {
    pub acceptor: TlsAcceptor,
    pub fingerprint: String,
}

impl std::fmt::Debug for Tls {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tls")
            .field("fingerprint", &self.fingerprint)
            .finish_non_exhaustive()
    }
}

/// The kept certificate, or a new one when there is none or it cannot be used.
pub fn load(dir: &Path) -> io::Result<Tls> {
    if let Some(found) = read(dir).and_then(|id| serve(&id).ok()) {
        return Ok(found);
    }
    let id = Identity::generate().map_err(io::Error::other)?;
    write(dir, &id)?;
    serve(&id).map_err(io::Error::other)
}

fn read(dir: &Path) -> Option<Identity> {
    Some(Identity {
        cert: std::fs::read(dir.join(CERT_FILE)).ok()?,
        key: std::fs::read(dir.join(KEY_FILE)).ok()?,
    })
}

fn serve(id: &Identity) -> Result<Tls, tls::rustls::Error> {
    Ok(Tls {
        acceptor: TlsAcceptor::from(tls::server_config(id)?),
        fingerprint: id.fingerprint(),
    })
}

fn write(dir: &Path, id: &Identity) -> io::Result<()> {
    write_private(&dir.join(KEY_FILE), &id.key)?;
    write_private(&dir.join(CERT_FILE), &id.cert)
}

/// Writes `bytes` readable by this user only, replacing whatever was there.
fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    io::Write::write_all(&mut options.open(&tmp)?, bytes)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_certificate_is_kept_across_restarts() {
        let dir = std::env::temp_dir().join(format!("vornd-tls-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = load(&dir).unwrap();
        assert_eq!(load(&dir).unwrap().fingerprint, first.fingerprint);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join(KEY_FILE)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::write(dir.join(KEY_FILE), b"garbage").unwrap();
        assert_ne!(load(&dir).unwrap().fingerprint, first.fingerprint, "remade");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
