use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct LoginAttempt {
    pub count: u32,
    pub first_attempt: Instant,
    pub banned_until: Option<Instant>,
}

pub struct LoginTracker {
    attempts: Mutex<HashMap<String, LoginAttempt>>,
    max_attempts: u32,
    ban_duration: Duration,
}

impl LoginTracker {
    #[must_use]
    pub fn new(max_attempts: u32, ban_duration_secs: u64) -> Self {
        Self {
            attempts: Mutex::new(HashMap::new()),
            max_attempts,
            ban_duration: Duration::from_secs(ban_duration_secs),
        }
    }

    pub fn check_and_record_failure(&self, ip: &str) -> bool {
        let mut attempts = self.attempts.lock().unwrap();

        if let Some(attempt) = attempts.get_mut(ip) {
            if let Some(banned_until) = attempt.banned_until {
                if Instant::now() < banned_until {
                    return false;
                }
                attempt.banned_until = None;
                attempt.count = 0;
            }

            attempt.count += 1;
            if attempt.count >= self.max_attempts {
                attempt.banned_until = Some(Instant::now() + self.ban_duration);
                return false;
            }
        } else {
            attempts.insert(
                ip.to_string(),
                LoginAttempt {
                    count: 1,
                    first_attempt: Instant::now(),
                    banned_until: None,
                },
            );
        }

        true
    }

    pub fn is_banned(&self, ip: &str) -> bool {
        let mut attempts = self.attempts.lock().unwrap();

        if let Some(attempt) = attempts.get_mut(ip)
            && let Some(banned_until) = attempt.banned_until
        {
            if Instant::now() < banned_until {
                return true;
            }
            attempt.banned_until = None;
            attempt.count = 0;
        }

        false
    }

    pub fn clear_attempts(&self, ip: &str) {
        let mut attempts = self.attempts.lock().unwrap();
        attempts.remove(ip);
    }

    pub fn get_remaining_attempts(&self, ip: &str) -> u32 {
        let attempts = self.attempts.lock().unwrap();
        if let Some(attempt) = attempts.get(ip) {
            if attempt.banned_until.is_some() {
                return 0;
            }
            self.max_attempts.saturating_sub(attempt.count)
        } else {
            self.max_attempts
        }
    }

    pub fn cleanup_expired(&self) {
        let mut attempts = self.attempts.lock().unwrap();
        let now = Instant::now();
        attempts.retain(|_, attempt| {
            if let Some(banned_until) = attempt.banned_until {
                now < banned_until
            } else {
                now.duration_since(attempt.first_attempt) < Duration::from_secs(3600)
            }
        });
    }
}
