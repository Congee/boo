//! Daemon identity and TLS trust plumbing for the remote subsystem.
//!
//! Extracted from `remote.rs` to shrink that file to a manageable size. Holds:
//!
//! - Process-wide rustls crypto provider install.
//! - Ed25519 keypair + self-signed X.509 cert generation, persistence, and
//!   validated-load (both for the auto-generated pair and for caller-supplied
//!   `--cert-path` / `--key-path` overrides).
//! - `daemon_identity` derivation from the cert's `SubjectPublicKeyInfo` hash.
//! - The SPKI-pinning rustls `ServerCertVerifier` used by TCP+TLS and QUIC
//!   clients.
//! - Helper builders that assemble rustls `ClientConfig` / `ServerConfig`
//!   values the rest of the remote transport consumes.

use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// DNS-like name presented by the server's self-signed cert. Used as the
/// `ServerName` input to rustls on the client side. The pinning verifier
/// ignores CA chain and hostname anyway, but rustls still requires a
/// syntactically valid name to feed SNI.
pub(crate) const REMOTE_DAEMON_SERVER_NAME: &str = "boo-remote-daemon";

/// Install the `ring` crypto provider as rustls's process-wide default.
///
/// Every `rustls::ServerConfig` / `ClientConfig` we build goes through
/// `builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))`,
/// which passes the provider explicitly. That covers today's rustls 0.23
/// code paths. But rustls reserves the right to grow new call sites that
/// look up the provider via `CryptoProvider::get_default()` instead of the
/// embedded one, and those panic with "no process-level CryptoProvider
/// available" if nothing is installed. rustls 0.24+ is expected to
/// tighten in that direction and may also flip the default from `ring` to
/// `aws-lc-rs` (which has a C build dep we do not want).
///
/// Called once from `fn main` before any rustls code path runs. Idempotent
/// via a `Once` guard so accidentally calling it again is free.
pub fn install_default_crypto_provider() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

#[derive(Clone)]
pub struct DaemonIdentityMaterial {
    pub identity_id: String,
    pub key_pem: String,
    pub cert_pem: String,
}

fn daemon_identity_dir() -> PathBuf {
    crate::config::config_dir().join("remote-daemon-identity")
}

pub(crate) fn load_or_create_daemon_identity() -> String {
    load_or_create_daemon_identity_material().identity_id
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn load_or_create_daemon_identity_material() -> DaemonIdentityMaterial {
    load_or_create_daemon_identity_material_at(&daemon_identity_dir())
}

/// Load daemon identity material from caller-provided cert/key paths.
/// Validates that the cert's SPKI matches the key's SPKI before accepting —
/// same contract as `try_load_validated_identity` — so mismatched files
/// refuse cleanly instead of silently advertising a rotated identity.
///
/// This is the `--cert-path` / `--key-path` escape hatch for deployments
/// behind external CAs (ACME, internal PKI, etc.). The generated-identity
/// directory is untouched in this case.
pub fn load_external_daemon_identity_material(
    cert_chain_path: &Path,
    key_path: &Path,
) -> Result<DaemonIdentityMaterial, String> {
    try_load_validated_identity(key_path, cert_chain_path).ok_or_else(|| {
        format!(
            "failed to load external daemon identity (cert {} + key {}): either the files are missing/unreadable, the key is not a valid PEM keypair, the cert.pem contains no certificate, or the cert's SubjectPublicKeyInfo does not match the key's",
            cert_chain_path.display(),
            key_path.display()
        )
    })
}

pub(crate) fn load_or_create_daemon_identity_material_at(
    dir: &Path,
) -> DaemonIdentityMaterial {
    let key_path = dir.join("key.pem");
    let cert_path = dir.join("cert.pem");

    if let Some(material) = try_load_validated_identity(&key_path, &cert_path) {
        return material;
    }

    let material = generate_daemon_identity_material();
    if let Err(error) = persist_daemon_identity(dir, &material) {
        log::warn!(
            "failed to persist remote daemon identity at {}: {error}",
            dir.display()
        );
    }
    material
}

/// Load a previously-persisted daemon identity pair and verify that the cert's
/// `SubjectPublicKeyInfo` hash matches the key's. A mismatch means the two files
/// drifted (partial write, manual edit, disk corruption) and the stored pair
/// cannot be trusted as a stable pin anchor, so the caller regenerates instead
/// of silently rotating the identity.
fn try_load_validated_identity(
    key_path: &Path,
    cert_path: &Path,
) -> Option<DaemonIdentityMaterial> {
    let key_pem = std::fs::read_to_string(key_path).ok()?;
    let cert_pem = std::fs::read_to_string(cert_path).ok()?;
    let keypair = rcgen::KeyPair::from_pem(&key_pem).ok()?;
    let key_spki: Vec<u8> = keypair.public_key_der();

    let cert_ders: Vec<_> = rustls_pemfile::certs(&mut cert_pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    let cert_der = cert_ders.first()?;
    let cert_spki = extract_cert_spki_der(cert_der.as_ref()).ok()?;
    if cert_spki != key_spki {
        log::warn!(
            "remote daemon cert/key SPKI mismatch at {}: regenerating identity pair",
            key_path.parent().unwrap_or(key_path).display()
        );
        return None;
    }

    let identity_id = derive_identity_id(key_spki);
    Some(DaemonIdentityMaterial {
        identity_id,
        key_pem,
        cert_pem,
    })
}

/// Persist a daemon identity pair via temp-file + rename. Errors surface to
/// the caller instead of being silently dropped: the daemon may continue
/// with the in-memory material for the current process, but the next
/// restart can then retry persistence rather than masking disk-full or
/// permission issues.
fn persist_daemon_identity(
    dir: &Path,
    material: &DaemonIdentityMaterial,
) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let key_path = dir.join("key.pem");
    let cert_path = dir.join("cert.pem");
    let key_tmp = dir.join("key.pem.tmp");
    let cert_tmp = dir.join("cert.pem.tmp");

    write_and_fsync(&key_tmp, material.key_pem.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&key_tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    write_and_fsync(&cert_tmp, material.cert_pem.as_bytes())?;
    // Rename order is not load-atomic across the two files, but the SPKI
    // validation in try_load_validated_identity catches any mismatch after a
    // partial rename and forces regeneration — preventing silent identity
    // rotation or startup failure.
    std::fs::rename(&cert_tmp, &cert_path)?;
    std::fs::rename(&key_tmp, &key_path)?;
    Ok(())
}

fn write_and_fsync(path: &Path, data: &[u8]) -> io::Result<()> {
    let mut file = std::fs::File::create(path)?;
    file.write_all(data)?;
    file.sync_all()?;
    Ok(())
}

pub(crate) fn generate_daemon_identity_material() -> DaemonIdentityMaterial {
    let keypair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519)
        .expect("generate ed25519 keypair for remote daemon identity");
    let mut params = rcgen::CertificateParams::new(vec!["boo-remote-daemon".to_string()])
        .expect("build remote daemon cert params");
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "boo remote daemon");
    let cert = params
        .self_signed(&keypair)
        .expect("self-sign remote daemon cert");
    let identity_id = derive_identity_id(keypair.public_key_der());
    DaemonIdentityMaterial {
        identity_id,
        key_pem: keypair.serialize_pem(),
        cert_pem: cert.pem(),
    }
}

pub(crate) fn derive_identity_id(spki_der: impl AsRef<[u8]>) -> String {
    use base64::Engine;
    use sha2::Digest;
    let digest = sha2::Sha256::digest(spki_der.as_ref());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

fn extract_cert_spki_der(cert_der: &[u8]) -> Result<Vec<u8>, String> {
    use x509_parser::prelude::*;
    let (_, cert) = X509Certificate::from_der(cert_der)
        .map_err(|error| format!("parse remote daemon cert: {error}"))?;
    Ok(cert.tbs_certificate.subject_pki.raw.to_vec())
}

pub(crate) fn cert_der_matches_identity(cert_der: &[u8], expected_identity: &str) -> bool {
    match extract_cert_spki_der(cert_der) {
        Ok(spki) => derive_identity_id(spki) == expected_identity,
        Err(_) => false,
    }
}

#[derive(Debug)]
pub(crate) struct PinnedSpkiServerCertVerifier {
    expected_identity: String,
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl PinnedSpkiServerCertVerifier {
    pub(crate) fn new(expected_identity: String) -> Self {
        Self {
            expected_identity,
            provider: Arc::new(rustls::crypto::ring::default_provider()),
        }
    }
}

impl rustls::client::danger::ServerCertVerifier for PinnedSpkiServerCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        if cert_der_matches_identity(end_entity.as_ref(), &self.expected_identity) {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
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
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

pub(crate) fn build_remote_client_tls_config(
    expected_identity: &str,
) -> Result<rustls::ClientConfig, String> {
    let verifier = Arc::new(PinnedSpkiServerCertVerifier::new(
        expected_identity.to_string(),
    ));
    let provider = Arc::clone(&verifier.provider);
    Ok(rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| format!("negotiate TLS protocol versions: {error}"))?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth())
}

pub(crate) fn build_remote_server_tls_config(
    material: &DaemonIdentityMaterial,
) -> Result<Arc<rustls::ServerConfig>, String> {
    use rustls::pki_types::PrivateKeyDer;

    let cert_chain = rustls_pemfile::certs(&mut material.cert_pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("parse remote daemon cert: {error}"))?;
    if cert_chain.is_empty() {
        return Err("remote daemon cert.pem contained no certificates".to_string());
    }
    let key_der: PrivateKeyDer<'static> =
        rustls_pemfile::private_key(&mut material.key_pem.as_bytes())
            .map_err(|error| format!("parse remote daemon private key: {error}"))?
            .ok_or_else(|| {
                "remote daemon key.pem contained no private key".to_string()
            })?;

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| format!("negotiate TLS protocol versions: {error}"))?
        .with_no_client_auth()
        .with_single_cert(cert_chain, key_der)
        .map_err(|error| format!("install remote daemon cert in rustls: {error}"))?;
    Ok(Arc::new(config))
}
