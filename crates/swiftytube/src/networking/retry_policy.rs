use std::hash::{BuildHasher, RandomState};
use std::time::Duration;

/// Exponential backoff with jitter. First retry waits `base_delay`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub base_delay: Duration,
    pub factor: f64,
    pub max_delay: Duration,
    pub jitter: Duration,
}

impl RetryPolicy {
    pub const INNER_TUBE: Self = Self {
        max_attempts: 3,
        base_delay: Duration::from_millis(500),
        factor: 2.0,
        max_delay: Duration::from_secs(30),
        jitter: Duration::from_millis(200),
    };

    pub const SINGLE_ATTEMPT: Self = Self {
        max_attempts: 1,
        base_delay: Duration::ZERO,
        factor: 1.0,
        max_delay: Duration::ZERO,
        jitter: Duration::ZERO,
    };

    pub fn delay(&self, attempt: u32) -> Duration {
        let exponential = self.base_delay.as_secs_f64() * self.factor.powi(attempt as i32);
        let capped = Duration::from_secs_f64(exponential.min(self.max_delay.as_secs_f64()));
        let jitter_ms = self.jitter.as_millis() as u64;
        capped + Duration::from_millis(random_u64() % (jitter_ms + 1))
    }

    pub async fn run<T, E, F, Fut>(
        &self,
        mut operation: F,
        is_retryable: impl Fn(&E) -> bool,
        on_retry: impl Fn(u32, &E),
    ) -> Result<T, E>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, E>>,
        E: From<RetryError>,
    {
        let mut last_error = None;
        for attempt in 0..self.max_attempts {
            match operation().await {
                Ok(value) => return Ok(value),
                Err(error) => {
                    if attempt + 1 >= self.max_attempts || !is_retryable(&error) {
                        last_error = Some(error);
                        break;
                    }
                    on_retry(attempt, &error);
                    last_error = Some(error);
                    tokio::time::sleep(self.delay(attempt)).await;
                }
            }
        }
        Err(last_error.unwrap_or_else(|| RetryError::Exhausted.into()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RetryError {
    /// The policy allowed zero attempts, so nothing ran.
    #[error("retry policy allowed zero attempts")]
    Exhausted,
}

/// Non-crypto random number for jitter and request ids (Swift `Int.random`, `UUID`).
/// Each `RandomState` gets fresh keys, so no `rand` dependency is needed.
pub(crate) fn random_u64() -> u64 {
    RandomState::new().hash_one(std::time::SystemTime::now())
}
