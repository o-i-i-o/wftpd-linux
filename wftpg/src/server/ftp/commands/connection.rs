use anyhow::Result;
use std::sync::Arc;
use tracing::{info, warn};

use super::super::data_connection::{create_passive_listener, find_available_passive_port};
use super::super::handler::FtpSession;

impl FtpSession {
    pub async fn cmd_pasv(&mut self) -> Result<()> {
        self.passive_mode = true;

        if self.remote_ip.contains(':') {
            self.stream.write_all(b"425 IPv6 addresses not supported in PASV mode, use EPSV instead\r\n").await?;
            warn!(client_ip = %self.remote_ip, "FTP PASV 模式不支持 IPv6 地址");
            return Ok(());
        }

        let (port_min, port_max, bind_ip, masquerade_ip) = {
            let cfg = self.config.lock().unwrap();
            let ports = cfg.ftp.passive_ports;
            (ports.0, ports.1, cfg.ftp.bind_ip.clone(), cfg.ftp.masquerade_ip.clone())
        };

        let passive_port = find_available_passive_port(&self.passive_listeners, port_min, port_max).await?;
        let passive_listener = create_passive_listener(&bind_ip, passive_port).await?;

        {
            let mut listeners = self.passive_listeners.lock().await;
            listeners.insert(passive_port, Arc::new(tokio::sync::Mutex::new(Some(passive_listener))));
        }

        self.data_port = Some(passive_port);

        let server_ip = if let Some(ref masq_ip) = masquerade_ip {
            masq_ip.clone()
        } else if bind_ip == "0.0.0.0" {
            self.local_ip.clone().unwrap_or_else(|| self.remote_ip.clone())
        } else {
            bind_ip.clone()
        };
        
        let ip_octets = server_ip.replace('.', ",");
        self.stream.write_all(
            format!(
                "227 Entering Passive Mode ({},{},{})\r\n",
                ip_octets,
                passive_port >> 8,
                passive_port & 0xFF
            )
            .as_bytes(),
        ).await?;

        info!(
            username = self.current_user.as_deref().unwrap_or("anonymous"),
            client_ip = %self.remote_ip,
            port = passive_port,
            server_ip = %server_ip,
            "FTP PASV 模式已启用"
        );
        Ok(())
    }

    pub async fn cmd_epsv(&mut self) -> Result<()> {
        self.passive_mode = true;
        let (port_min, port_max, bind_ip) = {
            let cfg = self.config.lock().unwrap();
            (cfg.ftp.passive_ports.0, cfg.ftp.passive_ports.1, cfg.ftp.bind_ip.clone())
        };

        let passive_port = find_available_passive_port(&self.passive_listeners, port_min, port_max).await?;
        let passive_listener = create_passive_listener(&bind_ip, passive_port).await?;

        {
            let mut listeners = self.passive_listeners.lock().await;
            listeners.insert(passive_port, Arc::new(tokio::sync::Mutex::new(Some(passive_listener))));
        }

        self.data_port = Some(passive_port);
        self.stream.write_all(
            format!("229 Entering Extended Passive Mode (|||{}|)\r\n", passive_port).as_bytes(),
        ).await?;
        Ok(())
    }

    pub async fn cmd_port(&mut self, arg: Option<&str>) -> Result<()> {
        if let Some(data) = arg {
            let parts: Vec<u16> = data.split(',').filter_map(|s| s.parse().ok()).collect();
            if parts.len() == 6 {
                let port = parts[4] * 256 + parts[5];
                let client_specified_ip = format!("{}.{}.{}.{}", parts[0], parts[1], parts[2], parts[3]);

                if client_specified_ip != self.remote_ip {
                    warn!(
                        client_ip = %self.remote_ip,
                        specified_ip = %client_specified_ip,
                        "FTP PORT 命令被拒绝：IP 地址不匹配"
                    );
                    self.stream.write_all(b"500 Illegal PORT command - IP must match client IP\r\n").await?;
                    return Ok(());
                }

                let addr = format!("{}:{}", client_specified_ip, port);
                self.data_port = Some(port);
                self.data_addr = Some(addr);
                self.passive_mode = false;
                self.stream.write_all(b"200 PORT command successful\r\n").await?;
            } else {
                self.stream.write_all(b"501 Syntax error in parameters or arguments\r\n").await?;
            }
        } else {
            self.stream.write_all(b"501 Syntax error: PORT requires parameters\r\n").await?;
        }
        Ok(())
    }

    pub async fn cmd_eprt(&mut self, arg: Option<&str>) -> Result<()> {
        if let Some(data) = arg {
            let parts: Vec<&str> = data.split('|').collect();
            if parts.len() >= 4 {
                let net_proto = parts[1];
                let net_addr = parts[2];
                let tcp_port = parts[3];

                if net_addr != self.remote_ip {
                    warn!(
                        client_ip = %self.remote_ip,
                        specified_ip = %net_addr,
                        "FTP EPRT 命令被拒绝：IP 地址不匹配"
                    );
                    self.stream.write_all(b"500 Illegal EPRT command - IP must match client IP\r\n").await?;
                    return Ok(());
                }

                match net_proto {
                    "1" => {
                        if let Ok(port) = tcp_port.parse::<u16>() {
                            self.data_port = Some(port);
                            self.data_addr = Some(format!("{}:{}", net_addr, port));
                            self.passive_mode = false;
                            self.stream.write_all(b"200 EPRT command successful\r\n").await?;
                        } else {
                            self.stream.write_all(b"501 Invalid port number\r\n").await?;
                        }
                    }
                    "2" => {
                        if let Ok(port) = tcp_port.parse::<u16>() {
                            self.data_port = Some(port);
                            self.data_addr = Some(format!("[{}]:{}", net_addr, port));
                            self.passive_mode = false;
                            self.stream.write_all(b"200 EPRT command successful (IPv6)\r\n").await?;
                        } else {
                            self.stream.write_all(b"501 Invalid port number\r\n").await?;
                        }
                    }
                    _ => {
                        self.stream.write_all(b"522 Protocol not supported, use (1,2)\r\n").await?;
                    }
                }
            } else {
                self.stream.write_all(b"501 Syntax error in EPRT parameters\r\n").await?;
            }
        } else {
            self.stream.write_all(b"501 Syntax error: EPRT requires parameters\r\n").await?;
        }
        Ok(())
    }
}
