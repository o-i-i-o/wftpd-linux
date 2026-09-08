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

    /// 记录一次失败尝试；达到阈值时封禁并返回 `false`
    ///
    /// # Panics
    /// 内部互斥锁中毒（持有线程 panic）时 panic
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

    /// 查询 IP 是否处于封禁期；封禁过期时顺带重置计数
    ///
    /// # Panics
    /// 内部互斥锁中毒（持有线程 panic）时 panic
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

    /// 清除 IP 的失败记录（登录成功后调用）
    ///
    /// # Panics
    /// 内部互斥锁中毒（持有线程 panic）时 panic
    pub fn clear_attempts(&self, ip: &str) {
        let mut attempts = self.attempts.lock().unwrap();
        attempts.remove(ip);
    }

    /// 剩余可尝试次数
    ///
    /// # Panics
    /// 内部互斥锁中毒（持有线程 panic）时 panic
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

    /// 清理过期条目（封禁结束或首条记录超过 1 小时）
    ///
    /// # Panics
    /// 内部互斥锁中毒（持有线程 panic）时 panic
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_below_threshold_allowed() {
        let tracker = LoginTracker::new(5, 60);
        for _ in 0..4 {
            assert!(tracker.check_and_record_failure("1.2.3.4"));
        }
        assert!(!tracker.is_banned("1.2.3.4"));
        assert_eq!(tracker.get_remaining_attempts("1.2.3.4"), 1);
    }

    #[test]
    fn reaching_threshold_bans_ip() {
        let tracker = LoginTracker::new(3, 60);
        assert!(tracker.check_and_record_failure("1.2.3.4"));
        assert!(tracker.check_and_record_failure("1.2.3.4"));
        // 第 3 次达到阈值：本次即返回 false 并封禁
        assert!(!tracker.check_and_record_failure("1.2.3.4"));
        assert!(tracker.is_banned("1.2.3.4"));
        assert_eq!(tracker.get_remaining_attempts("1.2.3.4"), 0);
        // 封禁期内继续失败仍被拒绝
        assert!(!tracker.check_and_record_failure("1.2.3.4"));
    }

    #[test]
    fn other_ips_unaffected() {
        let tracker = LoginTracker::new(2, 60);
        assert!(tracker.check_and_record_failure("1.1.1.1"));
        assert!(!tracker.check_and_record_failure("1.1.1.1"));
        assert!(tracker.is_banned("1.1.1.1"));
        assert!(!tracker.is_banned("2.2.2.2"));
        assert!(tracker.check_and_record_failure("2.2.2.2"));
    }

    #[test]
    fn unknown_ip_has_full_attempts() {
        let tracker = LoginTracker::new(4, 60);
        assert_eq!(tracker.get_remaining_attempts("9.9.9.9"), 4);
        assert!(!tracker.is_banned("9.9.9.9"));
    }

    #[test]
    fn clear_attempts_resets_state() {
        let tracker = LoginTracker::new(3, 60);
        tracker.check_and_record_failure("1.2.3.4");
        tracker.check_and_record_failure("1.2.3.4");
        tracker.clear_attempts("1.2.3.4");
        assert_eq!(tracker.get_remaining_attempts("1.2.3.4"), 3);
    }

    #[test]
    fn ban_expires_and_resets_count() {
        // 封禁时长 0：立刻过期，下次失败重新计数
        let tracker = LoginTracker::new(2, 0);
        assert!(tracker.check_and_record_failure("1.2.3.4"));
        assert!(!tracker.check_and_record_failure("1.2.3.4"));
        // banned_until = now，is_banned 检查过期并重置
        assert!(!tracker.is_banned("1.2.3.4"));
        assert_eq!(tracker.get_remaining_attempts("1.2.3.4"), 2);
    }

    #[test]
    fn cleanup_keeps_active_bans() {
        let tracker = LoginTracker::new(2, 3600);
        assert!(tracker.check_and_record_failure("1.2.3.4"));
        assert!(!tracker.check_and_record_failure("1.2.3.4"));

        tracker.cleanup_expired();
        assert!(tracker.is_banned("1.2.3.4"), "未过期的封禁应保留");
    }

    #[test]
    fn cleanup_keeps_fresh_entries() {
        let tracker = LoginTracker::new(10, 60);
        tracker.check_and_record_failure("1.2.3.4");
        tracker.cleanup_expired();
        assert_eq!(
            tracker.get_remaining_attempts("1.2.3.4"),
            9,
            "1 小时内的记录应保留"
        );
    }
}
