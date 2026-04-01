use anyhow::Result;
use std::sync::atomic::Ordering;
use tracing::info;

use super::super::handler::FtpSession;

impl FtpSession {
    pub async fn cmd_help(&mut self, arg: Option<&str>) -> Result<()> {
        if let Some(cmd) = arg {
            let help_text = match cmd.to_uppercase().as_str() {
                "USER" => "214 USER <username>: Specify user name\r\n",
                "PASS" => "214 PASS <password>: Specify password\r\n",
                "CWD" => "214 CWD <directory>: Change working directory\r\n",
                "CDUP" => "214 CDUP: Change to parent directory\r\n",
                "PWD" => "214 PWD: Print working directory\r\n",
                "LIST" => "214 LIST [<path>]: List directory contents\r\n",
                "NLST" => "214 NLST [<path>]: List directory names\r\n",
                "RETR" => "214 RETR <filename>: Retrieve file\r\n",
                "STOR" => "214 STOR <filename>: Store file\r\n",
                "DELE" => "214 DELE <filename>: Delete file\r\n",
                "MKD" => "214 MKD <directory>: Create directory\r\n",
                "RMD" => "214 RMD <directory>: Remove directory\r\n",
                "RNFR" => "214 RNFR <filename>: Specify rename source\r\n",
                "RNTO" => "214 RNTO <filename>: Specify rename destination\r\n",
                "PASV" => "214 PASV: Enter passive mode\r\n",
                "EPSV" => "214 EPSV: Enter extended passive mode\r\n",
                "PORT" => "214 PORT <h1,h2,h3,h4,p1,p2>: Enter active mode\r\n",
                "EPRT" => "214 EPRT |<netproto>|<netaddr>|<tcpport>|: Extended active mode\r\n",
                "TYPE" => "214 TYPE <type>: Set transfer type (A/I)\r\n",
                "MODE" => "214 MODE <mode>: Set transfer mode (S/B/C)\r\n",
                "STRU" => "214 STRU <structure>: Set file structure (F/R/P)\r\n",
                "REST" => "214 REST <offset>: Set restart marker\r\n",
                "SIZE" => "214 SIZE <filename>: Get file size\r\n",
                "MDTM" => "214 MDTM <filename>: Get modification time\r\n",
                "ABOR" => "214 ABOR: Abort current transfer\r\n",
                "QUIT" => "214 QUIT: Disconnect from server\r\n",
                _ => "214 Unknown command\r\n",
            };
            self.stream.write_all(help_text.as_bytes()).await?;
        } else {
            self.stream.write_all(b"214-The following commands are recognized:\r\n").await?;
            self.stream.write_all(b"214-USER PASS CWD CDUP PWD LIST NLST RETR STOR\r\n").await?;
            self.stream.write_all(b"214-DELE MKD RMD RNFR RNTO PASV EPSV PORT EPRT\r\n").await?;
            self.stream.write_all(b"214-TYPE MODE STRU REST SIZE MDTM ABOR QUIT\r\n").await?;
            self.stream.write_all(b"214-MLSD MLST SYST FEAT STAT HELP NOOP\r\n").await?;
            self.stream.write_all(b"214 Direct comments to admin\r\n").await?;
        }
        Ok(())
    }

    pub async fn cmd_mode(&mut self, arg: Option<&str>) -> Result<()> {
        if let Some(mode) = arg {
            match mode.to_uppercase().as_str() {
                "S" => self.stream.write_all(b"200 Mode set to Stream\r\n").await?,
                "B" => self.stream.write_all(b"504 Block mode not supported\r\n").await?,
                "C" => self.stream.write_all(b"504 Compressed mode not supported\r\n").await?,
                _ => self.stream.write_all(b"501 Unknown mode\r\n").await?,
            }
        } else {
            self.stream.write_all(b"501 Syntax error: MODE requires parameter\r\n").await?;
        }
        Ok(())
    }

    pub async fn cmd_stru(&mut self, arg: Option<&str>) -> Result<()> {
        if let Some(structure) = arg {
            match structure.to_uppercase().as_str() {
                "F" => self.stream.write_all(b"200 Structure set to File\r\n").await?,
                "R" => self.stream.write_all(b"504 Record structure not supported\r\n").await?,
                "P" => self.stream.write_all(b"504 Page structure not supported\r\n").await?,
                _ => self.stream.write_all(b"501 Unknown structure\r\n").await?,
            }
        } else {
            self.stream.write_all(b"501 Syntax error: STRU requires parameter\r\n").await?;
        }
        Ok(())
    }

    pub async fn cmd_type(&mut self, arg: Option<&str>) -> Result<()> {
        if let Some(type_code) = arg {
            match type_code.to_uppercase().as_str() {
                "I" | "L 8" => {
                    self.binary_transfer = true;
                    self.stream.write_all(b"200 Type set to I (Binary)\r\n").await?;
                }
                "A" | "A N" => {
                    self.binary_transfer = false;
                    self.stream.write_all(b"200 Type set to A (ASCII)\r\n").await?;
                }
                "E" => {
                    self.binary_transfer = true;
                    self.stream.write_all(b"200 Type set to E (EBCDIC)\r\n").await?;
                }
                _ => self.stream.write_all(b"501 Unknown type\r\n").await?,
            }
        } else {
            self.binary_transfer = true;
            self.stream.write_all(b"200 Type set to I (Binary)\r\n").await?;
        }
        Ok(())
    }

    pub async fn cmd_rest(&mut self, arg: Option<&str>) -> Result<()> {
        if let Some(offset_str) = arg {
            if let Ok(offset) = offset_str.parse::<u64>() {
                self.rest_offset = offset;
                self.stream.write_all(format!("350 Restarting at {}\r\n", offset).as_bytes()).await?;
                // 使用 tracing 记录客户端操作审计日志
                info!(
                    username = self.current_user.as_deref().unwrap_or("anonymous"),
                    client_ip = %self.remote_ip,
                    offset = offset,
                    "FTP 设置断点续传位置"
                );
            } else {
                self.stream.write_all(b"501 Syntax error in REST parameter\r\n").await?;
            }
        } else {
            self.rest_offset = 0;
            self.stream.write_all(b"350 Restarting at 0\r\n").await?;
        }
        Ok(())
    }

    pub async fn cmd_stat(&mut self) -> Result<()> {
        if let Some(ref username) = self.current_user {
            self.stream.write_all(b"211-FTP server status:\r\n").await?;
            self.stream.write_all(format!("211-Connected to: {}\r\n", self.remote_ip).as_bytes()).await?;
            self.stream.write_all(format!("211-Logged in as: {}\r\n", username).as_bytes()).await?;
            self.stream.write_all(format!("211-Current directory: {}\r\n", self.cwd).as_bytes()).await?;
            self.stream.write_all(format!("211-Transfer mode: {}\r\n", 
                if self.binary_transfer { "Binary" } else { "ASCII" }).as_bytes()).await?;
            self.stream.write_all(format!("211-Data connection: {}\r\n", 
                if self.passive_mode { "Passive" } else { "Active" }).as_bytes()).await?;
            self.stream.write_all(b"211 End\r\n").await?;
        } else {
            self.stream.write_all(b"211 FTP server status - Not logged in\r\n").await?;
        }
        Ok(())
    }

    pub async fn handle_abor(&mut self) -> Result<()> {
        self.abort_flag.store(true, Ordering::Relaxed);
        self.stream.write_all(b"426 Connection closed; transfer aborted\r\n").await?;
        self.stream.write_all(b"226 Abort successful\r\n").await?;
        Ok(())
    }

    pub async fn cmd_opts(&mut self, arg: Option<&str>) -> Result<()> {
        if let Some(opts_arg) = arg {
            let parts: Vec<&str> = opts_arg.split_whitespace().collect();
            if parts.is_empty() {
                self.stream.write_all(b"501 Syntax error: OPTS requires parameters\r\n").await?;
                return Ok(());
            }

            match parts[0].to_uppercase().as_str() {
                "UTF8" => {
                    if parts.len() > 1 && parts[1].to_uppercase() == "ON" {
                        self.utf8_enabled = true;
                        self.stream.write_all(b"200 UTF8 enabled\r\n").await?;
                    } else if parts.len() > 1 && parts[1].to_uppercase() == "OFF" {
                        self.utf8_enabled = false;
                        self.stream.write_all(b"200 UTF8 disabled\r\n").await?;
                    } else {
                        self.utf8_enabled = true;
                        self.stream.write_all(b"200 UTF8 enabled\r\n").await?;
                    }
                }
                "MLST" => {
                    self.stream.write_all(b"200 MLST OPTS Type*;Size*;Modify*;\r\n").await?;
                }
                _ => {
                    self.stream.write_all(b"501 Unknown OPTS parameter\r\n").await?;
                }
            }
        } else {
            self.stream.write_all(b"501 Syntax error: OPTS requires parameters\r\n").await?;
        }
        Ok(())
    }

    pub async fn cmd_rein(&mut self) -> Result<()> {
        self.current_user = None;
        self.authenticated = false;
        self.cwd = String::new();
        self.home_dir = String::new();
        self.rest_offset = 0;
        self.rename_from = None;
        self.passive_mode = false;
        self.data_port = None;
        self.data_addr = None;
        
        self.stream.write_all(b"220 Ready for new user\r\n").await?;
        
        // 使用 tracing 记录客户端操作审计日志
        info!(
            client_ip = %self.remote_ip,
            old_username = self.current_user.as_deref().unwrap_or("none"),
            "FTP 会话重新初始化"
        );
        
        Ok(())
    }

    pub async fn cmd_site(&mut self, arg: Option<&str>) -> Result<()> {
        if let Some(site_arg) = arg {
            let parts: Vec<&str> = site_arg.split_whitespace().collect();
            
            if parts.is_empty() {
                self.stream.write_all(b"501 Syntax error: SITE requires parameters\r\n").await?;
                return Ok(());
            }

            match parts[0].to_uppercase().as_str() {
                "CHMOD" => {
                    if parts.len() >= 3 {
                        self.stream.write_all(b"200 SITE CHMOD command accepted\r\n").await?;
                    } else {
                        self.stream.write_all(b"501 Syntax error: SITE CHMOD requires mode and path\r\n").await?;
                    }
                }
                "UMASK" => {
                    if parts.len() >= 2 {
                        self.stream.write_all(b"200 SITE UMASK command accepted\r\n").await?;
                    } else {
                        self.stream.write_all(b"501 Syntax error: SITE UMASK requires mask\r\n").await?;
                    }
                }
                "HELP" => {
                    self.stream.write_all(b"214-SITE commands: CHMOD, UMASK, HELP\r\n214 End\r\n").await?;
                }
                _ => {
                    self.stream.write_all(b"500 Unknown SITE command\r\n").await?;
                }
            }
        } else {
            self.stream.write_all(b"501 Syntax error: SITE requires parameters\r\n").await?;
        }
        Ok(())
    }
}
