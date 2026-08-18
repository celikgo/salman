// SPDX-License-Identifier: Apache-2.0
//! The simulation clock.
//!
//! salman runs on a **virtual clock** by default. A ten-minute sequence
//! therefore tests in a fraction of a second, and — far more importantly — it
//! produces exactly the same trace every time, on every machine. A simulation
//! whose results depend on how busy the laptop was is not a test.
//!
//! Nothing in this module reads a wall clock. `SystemTime::now` appears nowhere
//! in the evaluation path, and the determinism gate is what enforces that.
//!
//! # Real-time mode
//!
//! A real-time mode exists for hardware-in-the-loop work, where the simulation
//! must keep pace with equipment that has its own opinion about time. It makes
//! no determinism promise, and it does not pretend to: it **measures its own
//! jitter and reports it**, because a general-purpose operating system cannot
//! give a control loop the guarantees a PLC does, and claiming otherwise would
//! be the kind of confident lie this project exists to avoid.

use salman_core::time::{DateTime, Duration};

/// Which clock a run is using.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClockMode {
    /// Time advances only when the scheduler says so. Deterministic.
    #[default]
    Virtual,
    /// Time advances with the host clock. Not deterministic, and salman
    /// reports the jitter it measured rather than claiming it was not there.
    RealTime,
}

/// The simulation's notion of now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clock {
    mode: ClockMode,
    /// Nanoseconds since the run started. Monotonic, never decreasing.
    elapsed_ns: i64,
    /// What `DATE_AND_TIME` reads at elapsed zero.
    ///
    /// Configured, never taken from the host, so that a program that stamps a
    /// record with the current time produces the same trace on every machine.
    epoch: DateTime,
    /// Highest single advance the clock has been asked to make. In real-time
    /// mode this is the observed scan jitter.
    max_advance_ns: i64,
    /// Number of advances, so a mean can be reported alongside the maximum.
    advances: u64,
    /// Total advanced, for the mean.
    total_advance_ns: i128,
}

impl Clock {
    /// A virtual clock starting at `epoch` with zero elapsed time.
    #[must_use]
    pub const fn virtual_from(epoch: DateTime) -> Self {
        Self {
            mode: ClockMode::Virtual,
            elapsed_ns: 0,
            epoch,
            max_advance_ns: 0,
            advances: 0,
            total_advance_ns: 0,
        }
    }

    /// A virtual clock whose wall-clock epoch is 1970-01-01T00:00:00.
    ///
    /// A fixed epoch rather than the host's date: a program that reads the
    /// date must produce the same trace tomorrow as it does today.
    #[must_use]
    pub const fn virtual_default() -> Self {
        Self::virtual_from(DateTime::EPOCH)
    }

    /// A real-time clock. Deterministic output is not promised in this mode.
    #[must_use]
    pub const fn real_time_from(epoch: DateTime) -> Self {
        Self {
            mode: ClockMode::RealTime,
            ..Self::virtual_from(epoch)
        }
    }

    /// Which mode this clock is in.
    #[must_use]
    pub const fn mode(&self) -> ClockMode {
        self.mode
    }

    /// Whether results from this clock are reproducible.
    #[must_use]
    pub const fn is_deterministic(&self) -> bool {
        matches!(self.mode, ClockMode::Virtual)
    }

    /// Nanoseconds since the run started.
    #[must_use]
    pub const fn elapsed_ns(&self) -> i64 {
        self.elapsed_ns
    }

    /// Time since the run started, as a duration.
    #[must_use]
    pub const fn elapsed(&self) -> Duration {
        Duration::from_nanos(self.elapsed_ns)
    }

    /// What the program sees when it reads `DATE_AND_TIME`.
    #[must_use]
    pub fn wall_clock(&self) -> DateTime {
        DateTime::from_nanos_since_epoch(
            self.epoch
                .nanos_since_epoch()
                .saturating_add(self.elapsed_ns),
        )
    }

    /// Advances the clock by `delta`.
    ///
    /// A negative or zero delta is ignored rather than rejected: the scheduler
    /// computes deltas from task periods, and a clock that could run backwards
    /// would make every elapsed-time calculation in the standard function
    /// blocks unsound. Time in a salman run only ever moves forward.
    pub const fn advance(&mut self, delta: Duration) {
        let ns = delta.nanos();
        if ns <= 0 {
            return;
        }
        self.elapsed_ns = self.elapsed_ns.saturating_add(ns);
        if ns > self.max_advance_ns {
            self.max_advance_ns = ns;
        }
        self.advances = self.advances.saturating_add(1);
        self.total_advance_ns = self.total_advance_ns.saturating_add(ns as i128);
    }

    /// Moves the clock directly to `elapsed_ns`, if that is not in the past.
    pub const fn advance_to_ns(&mut self, elapsed_ns: i64) {
        if elapsed_ns > self.elapsed_ns {
            let delta = elapsed_ns - self.elapsed_ns;
            self.advance(Duration::from_nanos(delta));
        }
    }

    /// The largest single advance so far.
    ///
    /// In real-time mode this is the worst scan-to-scan jitter observed, and
    /// it is what salman reports instead of a determinism claim it cannot make.
    #[must_use]
    pub const fn max_advance(&self) -> Duration {
        Duration::from_nanos(self.max_advance_ns)
    }

    /// The mean advance so far, or zero before the first advance.
    #[must_use]
    pub fn mean_advance(&self) -> Duration {
        if self.advances == 0 {
            return Duration::ZERO;
        }
        let mean = self.total_advance_ns / i128::from(self.advances);
        Duration::from_nanos(i64::try_from(mean).unwrap_or(i64::MAX))
    }

    /// How many times the clock has advanced.
    #[must_use]
    pub const fn advance_count(&self) -> u64 {
        self.advances
    }
}

impl Default for Clock {
    fn default() -> Self {
        Self::virtual_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_virtual_clock_starts_at_zero_and_is_deterministic() {
        let clock = Clock::virtual_default();
        assert_eq!(clock.elapsed_ns(), 0);
        assert!(clock.is_deterministic());
        assert_eq!(clock.mode(), ClockMode::Virtual);
    }

    #[test]
    fn the_wall_clock_comes_from_a_configured_epoch_not_from_the_host() {
        // A program that stamps a record with DT must produce the same trace
        // tomorrow as it does today.
        let clock = Clock::virtual_default();
        assert_eq!(clock.wall_clock(), DateTime::EPOCH);
        assert_eq!(
            clock.wall_clock().to_iec_literal(),
            "DT#1970-01-01-00:00:00"
        );
    }

    #[test]
    fn advancing_moves_both_elapsed_time_and_the_wall_clock() {
        let mut clock = Clock::virtual_default();
        clock.advance(Duration::from_secs(90).unwrap());
        assert_eq!(clock.elapsed().to_iec_literal(), "T#1m30s");
        assert_eq!(
            clock.wall_clock().to_iec_literal(),
            "DT#1970-01-01-00:01:30"
        );
    }

    #[test]
    fn the_clock_never_runs_backwards() {
        // Every elapsed-time calculation in the standard timers assumes
        // monotonic time. A clock that could go back would make TON's ET
        // negative, and nothing downstream checks for that.
        let mut clock = Clock::virtual_default();
        clock.advance(Duration::from_secs(10).unwrap());
        clock.advance(Duration::from_nanos(-5_000_000_000));
        assert_eq!(clock.elapsed_ns(), 10_000_000_000);
        clock.advance(Duration::ZERO);
        assert_eq!(clock.elapsed_ns(), 10_000_000_000);
    }

    #[test]
    fn advancing_to_a_past_time_does_nothing() {
        let mut clock = Clock::virtual_default();
        clock.advance_to_ns(1_000);
        clock.advance_to_ns(500);
        assert_eq!(clock.elapsed_ns(), 1_000);
    }

    #[test]
    fn advance_statistics_are_what_real_time_mode_reports_instead_of_a_promise() {
        let mut clock = Clock::real_time_from(DateTime::EPOCH);
        assert!(!clock.is_deterministic());
        clock.advance(Duration::from_nanos(10_000_000));
        clock.advance(Duration::from_nanos(12_000_000));
        clock.advance(Duration::from_nanos(8_000_000));
        assert_eq!(clock.max_advance().nanos(), 12_000_000);
        assert_eq!(clock.mean_advance().nanos(), 10_000_000);
        assert_eq!(clock.advance_count(), 3);
    }

    #[test]
    fn statistics_are_zero_before_the_first_advance() {
        let clock = Clock::virtual_default();
        assert_eq!(clock.max_advance(), Duration::ZERO);
        assert_eq!(clock.mean_advance(), Duration::ZERO);
    }

    #[test]
    fn the_clock_saturates_rather_than_wrapping_into_the_past() {
        let mut clock = Clock::virtual_default();
        clock.advance(Duration::MAX);
        clock.advance(Duration::MAX);
        assert_eq!(clock.elapsed_ns(), i64::MAX);
    }

    #[test]
    fn two_clocks_advanced_the_same_way_agree_exactly() {
        let mut a = Clock::virtual_default();
        let mut b = Clock::virtual_default();
        for ns in [1, 10, 999_999, 1_000_000_000, 7] {
            a.advance(Duration::from_nanos(ns));
            b.advance(Duration::from_nanos(ns));
        }
        assert_eq!(a, b);
    }
}
