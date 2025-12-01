//! ETA calculation using Exponentially Weighted Moving Average (EWMA)
//!
//! This module provides ETA estimation for backfill jobs based on historical
//! partition execution times. Uses EWMA to smooth out variations and provide
//! more accurate estimates.

use chrono::{DateTime, Duration, Utc};

/// ETA calculator using Exponentially Weighted Moving Average
///
/// EWMA gives more weight to recent observations, making it responsive to
/// changing conditions while smoothing out noise.
#[derive(Debug, Clone)]
pub struct EtaCalculator {
    /// Smoothing factor (0 < alpha <= 1)
    /// Higher values give more weight to recent observations
    /// Recommended: 0.3 for stable workloads, 0.5 for variable workloads
    alpha: f64,
    /// Current EWMA estimate of partition duration in milliseconds
    avg_duration_ms: f64,
    /// Number of observations incorporated
    observation_count: u32,
}

impl Default for EtaCalculator {
    fn default() -> Self {
        Self::new(0.3)
    }
}

impl EtaCalculator {
    /// Create a new ETA calculator with the specified smoothing factor
    ///
    /// # Arguments
    /// * `alpha` - Smoothing factor (0 < alpha <= 1). Recommended: 0.3
    pub fn new(alpha: f64) -> Self {
        Self {
            alpha: alpha.clamp(0.01, 1.0),
            avg_duration_ms: 0.0,
            observation_count: 0,
        }
    }

    /// Create an ETA calculator initialized with a previous average
    ///
    /// Useful when resuming from a checkpoint with existing statistics
    pub fn with_initial_avg(alpha: f64, initial_avg_ms: i64) -> Self {
        Self {
            alpha: alpha.clamp(0.01, 1.0),
            avg_duration_ms: initial_avg_ms as f64,
            observation_count: if initial_avg_ms > 0 { 1 } else { 0 },
        }
    }

    /// Update the EWMA with a new partition duration observation
    ///
    /// # Arguments
    /// * `duration_ms` - The duration of the completed partition in milliseconds
    pub fn update(&mut self, duration_ms: u64) {
        let duration = duration_ms as f64;

        if self.observation_count == 0 {
            // First observation - use it directly
            self.avg_duration_ms = duration;
        } else {
            // EWMA formula: new_avg = alpha * current + (1 - alpha) * old_avg
            self.avg_duration_ms =
                self.alpha * duration + (1.0 - self.alpha) * self.avg_duration_ms;
        }

        self.observation_count += 1;
    }

    /// Get the current average partition duration in milliseconds
    pub fn avg_duration_ms(&self) -> i64 {
        self.avg_duration_ms.round() as i64
    }

    /// Estimate the remaining time for a backfill job
    ///
    /// # Arguments
    /// * `remaining_partitions` - Number of partitions left to process
    ///
    /// # Returns
    /// * `Some(Duration)` - Estimated remaining time
    /// * `None` - If no observations have been made yet
    pub fn estimate_remaining(&self, remaining_partitions: u32) -> Option<Duration> {
        if self.observation_count == 0 {
            return None;
        }

        let remaining_ms = self.avg_duration_ms * remaining_partitions as f64;
        Some(Duration::milliseconds(remaining_ms.round() as i64))
    }

    /// Calculate the estimated completion time
    ///
    /// # Arguments
    /// * `remaining_partitions` - Number of partitions left to process
    ///
    /// # Returns
    /// * `Some(DateTime<Utc>)` - Estimated completion time
    /// * `None` - If no observations have been made yet
    pub fn estimate_completion_time(&self, remaining_partitions: u32) -> Option<DateTime<Utc>> {
        self.estimate_remaining(remaining_partitions)
            .map(|duration| Utc::now() + duration)
    }

    /// Get the number of observations incorporated into the estimate
    pub fn observation_count(&self) -> u32 {
        self.observation_count
    }

    /// Check if the calculator has enough data for reliable estimates
    ///
    /// Returns true if at least 3 observations have been made
    pub fn has_reliable_estimate(&self) -> bool {
        self.observation_count >= 3
    }

    /// Calculate the estimated processing rate (partitions per minute)
    pub fn partitions_per_minute(&self) -> Option<f64> {
        if self.observation_count == 0 || self.avg_duration_ms <= 0.0 {
            return None;
        }

        // Convert ms per partition to partitions per minute
        Some(60_000.0 / self.avg_duration_ms)
    }
}

/// Format a duration for human-readable display
pub fn format_duration_human(duration: Duration) -> String {
    let total_secs = duration.num_seconds();

    if total_secs < 60 {
        format!("{}s", total_secs)
    } else if total_secs < 3600 {
        let mins = total_secs / 60;
        let secs = total_secs % 60;
        if secs == 0 {
            format!("{}m", mins)
        } else {
            format!("{}m {}s", mins, secs)
        }
    } else {
        let hours = total_secs / 3600;
        let mins = (total_secs % 3600) / 60;
        if mins == 0 {
            format!("{}h", hours)
        } else {
            format!("{}h {}m", hours, mins)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_eta_calculator_first_observation() {
        let mut calc = EtaCalculator::new(0.3);
        assert_eq!(calc.observation_count(), 0);
        assert!(calc.estimate_remaining(10).is_none());

        calc.update(1000); // 1 second
        assert_eq!(calc.observation_count(), 1);
        assert_eq!(calc.avg_duration_ms(), 1000);
    }

    #[test]
    fn test_eta_calculator_ewma() {
        let mut calc = EtaCalculator::new(0.5); // 50% weight to new observations

        calc.update(1000); // First: avg = 1000
        assert_eq!(calc.avg_duration_ms(), 1000);

        calc.update(2000); // Second: avg = 0.5 * 2000 + 0.5 * 1000 = 1500
        assert_eq!(calc.avg_duration_ms(), 1500);

        calc.update(2000); // Third: avg = 0.5 * 2000 + 0.5 * 1500 = 1750
        assert_eq!(calc.avg_duration_ms(), 1750);
    }

    #[test]
    fn test_estimate_remaining() {
        let mut calc = EtaCalculator::new(0.3);
        calc.update(1000); // 1 second per partition

        let remaining = calc.estimate_remaining(10).unwrap();
        assert_eq!(remaining.num_seconds(), 10);
    }

    #[test]
    fn test_partitions_per_minute() {
        let mut calc = EtaCalculator::new(0.3);
        calc.update(1000); // 1 second per partition = 60 per minute

        let rate = calc.partitions_per_minute().unwrap();
        assert!((rate - 60.0).abs() < 0.01);
    }

    #[test]
    fn test_with_initial_avg() {
        let calc = EtaCalculator::with_initial_avg(0.3, 2000);
        assert_eq!(calc.avg_duration_ms(), 2000);
        assert_eq!(calc.observation_count(), 1);
    }

    #[test]
    fn test_format_duration_human_seconds() {
        assert_eq!(format_duration_human(Duration::seconds(45)), "45s");
    }

    #[test]
    fn test_format_duration_human_minutes() {
        assert_eq!(format_duration_human(Duration::seconds(90)), "1m 30s");
        assert_eq!(format_duration_human(Duration::seconds(120)), "2m");
    }

    #[test]
    fn test_format_duration_human_hours() {
        assert_eq!(format_duration_human(Duration::seconds(3660)), "1h 1m");
        assert_eq!(format_duration_human(Duration::seconds(7200)), "2h");
    }

    #[test]
    fn test_has_reliable_estimate() {
        let mut calc = EtaCalculator::new(0.3);
        assert!(!calc.has_reliable_estimate());

        calc.update(1000);
        calc.update(1000);
        assert!(!calc.has_reliable_estimate());

        calc.update(1000);
        assert!(calc.has_reliable_estimate());
    }

    #[test]
    fn test_eta_persistence_across_pause_resume() {
        // Simulate a job that's paused after processing some partitions
        let mut calc = EtaCalculator::new(0.3);

        // Process 5 partitions with ~1000ms each
        calc.update(1000);
        calc.update(1100);
        calc.update(900);
        calc.update(1050);
        calc.update(950);

        // Save the avg_duration_ms (this would be persisted to DB on pause)
        let saved_avg = calc.avg_duration_ms();
        assert!(saved_avg > 0);

        // Simulate resumption by creating a new calculator with saved average
        let resumed_calc = EtaCalculator::with_initial_avg(0.3, saved_avg);

        // Verify the estimate is preserved
        assert_eq!(resumed_calc.avg_duration_ms(), saved_avg);
        assert!(resumed_calc.has_reliable_estimate() || resumed_calc.observation_count() >= 1);

        // Verify ETA calculation works after resume
        let remaining = resumed_calc.estimate_remaining(10);
        assert!(remaining.is_some());
        // 10 partitions * ~1000ms = ~10s
        let eta_secs = remaining.unwrap().num_seconds();
        assert!(
            (8..=12).contains(&eta_secs),
            "ETA should be ~10s, got {}s",
            eta_secs
        );
    }

    #[test]
    fn test_eta_accuracy_with_variable_durations() {
        let mut calc = EtaCalculator::new(0.3);

        // Simulate partitions with increasing duration (e.g., larger data later)
        calc.update(500); // Fast partitions early
        calc.update(600);
        calc.update(800);
        calc.update(1000);
        calc.update(1200); // Slower partitions later

        // EWMA should weight recent (slower) partitions more heavily
        let avg = calc.avg_duration_ms();
        // With alpha=0.3, the average should be closer to recent values
        // than a simple mean would suggest
        assert!(
            avg > 700,
            "EWMA should be pulled toward recent values, got {}",
            avg
        );
    }

    #[test]
    fn test_estimate_completion_time_returns_future() {
        let mut calc = EtaCalculator::new(0.3);
        calc.update(1000);

        let completion = calc.estimate_completion_time(5);
        assert!(completion.is_some());

        let now = Utc::now();
        let eta = completion.unwrap();
        assert!(eta > now, "Estimated completion should be in the future");
    }
}
