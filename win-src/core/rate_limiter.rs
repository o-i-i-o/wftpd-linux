use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const BUCKET_CAPACITY: u64 = 64 * 1024;
const REFILL_INTERVAL_MS: u64 = 100;

pub struct RateLimiter {
    state: Arc<Mutex<RateLimiterState>>,
    bytes_per_second: u64,
}

struct RateLimiterState {
    tokens: u64,
    last_refill: Instant,
}

impl RateLimiter {
    pub fn new(speed_limit_kbps: u64) -> Self {
        let bytes_per_second = if speed_limit_kbps == 0 {
            u64::MAX
        } else {
            speed_limit_kbps * 1024
        };
        
        let tokens_per_interval = bytes_per_second / (1000 / REFILL_INTERVAL_MS);
        
        RateLimiter {
            state: Arc::new(Mutex::new(RateLimiterState {
                tokens: tokens_per_interval.min(BUCKET_CAPACITY),
                last_refill: Instant::now(),
            })),
            bytes_per_second,
        }
    }

    pub fn is_unlimited(&self) -> bool {
        self.bytes_per_second == u64::MAX
    }

    pub async fn acquire(&self, bytes: usize) {
        if self.is_unlimited() {
            return;
        }
        
        let mut remaining = bytes as u64;
        
        while remaining > 0 {
            let wait_duration = {
                let mut state = self.state.lock().await;
                
                Self::refill_tokens(&mut state, self.bytes_per_second);
                
                if state.tokens > 0 {
                    let to_consume = remaining.min(state.tokens);
                    state.tokens -= to_consume;
                    remaining -= to_consume;
                    None
                } else {
                    let tokens_per_interval = self.bytes_per_second / (1000 / REFILL_INTERVAL_MS);
                    let refill_amount = tokens_per_interval.min(BUCKET_CAPACITY);
                    let wait_ms = if self.bytes_per_second > 0 {
                        (refill_amount as f64 / self.bytes_per_second as f64 * 1000.0) as u64
                    } else {
                        REFILL_INTERVAL_MS
                    };
                    Some(Duration::from_millis(wait_ms.max(1)))
                }
            };
            
            if let Some(duration) = wait_duration {
                tokio::time::sleep(duration).await;
            } else if remaining == 0 {
                break;
            }
        }
    }

    fn refill_tokens(state: &mut RateLimiterState, bytes_per_second: u64) {
        let now = Instant::now();
        let elapsed = now.duration_since(state.last_refill);
        
        if elapsed >= Duration::from_millis(REFILL_INTERVAL_MS) {
            let intervals = elapsed.as_millis() as u64 / REFILL_INTERVAL_MS;
            let tokens_per_interval = bytes_per_second / (1000 / REFILL_INTERVAL_MS);
            let tokens_to_add = intervals * tokens_per_interval;
            
            state.tokens = state.tokens.saturating_add(tokens_to_add).min(BUCKET_CAPACITY);
            state.last_refill = now;
        }
    }

    pub async fn get_available_tokens(&self) -> u64 {
        let mut state = self.state.lock().await;
        Self::refill_tokens(&mut state, self.bytes_per_second);
        state.tokens
    }
}

pub struct TransferRateTracker {
    start_time: Instant,
    total_bytes: u64,
}

impl TransferRateTracker {
    pub fn new() -> Self {
        TransferRateTracker {
            start_time: Instant::now(),
            total_bytes: 0,
        }
    }

    pub fn add_bytes(&mut self, bytes: u64) {
        self.total_bytes += bytes;
    }

    pub fn get_rate_kbps(&self) -> u64 {
        let elapsed_secs = self.start_time.elapsed().as_secs_f64();
        if elapsed_secs > 0.0 {
            let bytes_per_sec = self.total_bytes as f64 / elapsed_secs;
            (bytes_per_sec / 1024.0) as u64
        } else {
            0
        }
    }

    pub fn get_total_bytes(&self) -> u64 {
        self.total_bytes
    }

    pub fn get_elapsed_secs(&self) -> u64 {
        self.start_time.elapsed().as_secs()
    }
}

impl Default for TransferRateTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
pub struct RateLimitConfig {
    pub speed_limit_kbps: u64,
}

impl RateLimitConfig {
    pub fn new(speed_limit_kbps: u64) -> Self {
        RateLimitConfig { speed_limit_kbps }
    }

    pub fn unlimited() -> Self {
        RateLimitConfig { speed_limit_kbps: 0 }
    }

    pub fn create_limiter(&self) -> RateLimiter {
        RateLimiter::new(self.speed_limit_kbps)
    }
}
