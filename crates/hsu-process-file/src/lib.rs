//! # HSU Process File
//!
//! Process state persistence for the HSU framework.
//!
//! This crate provides functionality for:
//! - Saving process state to disk (PID files)
//! - Loading process state from disk
//! - Process attachment after manager restart
//! - Platform-specific directory resolution
//! - Config hash validation for process identity
//!
//! This corresponds to the Go package `pkg/processfile`.

use hsu_common::ProcessResult;
use hsu_process_state::ProcessState;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Default application name for HSU Process Manager
pub const DEFAULT_APP_NAME: &str = "hsu-process-manager";

/// Service context defines where the service runs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceContext {
    /// System service (daemon) - uses system directories
    System,
    /// User service - uses user directories
    User,
    /// Session service - cleaned up on logout
    Session,
}

/// Configuration for process file management
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessFileConfig {
    /// Base directory for PID files (if empty, uses OS default)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_directory: Option<String>,
    
    /// Service context - affects directory selection
    #[serde(default = "default_service_context")]
    pub service_context: ServiceContext,
    
    /// Application name for subdirectory creation
    #[serde(default = "default_app_name")]
    pub app_name: String,
    
    /// Create subdirectory for the app (recommended for system services)
    #[serde(default = "default_use_subdirectory")]
    pub use_subdirectory: bool,
}

fn default_service_context() -> ServiceContext {
    ServiceContext::User
}

fn default_app_name() -> String {
    DEFAULT_APP_NAME.to_string()
}

fn default_use_subdirectory() -> bool {
    true
}

impl Default for ProcessFileConfig {
    fn default() -> Self {
        Self {
            base_directory: None,
            service_context: ServiceContext::User,
            app_name: DEFAULT_APP_NAME.to_string(),
            use_subdirectory: true,
        }
    }
}

/// Process file data structure (persisted as JSON)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessFile {
    pub process_id: String,
    pub pid: u32,
    pub state: ProcessState,
    pub start_time: chrono::DateTime<chrono::Utc>,
    pub config_hash: Option<String>,
}

impl ProcessFile {
    /// Create a new process file.
    pub fn new(
        process_id: String,
        pid: u32,
        state: ProcessState,
        start_time: chrono::DateTime<chrono::Utc>,
    ) -> Self {
        Self {
            process_id,
            pid,
            state,
            start_time,
            config_hash: None,
        }
    }
    
    /// Create a new process file with config hash.
    pub fn with_config_hash(
        process_id: String,
        pid: u32,
        state: ProcessState,
        start_time: chrono::DateTime<chrono::Utc>,
        config_hash: String,
    ) -> Self {
        Self {
            process_id,
            pid,
            state,
            start_time,
            config_hash: Some(config_hash),
        }
    }

    /// Save process file to disk (atomic write).
    pub async fn save<P: AsRef<Path>>(&self, path: P) -> ProcessResult<()> {
        let path = path.as_ref();
        
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| hsu_common::ProcessError::Configuration {
                    id: self.process_id.clone(),
                    reason: format!("Failed to create directory {}: {}", parent.display(), e),
                })?;
        }
        
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| hsu_common::ProcessError::Configuration {
                id: self.process_id.clone(),
                reason: format!("Failed to serialize process file: {}", e),
            })?;

        // Atomic write: write to temp file, then rename
        let temp_path = path.with_extension("tmp");
        tokio::fs::write(&temp_path, json)
            .await
            .map_err(|e| hsu_common::ProcessError::Configuration {
                id: self.process_id.clone(),
                reason: format!("Failed to write process file: {}", e),
            })?;
            
        tokio::fs::rename(&temp_path, path)
            .await
            .map_err(|e| hsu_common::ProcessError::Configuration {
                id: self.process_id.clone(),
                reason: format!("Failed to rename process file: {}", e),
            })?;

        Ok(())
    }

    /// Load process file from disk.
    pub async fn load<P: AsRef<Path>>(path: P) -> ProcessResult<Self> {
        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| hsu_common::ProcessError::Configuration {
                id: "unknown".to_string(),
                reason: format!("Failed to read process file: {}", e),
            })?;

        let process_file: ProcessFile = serde_json::from_str(&content)
            .map_err(|e| hsu_common::ProcessError::Configuration {
                id: "unknown".to_string(),
                reason: format!("Failed to parse process file: {}", e),
            })?;

        Ok(process_file)
    }

    /// Delete process file from disk.
    pub async fn delete<P: AsRef<Path>>(path: P) -> ProcessResult<()> {
        if path.as_ref().exists() {
            tokio::fs::remove_file(&path)
                .await
                .map_err(|e| hsu_common::ProcessError::Configuration {
                    id: "unknown".to_string(),
                    reason: format!("Failed to delete process file: {}", e),
                })?;
        }
        Ok(())
    }
}

/// Process file manager for path generation and management
pub struct ProcessFileManager {
    config: ProcessFileConfig,
}

impl ProcessFileManager {
    /// Create a new process file manager
    pub fn new(config: ProcessFileConfig) -> Self {
        Self { config }
    }
    
    /// Create a new process file manager with default configuration
    pub fn with_defaults() -> Self {
        Self::new(ProcessFileConfig::default())
    }
    
    /// Get the base directory for process files (platform-specific)
    pub fn get_base_directory(&self) -> PathBuf {
        if let Some(ref base_dir) = self.config.base_directory {
            return PathBuf::from(base_dir);
        }
        
        // Platform-specific defaults
        match self.config.service_context {
            ServiceContext::System => self.get_system_directory(),
            ServiceContext::User => self.get_user_directory(),
            ServiceContext::Session => self.get_session_directory(),
        }
    }
    
    /// Get system service directory (platform-specific)
    fn get_system_directory(&self) -> PathBuf {
        #[cfg(target_os = "windows")]
        {
            // Windows: C:\ProgramData\HSU\
            PathBuf::from(std::env::var("ProgramData")
                .unwrap_or_else(|_| "C:\\ProgramData".to_string()))
                .join("HSU")
        }
        
        #[cfg(not(target_os = "windows"))]
        {
            // Unix: /var/run/ or /run/
            if Path::new("/run").exists() {
                PathBuf::from("/run")
            } else {
                PathBuf::from("/var/run")
            }
        }
    }
    
    /// Get user service directory (platform-specific)
    fn get_user_directory(&self) -> PathBuf {
        #[cfg(target_os = "windows")]
        {
            // Windows: %LOCALAPPDATA%\HSU\
            PathBuf::from(std::env::var("LOCALAPPDATA")
                .unwrap_or_else(|_| {
                    let user_profile = std::env::var("USERPROFILE")
                        .unwrap_or_else(|_| "C:\\Users\\Default".to_string());
                    format!("{}\\AppData\\Local", user_profile)
                }))
                .join("HSU")
        }
        
        #[cfg(not(target_os = "windows"))]
        {
            // Unix: $XDG_RUNTIME_DIR or ~/.local/share/hsu/
            if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
                PathBuf::from(runtime_dir)
            } else {
                dirs::data_local_dir()
                    .unwrap_or_else(|| {
                        dirs::home_dir()
                            .unwrap_or_else(|| PathBuf::from("/tmp"))
                            .join(".local/share")
                    })
                    .join("hsu")
            }
        }
    }
    
    /// Get session service directory (same as user for now)
    fn get_session_directory(&self) -> PathBuf {
        self.get_user_directory()
    }
    
    /// Generate a process file path for the given process ID
    fn generate_file_path(&self, process_id: &str, extension: &str) -> PathBuf {
        let mut base_dir = self.get_base_directory();
        
        // Add app subdirectory if configured
        if self.config.use_subdirectory {
            base_dir = base_dir.join(&self.config.app_name);
        }
        
        base_dir.join(format!("{}{}", process_id, extension))
    }
    
    /// Generate a PID file path for the given process ID
    pub fn generate_pid_file_path(&self, process_id: &str) -> PathBuf {
        self.generate_file_path(process_id, ".pid")
    }
    
    /// Generate a port file path for the given process ID
    pub fn generate_port_file_path(&self, process_id: &str) -> PathBuf {
        self.generate_file_path(process_id, ".port")
    }
    
    /// Write a PID file (simple text format: just the PID number)
    pub async fn write_pid_file(&self, process_id: &str, pid: u32) -> ProcessResult<()> {
        let path = self.generate_pid_file_path(process_id);
        
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| hsu_common::ProcessError::Configuration {
                    id: process_id.to_string(),
                    reason: format!("Failed to create directory {}: {}", parent.display(), e),
                })?;
        }
        
        let content = format!("{}\n", pid);
        tokio::fs::write(&path, content)
            .await
            .map_err(|e| hsu_common::ProcessError::Configuration {
                id: process_id.to_string(),
                reason: format!("Failed to write PID file {}: {}", path.display(), e),
            })?;
            
        Ok(())
    }
    
    /// Read a PID file
    pub async fn read_pid_file(&self, process_id: &str) -> ProcessResult<u32> {
        let path = self.generate_pid_file_path(process_id);
        
        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|_e| hsu_common::ProcessError::NotFound {
                id: process_id.to_string(),
            })?;
            
        let pid = content.trim().parse::<u32>()
            .map_err(|e| hsu_common::ProcessError::Configuration {
                id: process_id.to_string(),
                reason: format!("Invalid PID in file {}: {}", path.display(), e),
            })?;
            
        Ok(pid)
    }
    
    /// Delete a PID file
    pub async fn delete_pid_file(&self, process_id: &str) -> ProcessResult<()> {
        let path = self.generate_pid_file_path(process_id);
        ProcessFile::delete(&path).await
    }
    
    /// Write a port file
    pub async fn write_port_file(&self, process_id: &str, port: u16) -> ProcessResult<()> {
        let path = self.generate_port_file_path(process_id);
        
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| hsu_common::ProcessError::Configuration {
                    id: process_id.to_string(),
                    reason: format!("Failed to create directory {}: {}", parent.display(), e),
                })?;
        }
        
        let content = format!("{}\n", port);
        tokio::fs::write(&path, content)
            .await
            .map_err(|e| hsu_common::ProcessError::Configuration {
                id: process_id.to_string(),
                reason: format!("Failed to write port file {}: {}", path.display(), e),
            })?;
            
        Ok(())
    }
    
    /// Read a port file
    pub async fn read_port_file(&self, process_id: &str) -> ProcessResult<u16> {
        let path = self.generate_port_file_path(process_id);
        
        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|_e| hsu_common::ProcessError::NotFound {
                id: process_id.to_string(),
            })?;
            
        let port = content.trim().parse::<u16>()
            .map_err(|e| hsu_common::ProcessError::Configuration {
                id: process_id.to_string(),
                reason: format!("Invalid port in file {}: {}", path.display(), e),
            })?;
            
        Ok(port)
    }
}

/// Compute a configuration hash for process identity validation
pub fn compute_config_hash<T: Hash>(config: &T) -> String {
    let mut hasher = DefaultHasher::new();
    config.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

