use super::types::SWEEP_INTERVAL;

/// The sweep period for a service with this idle timeout: never longer than
/// the timeout itself, so a short timeout is honoured promptly.
pub fn sweep_interval(idle: std::time::Duration) -> std::time::Duration {
    SWEEP_INTERVAL
        .min(idle)
        .max(std::time::Duration::from_millis(100))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn the_sweep_never_waits_longer_than_the_idle_timeout() {
        assert_eq!(sweep_interval(Duration::from_secs(60)), SWEEP_INTERVAL);
        assert_eq!(
            sweep_interval(Duration::from_secs(1)),
            Duration::from_secs(1)
        );
        assert_eq!(
            sweep_interval(Duration::from_millis(1)),
            Duration::from_millis(100)
        );
    }
}
