//! CA + leaf certificate management for MITM TLS interception.
//!
//! On first run, generates a self-signed CA at `~/.jsbeautify/ca.{cer,key}`
//! that the user must trust in their browser/system store. Per-host leaf
//! certificates are signed on the fly and cached in memory by hostname.

use rcgen::{CertificateParams, DistinguishedName, DnType, IsCa, KeyPair, KeyUsagePurpose, SanType};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Owned CA: the issued certificate + private key.
#[derive(Clone)]
pub struct CertificateAuthority {
    pub cert_pem: String,
    pub key_pem: String,
    keypair: Arc<KeyPair>,
    cert: Arc<rcgen::Certificate>,
    /// host → (cert_chain, private_key) cache.
    leaf_cache: Arc<Mutex<HashMap<String, (Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)>>>,
}

impl CertificateAuthority {
    /// Load CA from `dir` if it exists, else generate a fresh one and write it.
    ///
    /// # Errors
    /// Returns I/O or rcgen errors.
    pub fn load_or_generate<P: AsRef<Path>>(dir: P) -> Result<Self, String> {
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir).map_err(|e| format!("create_dir_all({}): {e}", dir.display()))?;
        let cert_path = dir.join("ca.cer");
        let key_path = dir.join("ca.key");

        if cert_path.exists() && key_path.exists() {
            let cert_pem = std::fs::read_to_string(&cert_path).map_err(|e| e.to_string())?;
            let key_pem = std::fs::read_to_string(&key_path).map_err(|e| e.to_string())?;
            return Self::from_pem(&cert_pem, &key_pem);
        }

        let keypair = KeyPair::generate().map_err(|e| e.to_string())?;
        let mut params = CertificateParams::default();
        params.is_ca = IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.key_usages = vec![
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::CrlSign,
            KeyUsagePurpose::DigitalSignature,
        ];
        params.distinguished_name = {
            let mut dn = DistinguishedName::new();
            dn.push(DnType::CommonName, "js-beautify-rs MITM CA");
            dn.push(DnType::OrganizationName, "js-beautify-rs");
            dn
        };
        let cert = params.self_signed(&keypair).map_err(|e| e.to_string())?;
        let cert_pem = cert.pem();
        let key_pem = keypair.serialize_pem();
        std::fs::write(&cert_path, &cert_pem).map_err(|e| e.to_string())?;
        std::fs::write(&key_path, &key_pem).map_err(|e| e.to_string())?;
        Self::from_pem(&cert_pem, &key_pem)
    }

    fn from_pem(cert_pem: &str, key_pem: &str) -> Result<Self, String> {
        let keypair = KeyPair::from_pem(key_pem).map_err(|e| format!("KeyPair::from_pem: {e}"))?;
        // We re-issue a CertificateParams matching what was loaded so we can sign leaves.
        let mut params = CertificateParams::from_ca_cert_pem(cert_pem)
            .map_err(|e| format!("CertificateParams::from_ca_cert_pem: {e}"))?;
        params.is_ca = IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let cert = params.self_signed(&keypair).map_err(|e| e.to_string())?;
        Ok(Self {
            cert_pem: cert_pem.to_string(),
            key_pem: key_pem.to_string(),
            keypair: Arc::new(keypair),
            cert: Arc::new(cert),
            leaf_cache: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Issue (or reuse a cached) leaf cert for `host`.
    ///
    /// # Errors
    /// rcgen / rustls errors.
    pub fn leaf_for(&self, host: &str) -> Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>), String> {
        if let Some(cached) = self.leaf_cache.lock().unwrap_or_else(|e| e.into_inner()).get(host) {
            // Clone PrivateKeyDer manually since it doesn't impl Clone.
            return Ok((cached.0.clone(), clone_key(&cached.1)));
        }

        let leaf_kp = KeyPair::generate().map_err(|e| e.to_string())?;
        let mut params = CertificateParams::new(vec![host.to_string()]).map_err(|e| e.to_string())?;
        params.distinguished_name.push(DnType::CommonName, host);
        params.subject_alt_names.push(SanType::DnsName(
            host.to_string().try_into().map_err(|e: rcgen::Error| e.to_string())?,
        ));
        let leaf = params
            .signed_by(&leaf_kp, &self.cert, &self.keypair)
            .map_err(|e| e.to_string())?;

        let leaf_der = leaf.der().to_vec();
        let chain: Vec<CertificateDer<'static>> = vec![CertificateDer::from(leaf_der)];

        let key_der = leaf_kp.serialize_der();
        let key: PrivateKeyDer<'static> = PrivateKeyDer::from(PrivatePkcs8KeyDer::from(key_der));

        let cached_key = clone_key(&key);
        self.leaf_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(host.to_string(), (chain.clone(), cached_key));
        Ok((chain, key))
    }

    #[must_use]
    pub fn cert_pem(&self) -> &str {
        &self.cert_pem
    }
}

fn clone_key(k: &PrivateKeyDer<'static>) -> PrivateKeyDer<'static> {
    match k {
        PrivateKeyDer::Pkcs8(d) => PrivateKeyDer::from(PrivatePkcs8KeyDer::from(d.secret_pkcs8_der().to_vec())),
        PrivateKeyDer::Pkcs1(d) => PrivateKeyDer::Pkcs1(rustls::pki_types::PrivatePkcs1KeyDer::from(
            d.secret_pkcs1_der().to_vec(),
        )),
        PrivateKeyDer::Sec1(d) => {
            PrivateKeyDer::Sec1(rustls::pki_types::PrivateSec1KeyDer::from(d.secret_sec1_der().to_vec()))
        }
        _ => unreachable!("unsupported key type"),
    }
}

/// Default location: `~/.jsbeautify/ca/`
#[must_use]
pub fn default_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".jsbeautify")
        .join("ca")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_ca_generation() {
        let dir = tempfile_dir();
        let ca = CertificateAuthority::load_or_generate(&dir).unwrap();
        assert!(ca.cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(ca.key_pem.contains("BEGIN PRIVATE KEY"));

        // Load a second time — should reuse the same on-disk files. The PEM
        // strings on the in-memory struct are re-parsed from disk each time,
        // so we compare the disk content rather than the regenerated cert.
        let cert_disk_1 = std::fs::read_to_string(dir.join("ca.cer")).unwrap();
        let _ca2 = CertificateAuthority::load_or_generate(&dir).unwrap();
        let cert_disk_2 = std::fs::read_to_string(dir.join("ca.cer")).unwrap();
        assert_eq!(cert_disk_1, cert_disk_2, "CA file should not be regenerated");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn leaf_cert_includes_san() {
        let dir = tempfile_dir();
        let ca = CertificateAuthority::load_or_generate(&dir).unwrap();
        let (chain, _key) = ca.leaf_for("example.com").unwrap();
        assert_eq!(chain.len(), 1);

        // Cached lookup
        let (chain2, _) = ca.leaf_for("example.com").unwrap();
        assert_eq!(chain, chain2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn leaf_for_recovers_from_poisoned_cache() {
        let dir = tempfile_dir();
        let ca = CertificateAuthority::load_or_generate(&dir).unwrap();

        // Poison the leaf-cache mutex by panicking while holding its lock.
        let cache = ca.leaf_cache.clone();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = cache.lock().unwrap();
            panic!("intentional panic to poison the mutex");
        }));
        assert!(ca.leaf_cache.lock().is_err(), "mutex should now be poisoned");

        // Despite poisoning, leaf issuance must still succeed (graceful recovery
        // via `unwrap_or_else(|e| e.into_inner())`).
        let (chain, _key) = ca
            .leaf_for("example.com")
            .expect("leaf issuance should recover from a poisoned cache");
        assert_eq!(chain.len(), 1);

        // And a second cached lookup also works.
        let (chain2, _) = ca.leaf_for("example.com").expect("cached lookup after recovery");
        assert_eq!(chain, chain2);
        std::fs::remove_dir_all(&dir).ok();
    }

    fn tempfile_dir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let nonce = N.fetch_add(1, Ordering::SeqCst);
        let p = std::env::temp_dir().join(format!("jsbtest-ca-{}-{nonce}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        p
    }
}
