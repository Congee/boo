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

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_identity_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!(
            "boo-remote-daemon-identity-{label}-{}-{nanos}",
            std::process::id()
        ))
    }

    #[test]
    fn load_or_create_daemon_identity_material_persists_keypair_and_cert() {
        let dir = unique_identity_dir("persist");
        let _ = std::fs::remove_dir_all(&dir);

        let first = load_or_create_daemon_identity_material_at(&dir);
        let second = load_or_create_daemon_identity_material_at(&dir);

        assert!(!first.identity_id.is_empty());
        assert_eq!(first.identity_id, second.identity_id);
        assert_eq!(first.key_pem, second.key_pem);
        assert_eq!(first.cert_pem, second.cert_pem);
        assert!(dir.join("key.pem").exists());
        assert!(dir.join("cert.pem").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_or_create_daemon_identity_material_derives_identity_from_spki() {
        let dir = unique_identity_dir("spki");
        let _ = std::fs::remove_dir_all(&dir);

        let material = load_or_create_daemon_identity_material_at(&dir);
        let keypair = rcgen::KeyPair::from_pem(&material.key_pem).expect("parse key");
        let expected = derive_identity_id(keypair.public_key_der());
        assert_eq!(material.identity_id, expected);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cert_der_matches_identity_accepts_current_cert() {
        let dir = unique_identity_dir("spki-match");
        let _ = std::fs::remove_dir_all(&dir);
        let material = load_or_create_daemon_identity_material_at(&dir);

        let cert_ders = rustls_pemfile::certs(&mut material.cert_pem.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .expect("parse certs");
        let cert_der = cert_ders.first().expect("at least one cert");
        assert!(cert_der_matches_identity(
            cert_der.as_ref(),
            &material.identity_id
        ));

        let bogus = derive_identity_id([0u8; 32].as_slice());
        assert!(!cert_der_matches_identity(cert_der.as_ref(), &bogus));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_external_daemon_identity_material_accepts_matching_pair() {
        // Generate a pair in a scratch dir then load it via the external path:
        // simulates what the --remote-cert-path / --remote-key-path flags do.
        let dir = unique_identity_dir("external-ok");
        let _ = std::fs::remove_dir_all(&dir);
        let material = load_or_create_daemon_identity_material_at(&dir);

        let cert_path = dir.join("cert.pem");
        let key_path = dir.join("key.pem");
        let loaded = load_external_daemon_identity_material(&cert_path, &key_path)
            .expect("external load must accept matched pair");

        assert_eq!(loaded.identity_id, material.identity_id);
        assert_eq!(loaded.cert_pem, material.cert_pem);
        assert_eq!(loaded.key_pem, material.key_pem);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_external_daemon_identity_material_rejects_mismatched_pair() {
        let dir = unique_identity_dir("external-mismatch");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create dir");

        let material_a = generate_daemon_identity_material();
        let material_b = generate_daemon_identity_material();
        assert_ne!(material_a.identity_id, material_b.identity_id);
        let cert_path = dir.join("cert.pem");
        let key_path = dir.join("key.pem");
        std::fs::write(&cert_path, &material_a.cert_pem).expect("write cert");
        std::fs::write(&key_path, &material_b.key_pem).expect("write key");

        let err = match load_external_daemon_identity_material(&cert_path, &key_path) {
            Ok(_) => panic!("mismatched external pair must be rejected"),
            Err(err) => err,
        };
        assert!(err.contains("does not match"), "got error: {err}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn daemon_identity_material_regenerates_on_cert_key_spki_mismatch() {
        // Write a valid key.pem from keypair A but a valid cert.pem from keypair B.
        // The pair is well-formed individually but the SPKI hashes do not match,
        // so a stable-pin trust decision cannot be made and the loader must
        // regenerate.
        let dir = unique_identity_dir("spki-mismatch");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create dir");

        let material_a = generate_daemon_identity_material();
        let material_b = generate_daemon_identity_material();
        assert_ne!(
            material_a.identity_id, material_b.identity_id,
            "test fixture requires distinct keypairs"
        );
        std::fs::write(dir.join("key.pem"), &material_a.key_pem).expect("write key");
        std::fs::write(dir.join("cert.pem"), &material_b.cert_pem).expect("write cert");

        let loaded = load_or_create_daemon_identity_material_at(&dir);
        assert_ne!(
            loaded.identity_id, material_a.identity_id,
            "mismatched pair must not be accepted as identity A"
        );
        assert_ne!(
            loaded.identity_id, material_b.identity_id,
            "mismatched pair must not be accepted as identity B"
        );

        // Sanity: the newly-written pair must now validate on reload.
        let reload = load_or_create_daemon_identity_material_at(&dir);
        assert_eq!(reload.identity_id, loaded.identity_id);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn daemon_identity_material_regenerates_when_key_is_missing() {
        let dir = unique_identity_dir("regen");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create dir");
        // Only a cert present, key missing — must regenerate both.
        std::fs::write(dir.join("cert.pem"), "not a real cert").expect("write stale cert");

        let material = load_or_create_daemon_identity_material_at(&dir);

        assert!(!material.identity_id.is_empty());
        assert!(
            rcgen::KeyPair::from_pem(&material.key_pem).is_ok(),
            "regenerated key must parse",
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
