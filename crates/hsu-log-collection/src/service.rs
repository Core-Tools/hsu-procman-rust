//! Log collection service implementation

use crate::output::OutputWriter;
use crate::types::{
    LogEntry, LogMetadata, ProcessLogStatus, StreamType, SystemLogStatus,
};
use chrono::Utc;
use hsu_common::{ProcessError, ProcessResult};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

/// Configuration for process-specific log collection
#[derive(Debug, Clone)]
pub struct ProcessLogConfig {
    pub enabled: bool,
    pub capture_stdout: bool,
    pub capture_stderr: bool,
}

impl Default for ProcessLogConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            capture_stdout: true,
            capture_stderr: true,
        }
    }
}

/// System-wide log collection configuration
#[derive(Debug, Clone)]
pub struct SystemLogConfig {
    pub max_managed_processes: usize,
    pub output_file: Option<PathBuf>,
    pub buffer_size: usize,
}

impl Default for SystemLogConfig {
    fn default() -> Self {
        Self {
            max_managed_processes: 100,
            output_file: None,
            buffer_size: 8192,
        }
    }
}

/// Main log collection service
pub struct LogCollectionService {
    config: SystemLogConfig,
    processes: Arc<RwLock<HashMap<String, Arc<ProcessLogCollector>>>>,
    outputs: Arc<RwLock<Vec<Box<dyn OutputWriter>>>>,
    running: Arc<AtomicBool>,
    total_lines: Arc<AtomicI64>,
    total_bytes: Arc<AtomicI64>,
    start_time: Arc<RwLock<Option<chrono::DateTime<Utc>>>>,
    cancel_token: CancellationToken,
}

impl std::fmt::Debug for LogCollectionService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogCollectionService")
            .field("config", &self.config)
            .field("processes_count", &self.processes.read().len())
            .field("outputs_count", &self.outputs.read().len())
            .field("running", &self.running.load(Ordering::SeqCst))
            .field("total_lines", &self.total_lines.load(Ordering::SeqCst))
            .field("total_bytes", &self.total_bytes.load(Ordering::SeqCst))
            .finish()
    }
}

impl LogCollectionService {
    /// Create a new log collection service
    pub fn new(config: SystemLogConfig) -> Self {
        Self {
            config,
            processes: Arc::new(RwLock::new(HashMap::new())),
            outputs: Arc::new(RwLock::new(Vec::new())),
            running: Arc::new(AtomicBool::new(false)),
            total_lines: Arc::new(AtomicI64::new(0)),
            total_bytes: Arc::new(AtomicI64::new(0)),
            start_time: Arc::new(RwLock::new(None)),
            cancel_token: CancellationToken::new(),
        }
    }

    /// Start the log collection service
    pub async fn start(&self) -> ProcessResult<()> {
        if self
            .running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(ProcessError::LoggingError {
                id: "log-service".to_string(),
                reason: "Log collection service already running".to_string(),
            });
        }

        *self.start_time.write() = Some(Utc::now());

        // Initialize outputs
        self.initialize_outputs()?;

        info!(
            max_processes = self.config.max_managed_processes,
            "Log collection service started"
        );

        Ok(())
    }

    /// Stop the log collection service
    pub async fn stop(&self) -> ProcessResult<()> {
        if !self.running.swap(false, Ordering::SeqCst) {
            return Err(ProcessError::LoggingError {
                id: "log-service".to_string(),
                reason: "Log collection service not running".to_string(),
            });
        }

        // Cancel all collection tasks
        self.cancel_token.cancel();

        // Stop all process collectors
        let processes = self.processes.write();
        for (process_id, collector) in processes.iter() {
            if let Err(e) = collector.stop().await {
                warn!(
                    process_id = %process_id,
                    error = %e,
                    "Error stopping process collector"
                );
            }
        }
        drop(processes);

        // Close all outputs
        let mut outputs = self.outputs.write();
        for output in outputs.iter_mut() {
            if let Err(e) = output.close() {
                warn!(error = %e, "Error closing output writer");
            }
        }
        drop(outputs);

        info!("Log collection service stopped");

        Ok(())
    }

    /// Register a process for log collection
    pub fn register_process(
        &self,
        process_id: String,
        config: ProcessLogConfig,
    ) -> ProcessResult<()> {
        if !self.running.load(Ordering::SeqCst) {
            return Err(ProcessError::LoggingError {
                id: "log-service".to_string(),
                reason: "Log collection service not running".to_string(),
            });
        }

        let collector = Arc::new(ProcessLogCollector::new(
            process_id.clone(),
            config,
            self.clone_service_refs(),
        ));

        self.processes.write().insert(process_id.clone(), collector);

        debug!(process_id = %process_id, "Process registered for log collection");

        Ok(())
    }

    /// Unregister a process from log collection
    pub async fn unregister_process(&self, process_id: &str) -> ProcessResult<()> {
        let collector = {
            let mut processes = self.processes.write();
            processes.remove(process_id)
        };

        if let Some(collector) = collector {
            collector.stop().await?;
            debug!(process_id = %process_id, "Process unregistered from log collection");
        }

        Ok(())
    }

    /// Collect logs from a stream
    pub async fn collect_from_stream(
        &self,
        process_id: &str,
        stream: impl tokio::io::AsyncRead + Unpin + Send + 'static,
        stream_type: StreamType,
    ) -> ProcessResult<()> {
        let collector = self.get_process(process_id)?;
        collector
            .collect_from_stream(stream, stream_type, self.cancel_token.child_token())
            .await
    }

    /// Get process status
    pub fn get_process_status(&self, process_id: &str) -> ProcessResult<ProcessLogStatus> {
        let collector = self.get_process(process_id)?;
        Ok(collector.get_status())
    }

    /// Get system status
    pub fn get_system_status(&self) -> SystemLogStatus {
        let processes = self.processes.read();
        let start_time = self.start_time.read();

        SystemLogStatus {
            active: self.running.load(Ordering::SeqCst),
            processes_active: processes.values().filter(|p| p.is_active()).count(),
            total_processes: processes.len(),
            total_lines: self.total_lines.load(Ordering::SeqCst),
            total_bytes: self.total_bytes.load(Ordering::SeqCst),
            start_time: start_time.unwrap_or_else(Utc::now),
            last_activity: None, // TODO: Track last activity
            output_targets: self
                .outputs
                .read()
                .iter()
                .map(|_| "output".to_string())
                .collect(),
        }
    }

    // ===== INTERNAL METHODS =====

    fn initialize_outputs(&self) -> ProcessResult<()> {
        let mut outputs = self.outputs.write();

        // Add file output if configured
        if let Some(output_file) = &self.config.output_file {
            let writer = crate::output::FileOutputWriter::new(output_file.clone())?;
            outputs.push(Box::new(writer));
        }

        Ok(())
    }

    fn get_process(&self, process_id: &str) -> ProcessResult<Arc<ProcessLogCollector>> {
        self.processes
            .read()
            .get(process_id)
            .cloned()
            .ok_or_else(|| ProcessError::NotFound {
                id: format!("Process {} not registered", process_id),
            })
    }

    fn clone_service_refs(&self) -> ServiceRefs {
        ServiceRefs {
            outputs: Arc::clone(&self.outputs),
            total_lines: Arc::clone(&self.total_lines),
            total_bytes: Arc::clone(&self.total_bytes),
        }
    }
}

/// References to service components (for sharing with collectors)
#[derive(Clone)]
struct ServiceRefs {
    outputs: Arc<RwLock<Vec<Box<dyn OutputWriter>>>>,
    total_lines: Arc<AtomicI64>,
    total_bytes: Arc<AtomicI64>,
}

/// Per-process log collector
struct ProcessLogCollector {
    process_id: String,
    config: ProcessLogConfig,
    service_refs: ServiceRefs,
    active: Arc<AtomicBool>,
    lines_processed: Arc<AtomicI64>,
    bytes_processed: Arc<AtomicI64>,
    last_activity: Arc<RwLock<Option<chrono::DateTime<Utc>>>>,
    errors: Arc<RwLock<Vec<String>>>,
    tasks: Arc<RwLock<Vec<JoinHandle<()>>>>,
}

impl ProcessLogCollector {
    fn new(process_id: String, config: ProcessLogConfig, service_refs: ServiceRefs) -> Self {
        Self {
            process_id,
            config,
            service_refs,
            active: Arc::new(AtomicBool::new(false)),
            lines_processed: Arc::new(AtomicI64::new(0)),
            bytes_processed: Arc::new(AtomicI64::new(0)),
            last_activity: Arc::new(RwLock::new(None)),
            errors: Arc::new(RwLock::new(Vec::new())),
            tasks: Arc::new(RwLock::new(Vec::new())),
        }
    }

    async fn collect_from_stream(
        &self,
        stream: impl tokio::io::AsyncRead + Unpin + Send + 'static,
        stream_type: StreamType,
        cancel_token: CancellationToken,
    ) -> ProcessResult<()> {
        if !self.config.enabled {
            return Ok(());
        }

        // Check if we should capture this stream type
        match stream_type {
            StreamType::Stdout if !self.config.capture_stdout => return Ok(()),
            StreamType::Stderr if !self.config.capture_stderr => return Ok(()),
            _ => {}
        }

        self.active.store(true, Ordering::SeqCst);

        // Spawn stream reader task
        let process_id = self.process_id.clone();
        let lines_processed = Arc::clone(&self.lines_processed);
        let bytes_processed = Arc::clone(&self.bytes_processed);
        let last_activity = Arc::clone(&self.last_activity);
        let errors = Arc::clone(&self.errors);
        let service_refs = self.service_refs.clone();

        let task = tokio::spawn(async move {
            Self::stream_reader(
                stream,
                stream_type,
                process_id,
                lines_processed,
                bytes_processed,
                last_activity,
                errors,
                service_refs,
                cancel_token,
            )
            .await;
        });

        self.tasks.write().push(task);

        Ok(())
    }

    async fn stream_reader(
        stream: impl tokio::io::AsyncRead + Unpin,
        stream_type: StreamType,
        process_id: String,
        lines_processed: Arc<AtomicI64>,
        bytes_processed: Arc<AtomicI64>,
        last_activity: Arc<RwLock<Option<chrono::DateTime<Utc>>>>,
        errors: Arc<RwLock<Vec<String>>>,
        service_refs: ServiceRefs,
        cancel_token: CancellationToken,
    ) {
        debug!("stream_reader started for {} ({:?})", process_id, stream_type);
        let reader = BufReader::new(stream);
        let mut lines = reader.lines();
        let mut line_num = 0i64;

        loop {
            tokio::select! {
                _ = cancel_token.cancelled() => {
                    debug!(process_id = %process_id, "Stream reader cancelled");
                    break;
                }
                result = lines.next_line() => {
                    match result {
                        Ok(Some(line)) => {
                            line_num += 1;

                            let metadata = LogMetadata {
                                timestamp: Utc::now(),
                                process_id: process_id.clone(),
                                stream: stream_type,
                                line_num,
                            };

                            if let Err(e) = Self::process_log_line(
                                &line,
                                &metadata,
                                &lines_processed,
                                &bytes_processed,
                                &last_activity,
                                &service_refs,
                            ) {
                                warn!(
                                    process_id = %process_id,
                                    error = %e,
                                    "Failed to process log line"
                                );
                                errors.write().push(format!("Failed to process line {}: {}", line_num, e));
                            }
                        }
                        Ok(None) => {
                            debug!(process_id = %process_id, "Stream ended");
                            break;
                        }
                        Err(e) => {
                            error!(
                                process_id = %process_id,
                                error = %e,
                                "Error reading from stream"
                            );
                            errors.write().push(format!("Stream reading error: {}", e));
                            break;
                        }
                    }
                }
            }
        }

        debug!(
            process_id = %process_id,
            lines = line_num,
            "Stream reader finished"
        );
    }

    fn process_log_line(
        line: &str,
        metadata: &LogMetadata,
        lines_processed: &Arc<AtomicI64>,
        bytes_processed: &Arc<AtomicI64>,
        last_activity: &Arc<RwLock<Option<chrono::DateTime<Utc>>>>,
        service_refs: &ServiceRefs,
    ) -> ProcessResult<()> {
        // Update metrics
        lines_processed.fetch_add(1, Ordering::SeqCst);
        bytes_processed.fetch_add(line.len() as i64, Ordering::SeqCst);
        *last_activity.write() = Some(Utc::now());

        // Update service-level metrics
        service_refs.total_lines.fetch_add(1, Ordering::SeqCst);
        service_refs
            .total_bytes
            .fetch_add(line.len() as i64, Ordering::SeqCst);

        // Create log entry
        let entry = LogEntry {
            timestamp: metadata.timestamp,
            level: None, // TODO: Parse level from line
            message: line.to_string(),
            process_id: metadata.process_id.clone(),
            stream: metadata.stream,
            fields: None,
            raw_line: line.to_string(),
            enhanced: None,
        };

        // Write to all outputs
        let mut outputs = service_refs.outputs.write();
        for output in outputs.iter_mut() {
            output.write(&entry)?;
        }

        Ok(())
    }

    async fn stop(&self) -> ProcessResult<()> {
        self.active.store(false, Ordering::SeqCst);

        // Wait for all tasks to complete (with timeout)
        let tasks: Vec<_> = self.tasks.write().drain(..).collect();
        for task in tasks {
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), task).await;
        }

        Ok(())
    }

    fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }

    fn get_status(&self) -> ProcessLogStatus {
        ProcessLogStatus {
            process_id: self.process_id.clone(),
            active: self.active.load(Ordering::SeqCst),
            lines_processed: self.lines_processed.load(Ordering::SeqCst),
            bytes_processed: self.bytes_processed.load(Ordering::SeqCst),
            last_activity: *self.last_activity.read(),
            errors: self.errors.read().clone(),
        }
    }
}


