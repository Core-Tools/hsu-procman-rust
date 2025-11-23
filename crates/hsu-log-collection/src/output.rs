//! Output writers for log targets

use crate::types::LogEntry;
use hsu_common::ProcessResult;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Trait for writing log entries to various outputs
pub trait OutputWriter: Send + Sync {
    /// Write a log entry
    fn write(&mut self, entry: &LogEntry) -> ProcessResult<()>;

    /// Flush any buffered output
    fn flush(&mut self) -> ProcessResult<()>;

    /// Close the output writer
    fn close(&mut self) -> ProcessResult<()>;
}

/// File output writer
pub struct FileOutputWriter {
    writer: Arc<Mutex<BufWriter<File>>>,
    #[allow(dead_code)]
    path: PathBuf, // Kept for debugging/inspection
}

impl FileOutputWriter {
    /// Create a new file output writer
    pub fn new(path: PathBuf) -> ProcessResult<Self> {
        // Create parent directories if needed
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                hsu_common::ProcessError::LoggingError {
                    id: "log-service".to_string(),
                    reason: format!("Failed to create log directory: {}", e),
                }
            })?;
        }

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| hsu_common::ProcessError::LoggingError {
                id: "log-service".to_string(),
                reason: format!("Failed to open log file: {}", e),
            })?;

        let writer = BufWriter::new(file);

        Ok(Self {
            writer: Arc::new(Mutex::new(writer)),
            path,
        })
    }
}

impl OutputWriter for FileOutputWriter {
    fn write(&mut self, entry: &LogEntry) -> ProcessResult<()> {
        let mut writer = self.writer.lock().unwrap();

        // Format: [timestamp] [level] [process_id/stream] message
        let level_str = entry.level.as_deref().unwrap_or("INFO");
        let line = format!(
            "[{}] [{}] [{}/{}] {}\n",
            entry.timestamp.format("%Y-%m-%d %H:%M:%S%.3f"),
            level_str,
            entry.process_id,
            entry.stream,
            entry.message
        );

        writer
            .write_all(line.as_bytes())
            .map_err(|e| hsu_common::ProcessError::LoggingError {
                id: "log-service".to_string(),
                reason: format!("Failed to write to log file: {}", e),
            })?;

        Ok(())
    }

    fn flush(&mut self) -> ProcessResult<()> {
        let mut writer = self.writer.lock().unwrap();
        writer.flush().map_err(|e| hsu_common::ProcessError::LoggingError {
            id: "log-service".to_string(),
            reason: format!("Failed to flush log file: {}", e),
        })?;
        Ok(())
    }

    fn close(&mut self) -> ProcessResult<()> {
        self.flush()
    }
}

/// Stdout output writer
pub struct StdoutOutputWriter;

impl OutputWriter for StdoutOutputWriter {
    fn write(&mut self, entry: &LogEntry) -> ProcessResult<()> {
        let level_str = entry.level.as_deref().unwrap_or("INFO");
        println!(
            "[{}] [{}] [{}/{}] {}",
            entry.timestamp.format("%Y-%m-%d %H:%M:%S%.3f"),
            level_str,
            entry.process_id,
            entry.stream,
            entry.message
        );
        Ok(())
    }

    fn flush(&mut self) -> ProcessResult<()> {
        use std::io::stdout;
        stdout().flush().map_err(|e| hsu_common::ProcessError::LoggingError {
            id: "log-service".to_string(),
            reason: format!("Failed to flush stdout: {}", e),
        })?;
        Ok(())
    }

    fn close(&mut self) -> ProcessResult<()> {
        self.flush()
    }
}

/// Circular buffer output writer (keeps last N log entries)
pub struct CircularBufferOutputWriter {
    buffer: Arc<Mutex<Vec<LogEntry>>>,
    max_size: usize,
}

impl CircularBufferOutputWriter {
    pub fn new(max_size: usize) -> Self {
        Self {
            buffer: Arc::new(Mutex::new(Vec::with_capacity(max_size))),
            max_size,
        }
    }

    pub fn get_logs(&self) -> Vec<LogEntry> {
        self.buffer.lock().unwrap().clone()
    }
}

impl OutputWriter for CircularBufferOutputWriter {
    fn write(&mut self, entry: &LogEntry) -> ProcessResult<()> {
        let mut buffer = self.buffer.lock().unwrap();
        buffer.push(entry.clone());
        
        // Keep only last max_size entries
        if buffer.len() > self.max_size {
            buffer.remove(0);
        }
        
        Ok(())
    }

    fn flush(&mut self) -> ProcessResult<()> {
        Ok(()) // No-op for in-memory buffer
    }

    fn close(&mut self) -> ProcessResult<()> {
        Ok(()) // No-op for in-memory buffer
    }
}


