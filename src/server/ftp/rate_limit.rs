use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct ConnectionInfo {
    pub first_seen: Instant,
    pub connection_count: u32,
    pub last_connection: Instant,
}

struct RateLimiterInner {
    connections: HashMap<String, ConnectionInfo>,
    current_total: u32,
}

pub struct RateLimiter {
    inner: Mutex<RateLimiterInner>,
    max_connections_per_ip: u32,
    window_duration: Duration,
    max_total_connections: u32,
}

impl RateLimiter {
    pub fn new(max_connections_per_ip: u32, window_secs: u64, max_total_connections: u32) -> Self {
        Self {
            inner: Mutex::new(RateLimiterInner {
                connections: HashMap::new(),
                current_total: 0,
            }),
            max_connections_per_ip,
            window_duration: Duration::from_secs(window_secs),
            max_total_connections,
        }
    }

    pub fn check_and_record(&self, ip: &str) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap();
        let now = Instant::now();
        
        inner.connections.retain(|_, info| {
            info.first_seen.elapsed() < self.window_duration
        });

        if inner.current_total >= self.max_total_connections {
            return Err("Server connection limit reached".to_string());
        }

        if let Some(info) = inner.connections.get_mut(ip) {
            if info.connection_count >= self.max_connections_per_ip {
                return Err(format!(
                    "Rate limit exceeded for {}: {} connections in {} seconds",
                    ip, self.max_connections_per_ip, self.window_duration.as_secs()
                ));
            }
            info.connection_count += 1;
            info.last_connection = now;
        } else {
            inner.connections.insert(ip.to_string(), ConnectionInfo {
                first_seen: now,
                connection_count: 1,
                last_connection: now,
            });
        }

        inner.current_total += 1;
        Ok(())
    }

    pub fn release_for_ip(&self, ip: &str) {
        let mut inner = self.inner.lock().unwrap();
        
        if inner.current_total > 0 {
            inner.current_total -= 1;
        }
        
        if let Some(info) = inner.connections.get_mut(ip) {
            if info.connection_count > 0 {
                info.connection_count -= 1;
            }
            if info.connection_count == 0 {
                inner.connections.remove(ip);
            }
        }
    }

    pub fn get_stats(&self) -> (usize, u32) {
        let inner = self.inner.lock().unwrap();
        (inner.connections.len(), inner.current_total)
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new(10, 60, 100)
    }
}
