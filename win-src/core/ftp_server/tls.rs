use anyhow::Result;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use native_tls::Identity;
use tokio::net::TcpStream;
use tokio_native_tls::TlsAcceptor;
use tokio_native_tls::TlsStream as AsyncTlsStream;

pub type AsyncTlsTcpStream = AsyncTlsStream<TcpStream>;

#[allow(dead_code)]
#[derive(Clone)]
pub struct TlsConfig {
    pub enabled: bool,
    pub require_ssl: bool,
    pub acceptor: Option<Arc<TlsAcceptor>>,
}

impl TlsConfig {
    pub fn new(cert_path: Option<&str>, key_path: Option<&str>, require_ssl: bool) -> Self {
        match (cert_path, key_path) {
            (Some(cert), Some(key)) => {
                match load_tls_acceptor(cert, key) {
                    Ok(acceptor) => {
                        log::info!("TLS enabled with certificate: {}", cert);
                        TlsConfig {
                            enabled: true,
                            require_ssl,
                            acceptor: Some(Arc::new(acceptor)),
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to load TLS certificate: {}", e);
                        TlsConfig {
                            enabled: false,
                            require_ssl: false,
                            acceptor: None,
                        }
                    }
                }
            }
            _ => {
                if require_ssl {
                    log::warn!("SSL required but no certificate configured");
                }
                TlsConfig {
                    enabled: false,
                    require_ssl: false,
                    acceptor: None,
                }
            }
        }
    }

    pub fn is_tls_available(&self) -> bool {
        self.acceptor.is_some()
    }
}

fn load_tls_acceptor(cert_path: &str, key_path: &str) -> Result<TlsAcceptor> {
    let cert_path = Path::new(cert_path);
    let key_path = Path::new(key_path);
    
    let cert_extension = cert_path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase());
    
    let identity = match cert_extension.as_deref() {
        Some("pfx") | Some("p12") => {
            let mut cert_file = File::open(cert_path)?;
            let mut cert_data = Vec::new();
            cert_file.read_to_end(&mut cert_data)?;
            Identity::from_pkcs12(&cert_data, "")?
        }
        _ => {
            let mut cert_file = File::open(cert_path)?;
            let mut cert_data = Vec::new();
            cert_file.read_to_end(&mut cert_data)?;
            
            let mut key_file = File::open(key_path)?;
            let mut key_data = Vec::new();
            key_file.read_to_end(&mut key_data)?;
            
            Identity::from_pkcs8(&cert_data, &key_data)?
        }
    };
    
    let native_acceptor = native_tls::TlsAcceptor::builder(identity).build()?;
    let acceptor = TlsAcceptor::from(native_acceptor);
    Ok(acceptor)
}
