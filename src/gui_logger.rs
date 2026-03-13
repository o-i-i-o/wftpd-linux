use log::Level;

pub struct GuiLogger {
    _initialized: bool,
}

impl GuiLogger {
    pub fn new() -> Self {
        let initialized = systemd_journal_logger::JournalLog::new()
            .map(|j| j.with_syslog_identifier("wftp-gui".to_string()))
            .map(|j| j.install())
            .is_ok();
        
        Self { _initialized: initialized }
    }
    
    pub fn info(&self, source: &str, message: &str) {
        self.log(Level::Info, source, message);
    }
    
    pub fn warn(&self, source: &str, message: &str) {
        self.log(Level::Warn, source, message);
    }
    
    pub fn error(&self, source: &str, message: &str) {
        self.log(Level::Error, source, message);
    }
    
    pub fn debug(&self, source: &str, message: &str) {
        self.log(Level::Debug, source, message);
    }
    
    fn log(&self, level: Level, source: &str, message: &str) {
        let msg = format!("[{}] {}", source, message);
        
        match level {
            Level::Error => log::error!("{}", msg),
            Level::Warn => log::warn!("{}", msg),
            Level::Info => log::info!("{}", msg),
            Level::Debug => log::debug!("{}", msg),
            Level::Trace => log::trace!("{}", msg),
        }
        
        println!("[{}] {}", level, msg);
    }
}

impl Default for GuiLogger {
    fn default() -> Self {
        Self::new()
    }
}
