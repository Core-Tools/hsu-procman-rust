/// Utilities for parsing and analyzing log output
pub struct LogParser {
    logs: Vec<String>,
}

impl LogParser {
    pub fn new(logs: Vec<String>) -> Self {
        Self { logs }
    }

    /// Check if logs contain a specific pattern
    pub fn contains(&self, pattern: &str) -> bool {
        self.logs.iter().any(|line| line.contains(pattern))
    }

    /// Check if logs contain all patterns in order
    pub fn contains_sequence(&self, patterns: &[&str]) -> bool {
        let mut pattern_idx = 0;
        
        for line in &self.logs {
            if line.contains(patterns[pattern_idx]) {
                pattern_idx += 1;
                if pattern_idx >= patterns.len() {
                    return true;
                }
            }
        }
        
        false
    }

    /// Count occurrences of a pattern
    pub fn count_occurrences(&self, pattern: &str) -> usize {
        self.logs.iter().filter(|line| line.contains(pattern)).count()
    }

    /// Get all lines matching a pattern
    pub fn find_matching(&self, pattern: &str) -> Vec<&String> {
        self.logs.iter().filter(|line| line.contains(pattern)).collect()
    }

    /// Get all logs
    pub fn all_logs(&self) -> &[String] {
        &self.logs
    }

    /// Print all logs
    pub fn print_all(&self) {
        println!("\n=== All Logs ===");
        for (i, line) in self.logs.iter().enumerate() {
            println!("{:4}: {}", i + 1, line);
        }
        println!("================\n");
    }

    /// Find lines between two patterns
    pub fn find_between(&self, start_pattern: &str, end_pattern: &str) -> Vec<&String> {
        let mut result = Vec::new();
        let mut in_range = false;
        
        for line in &self.logs {
            if line.contains(start_pattern) {
                in_range = true;
                result.push(line);
                continue;
            }
            
            if line.contains(end_pattern) {
                if in_range {
                    result.push(line);
                }
                break;
            }
            
            if in_range {
                result.push(line);
            }
        }
        
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_parser_contains() {
        let logs = vec![
            "Starting process".to_string(),
            "Process running".to_string(),
            "Process stopped".to_string(),
        ];
        let parser = LogParser::new(logs);
        
        assert!(parser.contains("running"));
        assert!(!parser.contains("crashed"));
    }

    #[test]
    fn test_log_parser_sequence() {
        let logs = vec![
            "Starting process".to_string(),
            "Some other log".to_string(),
            "Process running".to_string(),
            "Another log".to_string(),
            "Process stopped".to_string(),
        ];
        let parser = LogParser::new(logs);
        
        assert!(parser.contains_sequence(&["Starting", "running", "stopped"]));
        assert!(!parser.contains_sequence(&["stopped", "running"]));
    }

    #[test]
    fn test_count_occurrences() {
        let logs = vec![
            "Error: something".to_string(),
            "Info: ok".to_string(),
            "Error: another".to_string(),
        ];
        let parser = LogParser::new(logs);
        
        assert_eq!(parser.count_occurrences("Error"), 2);
        assert_eq!(parser.count_occurrences("Info"), 1);
    }
}

