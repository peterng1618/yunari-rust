//! The local certificate authority: a root this machine keeps and a leaf it
//! mints at every start.
//!
//! A phone that installs the root once trusts every later start, because the
//! leaf is signed by it. The root's key is a file beside its certificate.

use std::net::IpAddr;
use std::path::Path;
use std::sync::Arc;

use rand::prelude::*;
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose,
    IsCa, Issuer, KeyPair, KeyUsagePurpose, SerialNumber, date_time_ymd,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use sha2::{Digest, Sha256};

pub const CA_COMMON_NAME: &str = "Yunari Local CA";
pub const CA_ORGANIZATION: &str = "Yunari";
pub const CA_DAYS: i64 = 3_650;
pub const LEAF_DAYS: i64 = 397;

#[derive(Debug)]
pub enum TlsError {
    Io(String),
    Key(String),
    Certificate(String),
}

impl std::fmt::Display for TlsError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(message) | Self::Key(message) | Self::Certificate(message) => {
                formatter.write_str(message)
            }
        }
    }
}

impl std::error::Error for TlsError {}

/// The root and the leaf this run serves.
pub struct Authority {
    root_pem: String,
    root_der: Vec<u8>,
    leaf_pem: String,
    server: Arc<rustls::ServerConfig>,
}

impl std::fmt::Debug for Authority {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Authority")
            .field("fingerprint", &self.fingerprint_hex())
            .finish_non_exhaustive()
    }
}

impl Authority {
    /// Read the root from `settings/tls`, minting it on first use, then sign a
    /// leaf valid for this machine's names.
    pub fn load(settings_dir: &Path) -> Result<Self, TlsError> {
        let dir = settings_dir.join("tls");
        let cert_path = dir.join("ca.crt");
        let key_path = dir.join("ca-key.bin");

        let (root_pem, root_der, issuer) = match (
            std::fs::read_to_string(&cert_path),
            std::fs::read(&key_path),
        ) {
            (Ok(root_pem), Ok(key_der)) => {
                let key = KeyPair::from_der_and_sign_algo(
                    &PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_der.clone())),
                    &rcgen::PKCS_ECDSA_P256_SHA256,
                )
                .map_err(|error| TlsError::Key(format!("the root key is unreadable: {error}")))?;
                let root_der = pem_der(&root_pem).ok_or_else(|| {
                    TlsError::Certificate("the root certificate is not PEM".to_owned())
                })?;
                let issuer = Issuer::new(root_params(), key);
                (root_pem, root_der, issuer)
            }
            _ => {
                let key = KeyPair::generate()
                    .map_err(|error| TlsError::Key(format!("no root key: {error}")))?;
                let params = root_params();
                let certificate = params
                    .self_signed(&key)
                    .map_err(|error| TlsError::Certificate(format!("no root cert: {error}")))?;
                let root_pem = certificate.pem();
                let root_der = certificate.der().to_vec();
                std::fs::create_dir_all(&dir)
                    .map_err(|error| TlsError::Io(format!("cannot create {}: {error}", dir.display())))?;
                crate::config::write_atomic(&cert_path, root_pem.as_bytes())
                    .map_err(|error| TlsError::Io(format!("cannot write the root cert: {error}")))?;
                crate::config::write_atomic(&key_path, &key.serialize_der())
                    .map_err(|error| TlsError::Io(format!("cannot write the root key: {error}")))?;
                let issuer = Issuer::new(root_params(), key);
                (root_pem, root_der, issuer)
            }
        };

        let leaf_key =
            KeyPair::generate().map_err(|error| TlsError::Key(format!("no leaf key: {error}")))?;
        let mut params = CertificateParams::new(local_names())
            .map_err(|error| TlsError::Certificate(format!("no leaf names: {error}")))?;
        params.distinguished_name = leaf_name();
        params.key_usages = vec![
            KeyUsagePurpose::DigitalSignature,
            KeyUsagePurpose::KeyEncipherment,
        ];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        params.is_ca = IsCa::NoCa;
        params.serial_number = Some(random_serial());
        params.not_before = date_time_ymd(1975, 1, 1);
        params.not_after = date_years_from_now(LEAF_DAYS);
        let leaf = params
            .signed_by(&leaf_key, &issuer)
            .map_err(|error| TlsError::Certificate(format!("no leaf cert: {error}")))?;
        let leaf_pem = leaf.pem();

        let chain = vec![
            CertificateDer::from(leaf.der().to_vec()),
            CertificateDer::from(root_der.clone()),
        ];
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der()));
        let server = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(chain, key)
            .map_err(|error| TlsError::Certificate(format!("no server config: {error}")))?;

        Ok(Self {
            root_pem,
            root_der,
            leaf_pem,
            server: Arc::new(server),
        })
    }

    pub fn server_config(&self) -> Arc<rustls::ServerConfig> {
        self.server.clone()
    }

    /// The root as PEM, which is what a phone installs.
    pub fn certificate_pem(&self) -> &str {
        &self.root_pem
    }

    pub fn leaf_pem(&self) -> &str {
        &self.leaf_pem
    }

    /// SHA-256 of the root's DER, lowercase hex: the header a phone compares
    /// before it trusts the download.
    pub fn fingerprint_hex(&self) -> String {
        Sha256::digest(&self.root_der)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

/// Every name this host answers to: the loopback names, the machine's own,
/// and each address a phone on the LAN might reach.
pub fn local_names() -> Vec<String> {
    let mut names = vec!["localhost".to_owned()];
    let host = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_default();
    if !host.is_empty() {
        names.push(host.clone());
        names.push(format!("{host}.local"));
    }
    if let Ok(networks) = sysinfo::Networks::new_with_refreshed_list() {
        for network in networks.values() {
            for address in network.ip_networks() {
                let address = address.addr;
                if address.is_loopback() {
                    continue;
                }
                names.push(address.to_string());
            }
        }
    }
    names.push("127.0.0.1".to_owned());
    names.push("::1".to_owned());
    names.sort();
    names.dedup();
    names
}

fn root_params() -> CertificateParams {
    let mut params = CertificateParams::default();
    let mut name = DistinguishedName::new();
    name.push(DnType::CommonName, CA_COMMON_NAME);
    name.push(DnType::OrganizationName, CA_ORGANIZATION);
    params.distinguished_name = name;
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    params.serial_number = Some(random_serial());
    params.not_before = date_time_ymd(1975, 1, 1);
    params.not_after = date_years_from_now(CA_DAYS);
    params
}

fn leaf_name() -> DistinguishedName {
    let mut name = DistinguishedName::new();
    name.push(DnType::CommonName, "Yunari");
    name.push(DnType::OrganizationName, CA_ORGANIZATION);
    name
}

/// A positive serial: the top bit clear, as the source does.
fn random_serial() -> SerialNumber {
    let mut bytes = [0u8; 16];
    rand::rng().fill(&mut bytes);
    bytes[0] &= 0x7f;
    if bytes[0] == 0 {
        bytes[0] = 1;
    }
    SerialNumber::from_slice(&bytes)
}

fn date_years_from_now(days: i64) -> time::OffsetDateTime {
    let today = jiff::Timestamp::now().to_zoned(jiff::tz::TimeZone::UTC).date();
    let later = today
        .checked_add(jiff::Span::new().days(days))
        .unwrap_or(today);
    date_time_ymd(i32::from(later.year()), later.month() as u8, later.day() as u8)
}

/// The DER a PEM certificate holds, when it holds exactly one.
fn pem_der(pem: &str) -> Option<Vec<u8>> {
    let body: String = pem
        .lines()
        .skip_while(|line| !line.starts_with("-----BEGIN"))
        .skip(1)
        .take_while(|line| !line.starts_with("-----END"))
        .collect();
    if body.is_empty() {
        return None;
    }
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(body).ok()
}

/// Whether an address is one a phone can reach over the LAN.
pub fn is_lan_address(address: IpAddr) -> bool {
    !address.is_loopback() && !address.is_unspecified()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yunari-tls-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok();
        dir
    }

    #[test]
    fn the_root_survives_a_restart() {
        let dir = dir("root");
        let first = Authority::load(&dir).expect("a first root");
        let second = Authority::load(&dir).expect("a second root");
        assert_eq!(first.certificate_pem(), second.certificate_pem());
        assert_eq!(first.fingerprint_hex(), second.fingerprint_hex());
        assert_eq!(first.fingerprint_hex().len(), 64);
        assert!(first.fingerprint_hex().chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_ne!(first.leaf_pem(), second.leaf_pem(), "the leaf is minted each start");
        assert!(dir.join("tls").join("ca.crt").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_names_cover_loopback_and_this_machine() {
        let names = local_names();
        assert!(names.contains(&"localhost".to_owned()));
        assert!(names.contains(&"127.0.0.1".to_owned()));
        assert!(names.contains(&"::1".to_owned()));
    }

    #[test]
    fn a_corrupt_root_is_reported_not_ignored() {
        let dir = dir("corrupt");
        let tls = dir.join("tls");
        std::fs::create_dir_all(&tls).unwrap();
        std::fs::write(tls.join("ca.crt"), "not a certificate").unwrap();
        std::fs::write(tls.join("ca-key.bin"), b"not a key").unwrap();
        assert!(Authority::load(&dir).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn private_addresses_are_not_lan_addresses() {
        assert!(!is_lan_address(IpAddr::from([127, 0, 0, 1])));
        assert!(is_lan_address(IpAddr::from([192, 168, 1, 4])));
    }
}
