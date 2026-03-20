use anyhow::Result;
use std::collections::HashMap;
use tokio::net::TcpListener;

pub struct PassiveManager {
    listeners: HashMap<u16, TcpListener>,
}

impl PassiveManager {
    pub fn new() -> Self {
        PassiveManager {
            listeners: HashMap::new(),
        }
    }

    pub async fn try_bind_port(&mut self, port_min: u16, port_max: u16, bind_ip: &str) -> Result<u16> {
        for port in port_min..=port_max {
            if self.listeners.contains_key(&port) {
                continue;
            }
            
            let addr = format!("{}:{}", bind_ip, port);
            match TcpListener::bind(&addr).await {
                Ok(listener) => {
                    self.listeners.insert(port, listener);
                    return Ok(port);
                }
                Err(_) => {
                    continue;
                }
            }
        }

        anyhow::bail!(
            "No available passive ports in range {}-{}",
            port_min,
            port_max
        )
    }

    pub fn get_listener(&mut self, port: u16) -> Option<TcpListener> {
        self.listeners.remove(&port)
    }

    pub fn remove_listener(&mut self, port: u16) {
        self.listeners.remove(&port);
    }

    #[allow(dead_code)]
    pub fn clear(&mut self) {
        self.listeners.clear();
    }
}

impl Default for PassiveManager {
    fn default() -> Self {
        Self::new()
    }
}
