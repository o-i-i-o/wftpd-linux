use anyhow::Result;
use std::mem;
use tokio_rustls::TlsAcceptor;

use super::super::handler::{FtpSession, FtpStream};

impl FtpSession {
    pub async fn cmd_auth(&mut self, arg: Option<&str>) -> Result<()> {
        let protocol = arg.map(|s| s.to_uppercase()).unwrap_or_default();
        
        if !self.is_tls_available() {
            self.stream.write_all(b"500 TLS not configured on server\r\n").await?;
            return Ok(());
        }

        if self.tls_enabled {
            self.stream.write_all(b"503 Already using TLS\r\n").await?;
            return Ok(());
        }

        match protocol.as_str() {
            "TLS" | "TLS-C" | "SSL" | "TLS-P" => {
                self.stream.write_all(b"234 OK, starting TLS negotiation\r\n").await?;
                
                let tls_acceptor = TlsAcceptor::from(
                    self.tls_server_config.as_ref().unwrap().clone()
                );

                let stream = mem::take(&mut self.stream);
                let raw_stream = match stream {
                    FtpStream::Plain(s) => s,
                    FtpStream::Tls(_) => {
                        anyhow::bail!("Unexpected TLS stream in AUTH");
                    }
                    FtpStream::Taken => {
                        anyhow::bail!("Stream already taken");
                    }
                };

                match tls_acceptor.accept(raw_stream).await {
                    Ok(tls_stream) => {
                        self.stream = FtpStream::Tls(Box::new(tls_stream));
                        self.tls_enabled = true;
                        self.logger.lock().unwrap().client_action(
                            "FTP",
                            "TLS negotiation successful",
                            &self.remote_ip,
                            self.current_user.as_deref(),
                            "TLS_START",
                        );
                    }
                    Err(e) => {
                        self.logger.lock().unwrap().error(
                            "FTP",
                            &format!("TLS negotiation failed: {}", e),
                        );
                        return Err(anyhow::anyhow!("TLS negotiation failed: {}", e));
                    }
                }
            }
            _ => {
                self.stream.write_all(b"504 Unknown authentication method\r\n").await?;
            }
        }

        Ok(())
    }

    pub async fn cmd_pbsz(&mut self, arg: Option<&str>) -> Result<()> {
        if !self.tls_enabled {
            if self.is_tls_available() {
                self.stream.write_all(b"503 Use AUTH first\r\n").await?;
            } else {
                self.stream.write_all(b"500 TLS not available\r\n").await?;
            }
            return Ok(());
        }

        if let Some(size_str) = arg {
            match size_str.parse::<u64>() {
                Ok(0) => {
                    self.pbsz_set = true;
                    self.stream.write_all(b"200 PBSZ=0\r\n").await?;
                }
                Ok(_) => {
                    self.stream.write_all(b"200 PBSZ=0 (only 0 supported for TLS)\r\n").await?;
                    self.pbsz_set = true;
                }
                Err(_) => {
                    self.stream.write_all(b"501 Invalid PBSZ parameter\r\n").await?;
                }
            }
        } else {
            self.stream.write_all(b"501 PBSZ requires parameter\r\n").await?;
        }

        Ok(())
    }

    pub async fn cmd_prot(&mut self, arg: Option<&str>) -> Result<()> {
        if !self.tls_enabled {
            if self.is_tls_available() {
                self.stream.write_all(b"503 Use AUTH first\r\n").await?;
            } else {
                self.stream.write_all(b"500 TLS not available\r\n").await?;
            }
            return Ok(());
        }

        if !self.pbsz_set {
            self.stream.write_all(b"503 Use PBSZ first\r\n").await?;
            return Ok(());
        }

        if let Some(level) = arg {
            match level.to_uppercase().as_str() {
                "P" => {
                    self.tls_data_required = true;
                    self.stream.write_all(b"200 PROT Private - Data channel secured\r\n").await?;
                    self.logger.lock().unwrap().client_action(
                        "FTP",
                        "Data channel protection set to Private",
                        &self.remote_ip,
                        self.current_user.as_deref(),
                        "PROT_P",
                    );
                }
                "C" => {
                    self.tls_data_required = false;
                    self.stream.write_all(b"200 PROT Clear - Data channel unsecured\r\n").await?;
                }
                "S" | "E" => {
                    self.stream.write_all(b"536 PROT level not supported (use P or C)\r\n").await?;
                }
                _ => {
                    self.stream.write_all(b"504 Unknown PROT level\r\n").await?;
                }
            }
        } else {
            self.stream.write_all(b"501 PROT requires parameter\r\n").await?;
        }

        Ok(())
    }

    pub async fn cmd_ccc(&mut self) -> Result<()> {
        if !self.tls_enabled {
            self.stream.write_all(b"533 Not in TLS mode\r\n").await?;
            return Ok(());
        }

        self.stream.write_all(b"200 CCC OK - reverting to clear text\r\n").await?;
        
        self.tls_enabled = false;
        self.tls_data_required = false;
        self.pbsz_set = false;

        self.logger.lock().unwrap().client_action(
            "FTP",
            "CCC - reverted to clear text",
            &self.remote_ip,
            self.current_user.as_deref(),
            "CCC",
        );

        Ok(())
    }
}
