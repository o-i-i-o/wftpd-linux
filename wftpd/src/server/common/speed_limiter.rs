use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct SpeedLimiter {
    last_check: Mutex<Instant>,
    tokens: Mutex<u64>,
    max_bytes_per_second: u64,
}

impl SpeedLimiter {
    pub fn new(max_kbps: u64) -> Self {
        Self {
            last_check: Mutex::new(Instant::now()),
            tokens: Mutex::new(0),
            max_bytes_per_second: max_kbps * 1024,
        }
    }

    pub fn with_unlimited() -> Self {
        Self {
            last_check: Mutex::new(Instant::now()),
            tokens: Mutex::new(0),
            max_bytes_per_second: 0,
        }
    }

    pub async fn throttle(&self, bytes: usize) {
        if self.max_bytes_per_second == 0 || bytes == 0 {
            return;
        }

        let now = Instant::now();
        let elapsed;
        let new_tokens;
        
        {
            let mut last = self.last_check.lock().unwrap();
            elapsed = now.duration_since(*last);
            *last = now;
        }

        {
            let mut tokens = self.tokens.lock().unwrap();
            let replenished = (elapsed.as_secs_f64() * self.max_bytes_per_second as f64) as u64;
            *tokens = tokens.saturating_add(replenished);
            new_tokens = *tokens;
        }

        if new_tokens < bytes as u64 {
            let deficit = bytes as u64 - new_tokens;
            let wait_time = Duration::from_micros(
                (deficit as f64 / self.max_bytes_per_second as f64 * 1_000_000.0) as u64
            );
            tokio::time::sleep(wait_time).await;
        }

        {
            let mut tokens = self.tokens.lock().unwrap();
            *tokens = tokens.saturating_sub(bytes as u64);
        }
    }

    pub fn set_limit(&mut self, max_kbps: u64) {
        self.max_bytes_per_second = max_kbps * 1024;
    }

    pub fn get_limit(&self) -> u64 {
        self.max_bytes_per_second / 1024
    }
}

impl Default for SpeedLimiter {
    fn default() -> Self {
        Self::with_unlimited()
    }
}

impl Clone for SpeedLimiter {
    fn clone(&self) -> Self {
        Self {
            last_check: Mutex::new(Instant::now()),
            tokens: Mutex::new(0),
            max_bytes_per_second: self.max_bytes_per_second,
        }
    }
}
