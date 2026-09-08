use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct SpeedLimiter {
    last_check: Mutex<Instant>,
    tokens: Mutex<u64>,
    max_bytes_per_second: u64,
}

impl SpeedLimiter {
    #[must_use]
    pub fn new(max_kbps: u64) -> Self {
        Self {
            last_check: Mutex::new(Instant::now()),
            tokens: Mutex::new(0),
            max_bytes_per_second: max_kbps * 1024,
        }
    }

    #[must_use]
    pub fn with_unlimited() -> Self {
        Self {
            last_check: Mutex::new(Instant::now()),
            tokens: Mutex::new(0),
            max_bytes_per_second: 0,
        }
    }

    /// 按令牌桶限速：不足时异步等待补足
    ///
    /// # Panics
    /// 内部互斥锁中毒（持有线程 panic）时 panic
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
            // 纳秒级整数运算：elapsed * 速率 / 1e9，等价于旧的 f64 计算
            let replenished = u64::try_from(
                elapsed.as_nanos() * u128::from(self.max_bytes_per_second) / 1_000_000_000,
            )
            .unwrap_or(u64::MAX);
            *tokens = tokens.saturating_add(replenished);
            new_tokens = *tokens;
        }

        if new_tokens < bytes as u64 {
            let deficit = bytes as u128 - u128::from(new_tokens);
            let nanos =
                u64::try_from(deficit * 1_000_000_000 / u128::from(self.max_bytes_per_second))
                    .unwrap_or(u64::MAX);
            tokio::time::sleep(Duration::from_nanos(nanos)).await;
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
