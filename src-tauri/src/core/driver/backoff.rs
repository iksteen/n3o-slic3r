//! Shared reconnect/connect timing for all printer drivers.
//!
//! One policy across every driver (Bambu MQTT, U1 Moonraker WS):
//!
//!   - **Reconnect backoff:** exponential, `2^attempt` seconds, capped
//!     at [`RECONNECT_CAP_SECS`]. The attempt counter resets to 0 on
//!     every successful connect, so a printer that drops briefly comes
//!     back fast, while a persistently-unreachable one settles at the
//!     60s ceiling. Sequence: 1, 2, 4, 8, 16, 32, 60, 60, …
//!   - **Connect timeout:** a single connect attempt is bounded by
//!     [`CONNECT_TIMEOUT`] so a dead/unreachable host cycles back into
//!     backoff instead of hanging the reconnect worker.
//!
//! There is no max-attempt give-up: drivers reconnect until the user
//! tears the connection down (the registry unregisters the driver).

use std::time::Duration;

/// Cap on the reconnect delay.
pub const RECONNECT_CAP_SECS: u64 = 60;

/// Bound on a single connect attempt. Realistic for a LAN printer, and
/// short enough that an unreachable host fails fast into backoff rather
/// than stalling on the OS TCP timeout.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Delay before reconnect `attempt` (0-based), in seconds: `2^attempt`
/// capped at [`RECONNECT_CAP_SECS`]. Increment `attempt` per failure;
/// reset to 0 on a successful connect.
pub fn reconnect_backoff_secs(attempt: u32) -> u64 {
    2u64.checked_pow(attempt)
        .unwrap_or(RECONNECT_CAP_SECS)
        .min(RECONNECT_CAP_SECS)
}

/// [`reconnect_backoff_secs`] as a [`Duration`].
pub fn reconnect_backoff(attempt: u32) -> Duration {
    Duration::from_secs(reconnect_backoff_secs(attempt))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_is_exponential_capped_at_60() {
        assert_eq!(reconnect_backoff_secs(0), 1);
        assert_eq!(reconnect_backoff_secs(1), 2);
        assert_eq!(reconnect_backoff_secs(2), 4);
        assert_eq!(reconnect_backoff_secs(3), 8);
        assert_eq!(reconnect_backoff_secs(4), 16);
        assert_eq!(reconnect_backoff_secs(5), 32);
        // 2^6 = 64 → capped.
        assert_eq!(reconnect_backoff_secs(6), 60);
        assert_eq!(reconnect_backoff_secs(7), 60);
        // Large attempts overflow 2^n → still the cap, never a panic.
        assert_eq!(reconnect_backoff_secs(64), 60);
        assert_eq!(reconnect_backoff_secs(u32::MAX), 60);
    }
}
