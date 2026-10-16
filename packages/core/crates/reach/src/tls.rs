//! TLS between a phone and vornd, trusted by pinning the server's
//! certificate rather than through any authority.
//!
//! vornd makes one self-signed certificate and keeps it; pairing hands the
//! phone its SHA-256 [`fingerprint`]; the phone's [`client_config`] accepts
//! that certificate and no other, whatever name or authority it shows.

use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{ring, CryptoProvider};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{CertificateError, ClientConfig, DigitallySignedStruct, ServerConfig, SignatureScheme};
use sha2::{Digest, Sha256};

pub use rustls;

/// A certificate and its private key, both DER (the key PKCS #8).
#[derive(Clone, PartialEq, Eq)]
pub struct Identity {
    pub cert: Vec<u8>,
    pub key: Vec<u8>,
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity")
            .field("fingerprint", &fingerprint(&self.cert))
            .finish_non_exhaustive()
    }
}

impl Identity {
    /// A new self-signed certificate that never needs renewing.
    #[cfg(feature = "mint")]
    pub fn generate() -> Result<Identity, String> {
        let made = rcgen::generate_simple_self_signed(vec!["vorn".to_owned()])
            .map_err(|e| e.to_string())?;
        Ok(Identity {
            cert: made.cert.der().to_vec(),
            key: made.signing_key.serialize_der(),
        })
    }

    pub fn fingerprint(&self) -> String {
        fingerprint(&self.cert)
    }
}

/// The SHA-256 of a DER certificate, as 64 lowercase hex digits.
pub fn fingerprint(cert: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(&Sha256::digest(cert))
}

/// `typed` as a fingerprint, ignoring case and the spaces or colons it is
/// grouped with; `None` unless it is 64 hex digits.
pub fn parse_fingerprint(typed: &str) -> Option<String> {
    let hex: String = typed
        .chars()
        .filter(|c| !matches!(c, ' ' | ':' | '-'))
        .map(|c| c.to_ascii_lowercase())
        .collect();
    (hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit())).then_some(hex)
}

/// A fingerprint in groups of four, for reading aloud or comparing by eye.
pub fn grouped(fingerprint: &str) -> String {
    fingerprint
        .as_bytes()
        .chunks(4)
        .map(|c| String::from_utf8_lossy(c))
        .collect::<Vec<_>>()
        .join(" ")
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(ring::default_provider())
}

/// vornd's side: presents `identity`, asks nothing of the client.
pub fn server_config(identity: &Identity) -> Result<Arc<ServerConfig>, rustls::Error> {
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(identity.key.clone()));
    let config = ServerConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(vec![CertificateDer::from(identity.cert.clone())], key)?;
    Ok(Arc::new(config))
}

/// The phone's side: trusts only the certificate whose fingerprint is
/// `pin` (as [`parse_fingerprint`] reads it).
pub fn client_config(pin: &str) -> Result<Arc<ClientConfig>, rustls::Error> {
    let pin = parse_fingerprint(pin)
        .ok_or_else(|| rustls::Error::General("not a certificate fingerprint".to_owned()))?;
    let provider = provider();
    let config = ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Pinned { pin, provider }))
        .with_no_client_auth();
    Ok(Arc::new(config))
}

/// Accepts the one certificate pinned, and checks the handshake is signed by
/// its key.
#[derive(Debug)]
struct Pinned {
    pin: String,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if fingerprint(end_entity) == self.pin {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[cfg(all(test, feature = "mint"))]
mod tests {
    use super::*;
    use rustls::{ClientConnection, ServerConnection};

    /// Runs a handshake in memory; the client's error if it refuses.
    fn handshake(server: &Identity, pin: &str) -> Result<(), rustls::Error> {
        let name = ServerName::try_from("192.168.1.20").unwrap();
        let mut client = ClientConnection::new(client_config(pin)?, name)?;
        let mut server = ServerConnection::new(server_config(server).unwrap()).unwrap();
        for _ in 0..10 {
            let mut wire = Vec::new();
            while client.wants_write() {
                client.write_tls(&mut wire).unwrap();
            }
            server.read_tls(&mut &wire[..]).unwrap();
            if server.process_new_packets().is_err() {
                break;
            }
            let mut wire = Vec::new();
            while server.wants_write() {
                server.write_tls(&mut wire).unwrap();
            }
            client.read_tls(&mut &wire[..]).unwrap();
            client.process_new_packets()?;
            if !client.is_handshaking() {
                return Ok(());
            }
        }
        Err(rustls::Error::General("the handshake did not finish".to_owned()))
    }

    #[test]
    fn the_pinned_certificate_is_trusted_whatever_its_name() {
        let id = Identity::generate().unwrap();
        assert_eq!(handshake(&id, &id.fingerprint()), Ok(()));
        let typed = grouped(&id.fingerprint().to_uppercase());
        assert_eq!(handshake(&id, &typed), Ok(()));
    }

    #[test]
    fn any_other_certificate_is_refused() {
        let id = Identity::generate().unwrap();
        let other = Identity::generate().unwrap();
        assert_ne!(id.fingerprint(), other.fingerprint());
        assert_eq!(
            handshake(&id, &other.fingerprint()),
            Err(rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure
            ))
        );
    }

    #[test]
    fn a_fingerprint_reads_with_or_without_its_grouping() {
        let hex = "ab".repeat(32);
        assert_eq!(parse_fingerprint(&hex).as_deref(), Some(hex.as_str()));
        let colons = vec!["AB"; 32].join(":");
        assert_eq!(parse_fingerprint(&colons).as_deref(), Some(hex.as_str()));
        assert_eq!(parse_fingerprint(&grouped(&hex)).as_deref(), Some(hex.as_str()));
        assert_eq!(parse_fingerprint("abcd"), None);
        assert_eq!(parse_fingerprint(&"zz".repeat(32)), None);
        assert!(client_config("abcd").is_err());
        assert_eq!(grouped("0123456789"), "0123 4567 89");
    }
}
