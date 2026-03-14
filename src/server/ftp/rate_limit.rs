use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct ConnectionInfo {
    pub first_seen: Instant,
    pub connection_count: u32,
    pub last_connection: Instant,
}

pub struct RateLimiter {
    connections: Mutex<HashMap<String, ConnectionInfo>>,
    max_connections_per_ip: u32,
    window_duration: Duration,
    max_total_connections: u32,
    current_total: Mutex<u32>,
}

impl RateLimiter {
    pub fn new(max_connections_per_ip: u32, window_secs: u64, max_total_connections: u32) -> Self {
        Self {
            connections: Mutex::new(HashMap::new()),
            max_connections_per_ip,
            window_duration: Duration::from_secs(window_secs),
            max_total_connections,
            current_total: Mutex::new(0),
        }
    }

    pub fn check_and_record(&self, ip: &str) -> Result<(), String> {
        let mut connections = self.connections.lock().unwrap();
        let mut current_total = self.current_total.lock().unwrap();
        
        connections.retain(|_, info| {
            info.first_seen.elapsed() < self.window_duration
        });

        if *current_total >= self.max_total_connections {
            return Err("Server connection limit reached".to_string());
        }

        let now = Instant::now();
        if let Some(info) = connections.get_mut(ip) {
            if info.connection_count >= self.max_connections_per_ip {
                return Err(format!(
                    "Rate limit exceeded for {}: {} connections in {} seconds",
                    ip, self.max_connections_per_ip, self.window_duration.as_secs()
                ));
            }
            info.connection_count += 1;
            info.last_connection = now;
        } else {
            connections.insert(ip.to_string(), ConnectionInfo {
                first_seen: now,
                connection_count: 1,
                last_connection: now,
            });
        }

        *current_total += 1;
        Ok(())
    }

    pub fn release(&self) {
        let mut current_total = self.current_total.lock().unwrap();
        if *current_total > 0 {
            *current_total -= 1;
        }
    }

    pub fn get_stats(&self) -> (usize, u32) {
        let connections = self.connections.lock().unwrap();
        let current_total = self.current_total.lock().unwrap();
        (connections.len(), *current_total)
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new(10, 60, 100)
    }
}
