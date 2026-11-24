use crate::model::OwnerLockState;
use std::time::{Duration, Instant};

const IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const PIN_ATTEMPT_LIMIT: u32 = 5;
const PIN_COOLDOWN: Duration = Duration::from_secs(30);

#[derive(Default)]
pub(super) struct OwnerSession {
    deadline: Option<Instant>,
    failed_attempts: u32,
    retry_at: Option<Instant>,
    epoch: u64,
}

impl OwnerSession {
    pub(super) fn status(&self, configured: bool, now: Instant) -> OwnerLockState {
        let remaining = self
            .deadline
            .and_then(|deadline| deadline.checked_duration_since(now));
        OwnerLockState {
            pin_configured: configured,
            unlocked: !configured || remaining.is_some_and(|time| !time.is_zero()),
            expires_in_seconds: if configured {
                remaining
                    .filter(|time| !time.is_zero())
                    .map_or(0, |time| time.as_secs().max(1))
            } else {
                0
            },
        }
    }

    pub(super) fn expired(&self, now: Instant) -> bool {
        self.deadline.is_some_and(|deadline| deadline <= now)
    }

    pub(super) fn unlock(&mut self, now: Instant) {
        self.deadline = Some(now + IDLE_TIMEOUT);
        self.failed_attempts = 0;
        self.retry_at = None;
    }

    pub(super) fn touch(&mut self, now: Instant) {
        if self.deadline.is_some_and(|deadline| deadline > now) {
            self.deadline = Some(now + IDLE_TIMEOUT);
        }
    }

    pub(super) fn lock(&mut self) {
        self.deadline = None;
        self.epoch = self.epoch.wrapping_add(1);
    }

    pub(super) fn epoch(&self) -> u64 {
        self.epoch
    }

    pub(super) fn retry_seconds(&mut self, now: Instant) -> Option<u64> {
        let retry_at = self.retry_at?;
        if retry_at > now {
            return Some((retry_at - now).as_millis().div_ceil(1000).max(1) as u64);
        }
        self.retry_at = None;
        self.failed_attempts = 0;
        None
    }

    pub(super) fn reject_pin(&mut self, now: Instant) -> Option<u64> {
        self.failed_attempts += 1;
        if self.failed_attempts >= PIN_ATTEMPT_LIMIT {
            self.retry_at = Some(now + PIN_COOLDOWN);
        }
        self.retry_seconds(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_checks_do_not_extend_idle_timeout_and_activity_cannot_unlock() {
        let now = Instant::now();
        let mut session = OwnerSession::default();
        assert!(session.status(false, now).unlocked);
        assert!(!session.status(true, now).unlocked);
        session.unlock(now);
        let activity = now + IDLE_TIMEOUT / 2;
        assert!(session.status(true, activity).unlocked);
        session.touch(activity);
        assert!(session.status(true, now + IDLE_TIMEOUT).unlocked);
        let deadline = activity + IDLE_TIMEOUT;
        assert!(session.expired(deadline));
        session.touch(deadline);
        assert!(!session.status(true, deadline).unlocked);
        let epoch = session.epoch();
        session.lock();
        session.unlock(deadline);
        assert_ne!(session.epoch(), epoch);
    }

    #[test]
    fn repeated_wrong_pins_trigger_cooldown_without_extension() {
        let now = Instant::now();
        let mut session = OwnerSession::default();
        for _ in 1..PIN_ATTEMPT_LIMIT {
            assert_eq!(session.reject_pin(now), None);
        }
        assert_eq!(session.reject_pin(now), Some(30));
        assert_eq!(
            session.retry_seconds(now + Duration::from_secs(1)),
            Some(29)
        );
        session.lock();
        assert_eq!(
            session.retry_seconds(now + Duration::from_secs(2)),
            Some(28)
        );
        let later = now + PIN_COOLDOWN;
        assert_eq!(session.retry_seconds(later), None);
        assert_eq!(session.reject_pin(later), None);
    }
}
