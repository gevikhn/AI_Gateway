use crate::config::InboundTlsConfig;
use rcgen::generate_simple_self_signed;
use std::fs;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlsPaths {
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsMaterialSource {
    Provided,
    ExistingSelfSigned,
    GeneratedSelfSigned,
}

pub fn resolve_tls_paths(
    tls: &InboundTlsConfig,
    listen_addr: SocketAddr,
) -> Result<(TlsPaths, TlsMaterialSource), String> {
    if let (Some(cert_path), Some(key_path)) = (&tls.cert_path, &tls.key_path) {
        return Ok((
            TlsPaths {
                cert_path: PathBuf::from(cert_path),
                key_path: PathBuf::from(key_path),
            },
            TlsMaterialSource::Provided,
        ));
    }

    let cert_path = PathBuf::from(tls.self_signed_cert_path.trim());
    let key_path = PathBuf::from(tls.self_signed_key_path.trim());
    let cert_exists = cert_path.exists();
    let key_exists = key_path.exists();

    if cert_exists && key_exists {
        return Ok((
            TlsPaths {
                cert_path,
                key_path,
            },
            TlsMaterialSource::ExistingSelfSigned,
        ));
    }

    if cert_exists ^ key_exists {
        return Err(
            "self-signed certificate and key must both exist or both be absent".to_string(),
        );
    }

    generate_self_signed_cert_files(&cert_path, &key_path, listen_addr.ip())?;
    Ok((
        TlsPaths {
            cert_path,
            key_path,
        },
        TlsMaterialSource::GeneratedSelfSigned,
    ))
}

fn generate_self_signed_cert_files(
    cert_path: &Path,
    key_path: &Path,
    listen_ip: IpAddr,
) -> Result<(), String> {
    ensure_parent_dir(cert_path)?;
    ensure_parent_dir(key_path)?;

    let subject_alt_names = build_subject_alt_names(listen_ip);
    let certified_key = generate_simple_self_signed(subject_alt_names)
        .map_err(|err| format!("failed to generate self-signed cert: {err}"))?;

    fs::write(cert_path, certified_key.cert.pem())
        .map_err(|err| format!("failed to write cert file `{}`: {err}", cert_path.display()))?;
    fs::write(key_path, certified_key.key_pair.serialize_pem())
        .map_err(|err| format!("failed to write key file `{}`: {err}", key_path.display()))?;
    Ok(())
}

fn ensure_parent_dir(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create directory `{}`: {err}", parent.display()))?;
    }
    Ok(())
}

fn build_subject_alt_names(listen_ip: IpAddr) -> Vec<String> {
    let mut names = vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "::1".to_string(),
    ];
    if !listen_ip.is_unspecified() {
        names.push(listen_ip.to_string());
    }
    names.sort();
    names.dedup();
    names
}

#[cfg(test)]
mod tests {
    use super::{TlsMaterialSource, resolve_tls_paths};
    use crate::config::InboundTlsConfig;
    use std::fs;
    use std::net::{Ipv4Addr, SocketAddr};
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn use_provided_cert_and_key_paths() {
        let tls = InboundTlsConfig {
            cert_path: Some("custom/server.crt".to_string()),
            key_path: Some("custom/server.key".to_string()),
            self_signed_cert_path: "certs/self.crt".to_string(),
            self_signed_key_path: "certs/self.key".to_string(),
        };

        let (paths, source) = resolve_tls_paths(&tls, test_addr()).expect("paths should resolve");
        assert_eq!(source, TlsMaterialSource::Provided);
        assert_eq!(paths.cert_path, PathBuf::from("custom/server.crt"));
        assert_eq!(paths.key_path, PathBuf::from("custom/server.key"));
    }

    #[test]
    fn generate_self_signed_when_missing() {
        let temp = temp_dir("generate");
        let cert_path = temp.join("selfsigned.crt");
        let key_path = temp.join("selfsigned.key");
        let tls = InboundTlsConfig {
            cert_path: None,
            key_path: None,
            self_signed_cert_path: cert_path.to_string_lossy().to_string(),
            self_signed_key_path: key_path.to_string_lossy().to_string(),
        };

        let (paths, source) = resolve_tls_paths(&tls, test_addr()).expect("paths should resolve");
        assert_eq!(source, TlsMaterialSource::GeneratedSelfSigned);
        assert!(paths.cert_path.exists());
        assert!(paths.key_path.exists());
        cleanup_temp(&temp);
    }

    #[test]
    fn load_existing_self_signed_files() {
        let temp = temp_dir("existing");
        let cert_path = temp.join("selfsigned.crt");
        let key_path = temp.join("selfsigned.key");
        fs::write(&cert_path, "CERT").expect("cert should be written");
        fs::write(&key_path, "KEY").expect("key should be written");

        let tls = InboundTlsConfig {
            cert_path: None,
            key_path: None,
            self_signed_cert_path: cert_path.to_string_lossy().to_string(),
            self_signed_key_path: key_path.to_string_lossy().to_string(),
        };

        let (paths, source) = resolve_tls_paths(&tls, test_addr()).expect("paths should resolve");
        assert_eq!(source, TlsMaterialSource::ExistingSelfSigned);
        assert_eq!(paths.cert_path, cert_path);
        assert_eq!(paths.key_path, key_path);
        cleanup_temp(&temp);
    }

    fn test_addr() -> SocketAddr {
        SocketAddr::from((Ipv4Addr::LOCALHOST, 8443))
    }

    fn temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ai-gw-lite-tls-{prefix}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("temp dir should be created");
        dir
    }

    fn cleanup_temp(path: &PathBuf) {
        let _ = fs::remove_dir_all(path);
    }
}
