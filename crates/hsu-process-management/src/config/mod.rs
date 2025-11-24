use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;
use anyhow::{Context, Result};

pub mod validation;

/// Top-level configuration structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessManagerConfig {
    pub process_manager: ProcessManagerOptions,
    pub managed_processes: Vec<ProcessConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_collection: Option<LogCollectionConfig>,
}

/// Process manager configuration options
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessManagerOptions {
    pub port: u16,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(
        default = "default_force_shutdown_timeout",
        with = "duration_serde"
    )]
    pub force_shutdown_timeout: Duration,
    
    /// Optional base directory override for PID files and log files
    /// When specified, all PID files and log files will be placed under this directory
    /// This is particularly useful for E2E tests to isolate artifacts
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_directory: Option<String>,
}

/// Individual process configuration (matches Go's ProcessConfig)
#[derive(Debug, Clone, Serialize, Deserialize, Hash)]
pub struct ProcessConfig {
    pub id: String,
    #[serde(rename = "type")]
    pub process_type: ProcessManagementType,
    #[serde(default = "default_profile_type")]
    pub profile_type: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub management: ProcessManagementConfig,
}

/// Process management type enumeration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ProcessManagementType {
    StandardManaged,
    IntegratedManaged,
    Unmanaged,
}

/// Process management configuration - union type for different process types
/// Matches Go's ProcessManagementConfig structure
#[derive(Debug, Clone, Serialize, Deserialize, Hash)]
pub struct ProcessManagementConfig {
    // Only one of these should be populated based on ProcessConfig.Type
    #[serde(skip_serializing_if = "Option::is_none")]
    pub standard_managed: Option<StandardManagedProcessConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integrated_managed: Option<IntegratedManagedProcessConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unmanaged: Option<UnmanagedProcessConfig>,
}

/// Standard managed process configuration (matches Go)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StandardManagedProcessConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<ProcessMetadata>,
    pub control: ManagedProcessControlConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health_check: Option<HealthCheckConfig>,
}

impl std::hash::Hash for StandardManagedProcessConfig {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Hash control config only for process identity
        self.control.hash(state);
    }
}

/// Integrated managed process configuration (matches Go)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegratedManagedProcessConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<ProcessMetadata>,
    pub control: ManagedProcessControlConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health_check: Option<HealthCheckConfig>,
}

impl std::hash::Hash for IntegratedManagedProcessConfig {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.control.hash(state);
    }
}

/// Unmanaged process configuration (matches Go)
#[derive(Debug, Clone, Serialize, Deserialize, Hash)]
pub struct UnmanagedProcessConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<ProcessMetadata>,
    pub discovery: ProcessDiscoveryConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control: Option<UnmanagedProcessControlConfig>,
}

/// Process metadata (matches Go)
#[derive(Debug, Clone, Serialize, Deserialize, Hash)]
pub struct ProcessMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Managed process control configuration (matches Go's ManagedProcessControlConfig)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedProcessControlConfig {
    /// Process execution configuration
    pub execution: ExecutionConfig,
    
    /// PID file configuration (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_file: Option<ProcessFileConfig>,
    
    /// Context-aware restart configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_aware_restart: Option<ContextAwareRestartConfig>,
    
    /// Simple restart policy
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restart_policy: Option<String>,
    
    /// Resource limits
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limits: Option<ResourceLimitsConfig>,
    
    /// Graceful shutdown timeout
    #[serde(
        skip_serializing_if = "Option::is_none",
        with = "option_duration_serde"
    )]
    pub graceful_timeout: Option<Duration>,
    
    /// Log collection configuration (process-specific)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_collection: Option<ProcessLogCollectionConfig>,
}

impl std::hash::Hash for ManagedProcessControlConfig {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.execution.hash(state);
        // Only hash execution for identity
    }
}

/// Process execution configuration (matches Go's process.ExecutionConfig)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionConfig {
    pub executable_path: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
    #[serde(default)]
    pub environment: HashMap<String, String>,
    #[serde(
        default = "default_wait_delay",
        with = "duration_serde"
    )]
    pub wait_delay: Duration,
}

impl std::hash::Hash for ExecutionConfig {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.executable_path.hash(state);
        self.args.hash(state);
        self.working_directory.hash(state);
        // Hash environment as sorted vec
        let mut env_vec: Vec<_> = self.environment.iter().collect();
        env_vec.sort_by_key(|(k, _)| *k);
        env_vec.hash(state);
        self.wait_delay.hash(state);
    }
}

/// PID file configuration
#[derive(Debug, Clone, Serialize, Deserialize, Hash)]
pub struct ProcessFileConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_directory: Option<String>,
    #[serde(default = "default_true")]
    pub use_process_id: bool,
}

/// Context-aware restart configuration (matches Go)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextAwareRestartConfig {
    /// Default restart configuration
    pub default: RestartConfig,
    
    /// Health failure-specific restart configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_failures: Option<RestartConfig>,
    
    /// Resource violation-specific restart configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_violations: Option<RestartConfig>,
    
    /// Startup grace period
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "option_duration_serde"
    )]
    pub startup_grace_period: Option<Duration>,
    
    /// Sustained violation time
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "option_duration_serde"
    )]
    pub sustained_violation_time: Option<Duration>,
}

/// Restart configuration (matches Go's RestartConfig)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestartConfig {
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    #[serde(
        default = "default_retry_delay",
        with = "duration_serde"
    )]
    pub retry_delay: Duration,
    #[serde(default = "default_backoff_rate")]
    pub backoff_rate: f32,
}

/// Simple restart policy configuration (for backward compatibility with lifecycle manager)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestartPolicyConfig {
    #[serde(default = "default_restart_strategy")]
    pub strategy: RestartStrategy,
    #[serde(default = "default_max_retries")]
    pub max_attempts: u32,
    #[serde(
        default = "default_retry_delay",
        with = "duration_serde"
    )]
    pub restart_delay: Duration,
    #[serde(default = "default_backoff_rate")]
    pub backoff_multiplier: f32,
}

/// Restart strategy enumeration (for backward compatibility)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum RestartStrategy {
    Never,
    OnFailure,
    Always,
}

fn default_restart_strategy() -> RestartStrategy {
    RestartStrategy::OnFailure
}

/// Resource limits configuration (matches Go's resourcelimits.ResourceLimits)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimitsConfig {
    /// Memory limits with policy
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<MemoryLimits>,
    
    /// CPU limits with policy
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu: Option<CpuLimits>,
    
    /// Process/file descriptor limits with policy
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process: Option<ProcessLimits>,
}

/// Memory limits configuration
/// 
/// Accepts memory limit in MB via `limit_mb` field for convenience,
/// or in bytes via `max_rss`. At least one must be specified.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryLimits {
    /// Maximum RSS in bytes (alternative to limit_mb)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_rss: Option<u64>,
    
    /// Maximum RSS in megabytes (alternative to max_rss, more convenient)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_mb: Option<u64>,
    
    /// Warning threshold percentage (0-100)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning_threshold: Option<u32>,
    
    /// Policy to apply on violation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
}

impl MemoryLimits {
    /// Get the memory limit in bytes, preferring limit_mb if specified
    pub fn max_rss_bytes(&self) -> u64 {
        if let Some(mb) = self.limit_mb {
            mb * 1024 * 1024
        } else if let Some(bytes) = self.max_rss {
            bytes
        } else {
            0  // No limit specified
        }
    }
}

/// CPU limits configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuLimits {
    /// Maximum CPU percent
    pub max_percent: f32,
    /// Warning threshold percentage (0-100)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning_threshold: Option<u32>,
    /// Policy to apply on violation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
}

/// Process limits configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessLimits {
    /// Maximum file descriptors
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_file_descriptors: Option<u32>,
    /// Policy to apply on violation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
}

/// Process-specific log collection configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessLogCollectionConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub capture_stdout: bool,
    #[serde(default = "default_true")]
    pub capture_stderr: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processing: Option<LogProcessingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outputs: Option<LogOutputsConfig>,
}

/// Log processing configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogProcessingConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filters: Option<LogFiltersConfig>,
}

/// Log filters configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogFiltersConfig {
    #[serde(default)]
    pub exclude_patterns: Vec<String>,
}

/// Log outputs configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogOutputsConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub separate: Option<SeparateOutputsConfig>,
}

/// Separate outputs for stdout/stderr
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeparateOutputsConfig {
    #[serde(default)]
    pub stdout: Vec<LogTargetConfig>,
    #[serde(default)]
    pub stderr: Vec<LogTargetConfig>,
}

/// Health check configuration (matches Go's monitoring.HealthCheckConfig)
#[derive(Debug, Clone, Serialize, Deserialize, Hash)]
pub struct HealthCheckConfig {
    #[serde(rename = "type")]
    pub health_check_type: String, // "process", "http", "grpc", "tcp"
    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_endpoint: Option<String>,  // HTTP endpoint for health checks (e.g., "http://localhost:8080/health")
    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_options: Option<HealthCheckRunOptions>,
}

/// Health check run options
#[derive(Debug, Clone, Serialize, Deserialize, Hash)]
pub struct HealthCheckRunOptions {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(
        default = "default_health_check_interval",
        with = "duration_serde"
    )]
    pub interval: Duration,
    #[serde(
        default = "default_health_check_timeout",
        with = "duration_serde"
    )]
    pub timeout: Duration,
    #[serde(
        default = "default_initial_delay",
        with = "duration_serde"
    )]
    pub initial_delay: Duration,
    #[serde(default = "default_retries")]
    pub retries: u32,
}

/// Process discovery configuration (for unmanaged processes)
#[derive(Debug, Clone, Serialize, Deserialize, Hash)]
pub struct ProcessDiscoveryConfig {
    pub method: String, // "pid_file", "process_name", "port"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_name: Option<String>,
    #[serde(
        default = "default_check_interval",
        with = "duration_serde"
    )]
    pub check_interval: Duration,
}

/// Unmanaged process control configuration
#[derive(Debug, Clone, Serialize, Deserialize, Hash)]
pub struct UnmanagedProcessControlConfig {
    #[serde(
        default = "default_graceful_timeout",
        with = "duration_serde"
    )]
    pub graceful_timeout: Duration,
}

/// Log collection configuration (optional top-level)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogCollectionConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global_aggregation: Option<GlobalAggregationConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enhancement: Option<LogEnhancementConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<DefaultLogConfig>,
}

/// Default log configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefaultLogConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub capture_stdout: bool,
    #[serde(default = "default_true")]
    pub capture_stderr: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outputs: Option<LogOutputsConfig>,
}

/// Global log aggregation configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalAggregationConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub targets: Vec<LogTargetConfig>,
}

/// Log target configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogTargetConfig {
    #[serde(rename = "type")]
    pub target_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default = "default_log_format")]
    pub format: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
}

/// Log enhancement configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEnhancementConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<LogMetadataConfig>,
}

/// Log metadata configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogMetadataConfig {
    #[serde(default)]
    pub add_process_manager_id: bool,
    #[serde(default)]
    pub add_hostname: bool,
    #[serde(default)]
    pub add_timestamp: bool,
    #[serde(default)]
    pub add_sequence: bool,
    #[serde(default)]
    pub add_line_number: bool,
}

impl ProcessManagerConfig {
    /// Load configuration from a YAML file
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config file: {}", path.as_ref().display()))?;
        
        Self::load_from_string(&content)
    }

    /// Load configuration from a YAML string
    pub fn load_from_string(content: &str) -> Result<Self> {
        let config: ProcessManagerConfig = serde_yaml::from_str(content)
            .context("Failed to parse YAML configuration")?;
        
        config.validate()?;
        Ok(config)
    }

    /// Validate the configuration
    pub fn validate(&self) -> Result<()> {
        validation::validate_config(self)
    }

    /// Get enabled processes only
    pub fn enabled_processes(&self) -> Vec<&ProcessConfig> {
        self.managed_processes
            .iter()
            .filter(|p| p.enabled)
            .collect()
    }
}

// Default value functions
fn default_log_level() -> String {
    "info".to_string()
}

fn default_force_shutdown_timeout() -> Duration {
    Duration::from_secs(30)
}

fn default_profile_type() -> String {
    "default".to_string()
}

fn default_enabled() -> bool {
    true
}

fn default_true() -> bool {
    true
}

fn default_wait_delay() -> Duration {
    Duration::from_secs(10)
}

fn default_max_retries() -> u32 {
    3
}

fn default_retry_delay() -> Duration {
    Duration::from_secs(5)
}

fn default_backoff_rate() -> f32 {
    1.5
}

fn default_health_check_interval() -> Duration {
    Duration::from_secs(30)
}

fn default_health_check_timeout() -> Duration {
    Duration::from_secs(5)
}

fn default_initial_delay() -> Duration {
    Duration::from_secs(2)
}

fn default_retries() -> u32 {
    2
}

fn default_check_interval() -> Duration {
    Duration::from_secs(30)
}

fn default_graceful_timeout() -> Duration {
    Duration::from_secs(10)
}

fn default_log_format() -> String {
    "plain".to_string()
}

// Custom serialization for Duration
mod duration_serde {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let secs = duration.as_secs();
        serializer.serialize_str(&format!("{}s", secs))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        parse_duration(&s).map_err(serde::de::Error::custom)
    }

    fn parse_duration(s: &str) -> Result<Duration, String> {
        // Check for "ms" BEFORE "s" since "ms" ends with 's'
        if s.ends_with("ms") {
            let num_str = &s[..s.len() - 2];
            let millis: u64 = num_str.parse().map_err(|_| format!("Invalid duration: {}", s))?;
            Ok(Duration::from_millis(millis))
        } else if s.ends_with('s') {
            let num_str = &s[..s.len() - 1];
            let secs: u64 = num_str.parse().map_err(|_| format!("Invalid duration: {}", s))?;
            Ok(Duration::from_secs(secs))
        } else if s.ends_with('m') {
            let num_str = &s[..s.len() - 1];
            let mins: u64 = num_str.parse().map_err(|_| format!("Invalid duration: {}", s))?;
            Ok(Duration::from_secs(mins * 60))
        } else {
            Err(format!("Duration must end with 's', 'ms', or 'm': {}", s))
        }
    }
}

// Custom serialization for Option<Duration>
mod option_duration_serde {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Option<Duration>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match duration {
            Some(d) => {
                let secs = d.as_secs();
                serializer.serialize_str(&format!("{}s", secs))
            }
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let opt: Option<String> = Option::deserialize(deserializer)?;
        match opt {
            Some(s) => super::duration_serde::deserialize(serde::de::value::StringDeserializer::new(s)).map(Some),
            None => Ok(None),
        }
    }
}
