use anyhow::{Context, Result};
use rustls::ServerConfig;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;
use std::sync::Arc;

#[derive(Clone, Debug, Default)]
pub struct TlsConfig {
    pub enabled: bool,
    pub cert_path: String,
    pub key_path: String,
    pub require_tls: bool,
}

impl TlsConfig {
    pub fn new(enabled: bool, cert_path: String, key_path: String, require_tls: bool) -> Self {
        Self {
            enabled,
            cert_path,
            key_path,
            require_tls,
        }
    }

    pub fn load_server_config(&self) -> Result<Arc<ServerConfig>> {
        if !self.enabled {
            anyhow::bail!("TLS is not enabled");
        }

        let cert_path = Path::new(&self.cert_path);
        let key_path = Path::new(&self.key_path);

        let cert_file = File::open(cert_path)
            .with_context(|| format!("Failed to open certificate file: {:?}", cert_path))?;
        let key_file = File::open(key_path)
            .with_context(|| format!("Failed to open private key file: {:?}", key_path))?;

        let mut cert_reader = BufReader::new(cert_file);
        let mut key_reader = BufReader::new(key_file);

        let cert_chain = parse_certs(&mut cert_reader).context("Failed to read certificates")?;

        let private_key = parse_private_key(&mut key_reader)
            .context("Failed to read private key")?
            .ok_or_else(|| anyhow::anyhow!("No private key found in file"))?;

        let config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(cert_chain, private_key)
            .context("Failed to create TLS server config")?;

        Ok(Arc::new(config))
    }

    pub fn is_configured(&self) -> bool {
        self.enabled && !self.cert_path.is_empty() && !self.key_path.is_empty()
    }

    pub fn validate(&self) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        if self.cert_path.is_empty() {
            anyhow::bail!("TLS enabled but certificate path is not configured");
        }

        if self.key_path.is_empty() {
            anyhow::bail!("TLS enabled but private key path is not configured");
        }

        let cert_path = Path::new(&self.cert_path);
        if !cert_path.exists() {
            anyhow::bail!("Certificate file does not exist: {:?}", cert_path);
        }

        let key_path = Path::new(&self.key_path);
        if !key_path.exists() {
            anyhow::bail!("Private key file does not exist: {:?}", key_path);
        }

        self.load_server_config()
            .context("Failed to load TLS configuration")?;

        Ok(())
    }
}

pub fn load_tls_config(cert_path: &str, key_path: &str) -> Result<Arc<ServerConfig>> {
    let config = TlsConfig::new(true, cert_path.to_string(), key_path.to_string(), false);
    config.load_server_config()
}

fn parse_certs(reader: &mut impl Read) -> Result<Vec<rustls::pki_types::CertificateDer<'static>>> {
    let mut cert_data = Vec::new();
    reader.read_to_end(&mut cert_data)?;

    let pem_items = pem::parse_many(&cert_data).context("Failed to parse PEM data")?;

    let mut certs = Vec::new();
    for pem_obj in pem_items {
        if pem_obj.tag() == "CERTIFICATE" {
            certs.push(rustls::pki_types::CertificateDer::from(
                pem_obj.contents().to_vec(),
            ));
        }
    }

    Ok(certs)
}

fn parse_private_key(
    reader: &mut impl Read,
) -> Result<Option<rustls::pki_types::PrivateKeyDer<'static>>> {
    let mut key_data = Vec::new();
    reader.read_to_end(&mut key_data)?;

    let pem_items = pem::parse_many(&key_data).context("Failed to parse PEM private key")?;

    for pem_obj in pem_items {
        let tag = pem_obj.tag();

        // 支持多种私钥格式
        if tag == "PRIVATE KEY" || tag == "RSA PRIVATE KEY" || tag == "EC PRIVATE KEY" {
            // 使用 PrivatePkcs8KeyDer 包装，然后转换为 PrivateKeyDer
            let key_der = rustls::pki_types::PrivatePkcs8KeyDer::from(pem_obj.contents().to_vec());
            return Ok(Some(key_der.into()));
        }
    }

    Ok(None)
}
