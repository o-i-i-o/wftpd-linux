use anyhow::Result;
use std::io::Write;
use std::sync::atomic::Ordering;

use super::super::handler::FtpSession;

impl FtpSession {
    pub fn cmd_help(&mut self, arg: Option<&str>) -> Result<()> {
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
            self.stream.write_all(help_text.as_bytes())?;
        } else {
            self.stream.write_all(b"214-The following commands are recognized:\r\n")?;
            self.stream.write_all(b"214-USER PASS CWD CDUP PWD LIST NLST RETR STOR\r\n")?;
            self.stream.write_all(b"214-DELE MKD RMD RNFR RNTO PASV EPSV PORT EPRT\r\n")?;
            self.stream.write_all(b"214-TYPE MODE STRU REST SIZE MDTM ABOR QUIT\r\n")?;
            self.stream.write_all(b"214-MLSD MLST SYST FEAT STAT HELP NOOP\r\n")?;
            self.stream.write_all(b"214 Direct comments to admin\r\n")?;
        }
        Ok(())
    }

    pub fn cmd_mode(&mut self, arg: Option<&str>) -> Result<()> {
        if let Some(mode) = arg {
            match mode.to_uppercase().as_str() {
                "S" => self.stream.write_all(b"200 Mode set to Stream\r\n")?,
                "B" => self.stream.write_all(b"504 Block mode not supported\r\n")?,
                "C" => self.stream.write_all(b"504 Compressed mode not supported\r\n")?,
                _ => self.stream.write_all(b"501 Unknown mode\r\n")?,
            }
        } else {
            self.stream.write_all(b"501 Syntax error: MODE requires parameter\r\n")?;
        }
        Ok(())
    }

    pub fn cmd_stru(&mut self, arg: Option<&str>) -> Result<()> {
        if let Some(structure) = arg {
            match structure.to_uppercase().as_str() {
                "F" => self.stream.write_all(b"200 Structure set to File\r\n")?,
                "R" => self.stream.write_all(b"504 Record structure not supported\r\n")?,
                "P" => self.stream.write_all(b"504 Page structure not supported\r\n")?,
                _ => self.stream.write_all(b"501 Unknown structure\r\n")?,
            }
        } else {
            self.stream.write_all(b"501 Syntax error: STRU requires parameter\r\n")?;
        }
        Ok(())
    }

    pub fn cmd_type(&mut self, arg: Option<&str>) -> Result<()> {
        if let Some(type_code) = arg {
            match type_code.to_uppercase().as_str() {
                "I" | "L 8" => self.stream.write_all(b"200 Type set to I (Binary)\r\n")?,
                "A" | "A N" => self.stream.write_all(b"200 Type set to A (ASCII)\r\n")?,
                "E" => self.stream.write_all(b"200 Type set to E (EBCDIC)\r\n")?,
                _ => self.stream.write_all(b"501 Unknown type\r\n")?,
            }
        } else {
            self.stream.write_all(b"200 Type set to I (Binary)\r\n")?;
        }
        Ok(())
    }

    pub fn cmd_rest(&mut self, arg: Option<&str>) -> Result<()> {
        if let Some(offset_str) = arg {
            if let Ok(offset) = offset_str.parse::<u64>() {
                self.rest_offset = offset;
                self.stream.write_all(format!("350 Restarting at {}\r\n", offset).as_bytes())?;
                self.logger.lock().unwrap().client_action(
                    "FTP",
                    &format!("REST command: offset {}", offset),
                    &self.remote_ip,
                    self.current_user.as_deref(),
                    "REST",
                );
            } else {
                self.stream.write_all(b"501 Syntax error in REST parameter\r\n")?;
            }
        } else {
            self.rest_offset = 0;
            self.stream.write_all(b"350 Restarting at 0\r\n")?;
        }
        Ok(())
    }

    pub fn cmd_prot(&mut self, arg: Option<&str>) -> Result<()> {
        if let Some(level) = arg {
            match level.to_uppercase().as_str() {
                "P" => self.stream.write_all(b"200 PROT Private\r\n")?,
                "C" => self.stream.write_all(b"200 PROT Clear\r\n")?,
                _ => self.stream.write_all(b"504 PROT level not supported\r\n")?,
            }
        }
        Ok(())
    }

    pub fn cmd_stat(&mut self) -> Result<()> {
        if let Some(ref username) = self.current_user {
            self.stream.write_all(b"211-FTP server status:\r\n")?;
            self.stream.write_all(format!("211-Connected to: {}\r\n", self.remote_ip).as_bytes())?;
            self.stream.write_all(format!("211-Logged in as: {}\r\n", username).as_bytes())?;
            self.stream.write_all(format!("211-Current directory: {}\r\n", self.cwd).as_bytes())?;
            self.stream.write_all(format!("211-Transfer mode: {}\r\n", if self.passive_mode { "Passive" } else { "Active" }).as_bytes())?;
            self.stream.write_all(b"211 End\r\n")?;
        } else {
            self.stream.write_all(b"211 FTP server status - Not logged in\r\n")?;
        }
        Ok(())
    }

    pub fn handle_abor(&mut self) -> Result<()> {
        self.abort_flag.store(true, Ordering::Relaxed);
        self.stream.write_all(b"426 Connection closed; transfer aborted\r\n")?;
        self.stream.write_all(b"226 Abort successful\r\n")?;
        Ok(())
    }
}
