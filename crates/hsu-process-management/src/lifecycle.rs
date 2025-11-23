// Process lifecycle management
// This module will handle process restart policies, startup sequences, and shutdown procedures

use crate::config::{RestartPolicyConfig, RestartStrategy};
#[allow(unused_imports)]
use hsu_common::{ProcessError, ProcessResult};
use chrono::{DateTime, Utc};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, info, warn};

/// Manages process restart policies and lifecycle events
#[derive(Debug, Clone)]
pub struct ProcessLifecycleManager {
    process_id: String,
    restart_policy: Option<RestartPolicyConfig>,
    restart_attempts: u32,
    last_restart_time: Option<DateTime<Utc>>,
    consecutive_failures: u32,
    circuit_breaker: CircuitBreaker,
}

/// Circuit breaker to prevent restart loops
#[derive(Debug, Clone)]
pub struct CircuitBreaker {
    /// Maximum failures before tripping
    failure_threshold: u32,
    /// Time window for counting failures
    time_window: Duration,
    /// Recent failure timestamps
    recent_failures: Vec<DateTime<Utc>>,
    /// Whether the circuit breaker is currently tripped
    is_tripped: bool,
    /// When the circuit breaker was tripped
    tripped_at: Option<DateTime<Utc>>,
    /// How long to wait before allowing retry
    cooldown_period: Duration,
}

impl CircuitBreaker {
    pub fn new() -> Self {
        Self {
            failure_threshold: 5,
            time_window: Duration::from_secs(60),
            recent_failures: Vec::new(),
            is_tripped: false,
            tripped_at: None,
            cooldown_period: Duration::from_secs(300), // 5 minutes
        }
    }
    
    /// Record a failure and check if circuit breaker should trip
    pub fn record_failure(&mut self) -> bool {
        let now = Utc::now();
        self.recent_failures.push(now);
        
        // Remove failures outside the time window
        let cutoff = now - chrono::Duration::from_std(self.time_window).unwrap();
        self.recent_failures.retain(|&failure_time| failure_time > cutoff);
        
        // Check if we should trip
        if self.recent_failures.len() >= self.failure_threshold as usize && !self.is_tripped {
            self.is_tripped = true;
            self.tripped_at = Some(now);
            warn!(
                "Circuit breaker tripped: {} failures in {:?} window",
                self.recent_failures.len(),
                self.time_window
            );
            return true;
        }
        
        false
    }
    
    /// Check if circuit breaker is currently tripped
    pub fn is_tripped(&self) -> bool {
        if !self.is_tripped {
            return false;
        }
        
        // Check if cooldown period has passed
        if let Some(tripped_at) = self.tripped_at {
            let now = Utc::now();
            let elapsed = (now - tripped_at).to_std().unwrap_or(Duration::from_secs(0));
            
            if elapsed >= self.cooldown_period {
                // Cooldown passed, but we don't auto-reset here
                // The caller should explicitly reset after successful operation
                return false;
            }
        }
        
        true
    }
    
    /// Reset the circuit breaker
    pub fn reset(&mut self) {
        self.is_tripped = false;
        self.tripped_at = None;
        self.recent_failures.clear();
        info!("Circuit breaker reset");
    }
    
    /// Get remaining cooldown time
    pub fn remaining_cooldown(&self) -> Option<Duration> {
        if !self.is_tripped {
            return None;
        }
        
        if let Some(tripped_at) = self.tripped_at {
            let now = Utc::now();
            let elapsed = (now - tripped_at).to_std().unwrap_or(Duration::from_secs(0));
            
            if elapsed < self.cooldown_period {
                return Some(self.cooldown_period - elapsed);
            }
        }
        
        None
    }
}

impl ProcessLifecycleManager {
    pub fn new(process_id: String, restart_policy: Option<RestartPolicyConfig>) -> Self {
        Self {
            process_id,
            restart_policy,
            restart_attempts: 0,
            last_restart_time: None,
            consecutive_failures: 0,
            circuit_breaker: CircuitBreaker::new(),
        }
    }

    /// Check if restart is allowed (without exit code - for health checks)
    /// This is a synchronous, simple policy check
    pub fn should_restart(&self) -> bool {
        // Check circuit breaker first
        if self.circuit_breaker.is_tripped() {
            return false;
        }
        
        // Check restart policy
        match &self.restart_policy {
            Some(policy) => {
                // Check restart attempts
                if self.restart_attempts >= policy.max_attempts {
                    return false;
                }
                
                // Check policy strategy
                match policy.strategy {
                    RestartStrategy::Never => false,
                    RestartStrategy::Always | RestartStrategy::OnFailure => true,
                }
            }
            None => false, // No restart policy
        }
    }
    
    /// Check if the process should be restarted based on the restart policy and exit code
    pub async fn should_restart_with_exit_code(&mut self, exit_code: Option<i32>) -> bool {
        // Check circuit breaker first
        if self.circuit_breaker.is_tripped() {
            if let Some(remaining) = self.circuit_breaker.remaining_cooldown() {
                warn!(
                    "Circuit breaker is tripped for process {}, cooldown remaining: {:?}",
                    self.process_id, remaining
                );
            } else {
                info!("Circuit breaker cooldown expired for process {}, ready for reset", self.process_id);
            }
            return false;
        }
        
        let policy = match &self.restart_policy {
            Some(policy) => policy.clone(),
            None => {
                debug!("No restart policy configured for process: {}", self.process_id);
                return false;
            }
        };

        match policy.strategy {
            RestartStrategy::Never => {
                debug!("Restart policy is 'never' for process: {}", self.process_id);
                false
            }
            RestartStrategy::Always => {
                self.check_restart_limits(&policy).await
            }
            RestartStrategy::OnFailure => {
                if let Some(code) = exit_code {
                    if code == 0 {
                        debug!("Process {} exited successfully, not restarting", self.process_id);
                        // Reset circuit breaker on successful exit
                        self.circuit_breaker.reset();
                        return false;
                    }
                }
                self.check_restart_limits(&policy).await
            }
        }
    }

    /// Check if restart attempts are within limits
    async fn check_restart_limits(&mut self, policy: &RestartPolicyConfig) -> bool {
        if self.restart_attempts >= policy.max_attempts {
            warn!(
                "Process {} has exceeded maximum restart attempts ({}/{})",
                self.process_id, self.restart_attempts, policy.max_attempts
            );
            return false;
        }

        // Calculate restart delay with backoff
        let delay = self.calculate_restart_delay(policy);
        
        info!(
            "Restarting process {} in {:?} (attempt {}/{})",
            self.process_id, delay, self.restart_attempts + 1, policy.max_attempts
        );

        sleep(delay).await;
        
        self.restart_attempts += 1;
        self.last_restart_time = Some(Utc::now());
        true
    }

    /// Calculate restart delay with exponential backoff
    fn calculate_restart_delay(&self, policy: &RestartPolicyConfig) -> Duration {
        let base_delay = policy.restart_delay;
        let multiplier = policy.backoff_multiplier.powf(self.restart_attempts as f32);
        
        let delay_secs = base_delay.as_secs_f64() * multiplier as f64;
        Duration::from_secs_f64(delay_secs.min(300.0)) // Cap at 5 minutes
    }

    /// Reset restart counters (called on successful restart)
    pub fn reset_restart_counters(&mut self) {
        self.restart_attempts = 0;
        self.consecutive_failures = 0;
        self.circuit_breaker.reset();
        debug!("Reset restart counters for process: {}", self.process_id);
    }

    /// Record a failure
    pub fn record_failure(&mut self) {
        self.consecutive_failures += 1;
        let tripped = self.circuit_breaker.record_failure();
        
        if tripped {
            warn!(
                "Circuit breaker TRIPPED for process: {} after {} consecutive failures",
                self.process_id, self.consecutive_failures
            );
        } else {
            debug!(
                "Recorded failure for process: {} (consecutive: {})",
                self.process_id, self.consecutive_failures
            );
        }
    }
    
    /// Check if circuit breaker is tripped
    pub fn is_circuit_breaker_tripped(&self) -> bool {
        self.circuit_breaker.is_tripped()
    }
    
    /// Get remaining cooldown time
    pub fn get_remaining_cooldown(&self) -> Option<Duration> {
        self.circuit_breaker.remaining_cooldown()
    }
    
    /// Manually reset circuit breaker (admin override)
    pub fn reset_circuit_breaker(&mut self) {
        self.circuit_breaker.reset();
        info!("Circuit breaker manually reset for process: {}", self.process_id);
    }

    /// Get restart statistics
    pub fn get_restart_stats(&self) -> RestartStats {
        RestartStats {
            restart_attempts: self.restart_attempts,
            last_restart_time: self.last_restart_time,
            consecutive_failures: self.consecutive_failures,
        }
    }
}

/// Restart statistics for monitoring
#[derive(Debug, Clone)]
pub struct RestartStats {
    pub restart_attempts: u32,
    pub last_restart_time: Option<DateTime<Utc>>,
    pub consecutive_failures: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{RestartPolicyConfig, RestartStrategy};

    fn create_test_policy() -> RestartPolicyConfig {
        RestartPolicyConfig {
            strategy: RestartStrategy::OnFailure,
            max_attempts: 3,
            restart_delay: Duration::from_secs(1),
            backoff_multiplier: 2.0,
        }
    }

    #[tokio::test]
    async fn test_restart_on_failure() {
        let policy = create_test_policy();
        let mut manager = ProcessLifecycleManager::new("test".to_string(), Some(policy));

        // Should restart on non-zero exit code
        assert!(manager.should_restart_with_exit_code(Some(1)).await);
        assert_eq!(manager.restart_attempts, 1);

        // Should not restart on zero exit code
        assert!(!manager.should_restart_with_exit_code(Some(0)).await);
    }

    #[tokio::test]
    async fn test_max_restart_attempts() {
        let policy = create_test_policy();
        let mut manager = ProcessLifecycleManager::new("test".to_string(), Some(policy));

        // First 3 attempts should succeed
        for i in 1..=3 {
            assert!(manager.should_restart_with_exit_code(Some(1)).await);
            assert_eq!(manager.restart_attempts, i);
        }

        // 4th attempt should fail (max_attempts is 3)
        assert!(!manager.should_restart_with_exit_code(Some(1)).await);
    }

    #[test]
    fn test_restart_delay_calculation() {
        let policy = create_test_policy();
        let mut manager = ProcessLifecycleManager::new("test".to_string(), Some(policy.clone()));

        // First restart: base delay
        let delay1 = manager.calculate_restart_delay(&policy);
        assert_eq!(delay1, Duration::from_secs(1));

        // Second restart: base delay * multiplier
        manager.restart_attempts = 1;
        let delay2 = manager.calculate_restart_delay(&policy);
        assert_eq!(delay2, Duration::from_secs(2));

        // Third restart: base delay * multiplier^2
        manager.restart_attempts = 2;
        let delay3 = manager.calculate_restart_delay(&policy);
        assert_eq!(delay3, Duration::from_secs(4));
    }
}
