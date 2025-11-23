//! Lifecycle management traits and utilities.
//! 
//! # Rust Learning Note
//! 
//! The Lifecycle trait is equivalent to Go's Lifecycle interface.
//! The key difference is the use of `async fn` for asynchronous operations.

use async_trait::async_trait;
use hsu_common::Result;

/// Lifecycle management trait.
/// 
/// Components that implement this trait can be started and stopped
/// in a controlled manner.
/// 
/// # Rust Learning Note
/// 
/// ## async_trait Macro
/// 
/// ```rust,ignore
/// #[async_trait]
/// pub trait Lifecycle {
///     async fn start(&mut self) -> Result<()>;
///     async fn stop(&mut self) -> Result<()>;
/// }
/// ```
/// 
/// This macro transforms the trait into one that returns `Pin<Box<dyn Future>>`
/// internally, allowing async methods in traits.
/// 
/// ## Go Comparison
/// 
/// ```go
/// type Lifecycle interface {
///     Start(ctx context.Context) error
///     Stop(ctx context.Context) error
/// }
/// ```
/// 
/// Rust's async/await is similar to Go's goroutines, but:
/// - **Explicit**: You must `.await` async functions
/// - **Zero-cost**: No runtime overhead when not used
/// - **Type-safe**: Compiler checks Send + Sync requirements
#[async_trait]
pub trait Lifecycle: Send + Sync {
    /// Starts the component.
    /// 
    /// # Errors
    /// Returns an error if the component fails to start.
    async fn start(&mut self) -> Result<()>;

    /// Stops the component gracefully.
    /// 
    /// # Errors
    /// Returns an error if the component fails to stop cleanly.
    async fn stop(&mut self) -> Result<()>;
}

/// Lifecycle manager - manages multiple lifecycle components.
/// 
/// # Rust Learning Note
/// 
/// This struct demonstrates Rust's approach to managing a collection
/// of trait objects. Key points:
/// 
/// - `Box<dyn Lifecycle>`: Heap-allocated trait object (like Go interface)
/// - `Vec`: Growable array (like Go slice)
/// - No garbage collector: Cleanup happens when LifecycleManager is dropped
pub struct LifecycleManager {
    components: Vec<Box<dyn Lifecycle>>,
}

impl LifecycleManager {
    /// Creates a new lifecycle manager.
    pub fn new() -> Self {
        Self {
            components: Vec::new(),
        }
    }

    /// Adds a lifecycle component to manage.
    pub fn add(&mut self, component: Box<dyn Lifecycle>) {
        self.components.push(component);
    }

    /// Starts all managed components in order.
    /// 
    /// # Errors
    /// Returns the first error encountered. Already-started components
    /// are not stopped automatically.
    pub async fn start_all(&mut self) -> Result<()> {
        for component in &mut self.components {
            component.start().await?;
        }
        Ok(())
    }

    /// Stops all managed components in reverse order.
    /// 
    /// # Errors
    /// Attempts to stop all components even if errors occur.
    /// Returns the first error encountered.
    pub async fn stop_all(&mut self) -> Result<()> {
        let mut first_error: Option<hsu_common::Error> = None;

        // Stop in reverse order
        for component in self.components.iter_mut().rev() {
            if let Err(e) = component.stop().await {
                if first_error.is_none() {
                    first_error = Some(e);
                }
            }
        }

        match first_error {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

impl Default for LifecycleManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mock component for testing
    struct MockComponent {
        started: bool,
    }

    #[async_trait]
    impl Lifecycle for MockComponent {
        async fn start(&mut self) -> Result<()> {
            self.started = true;
            Ok(())
        }

        async fn stop(&mut self) -> Result<()> {
            self.started = false;
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_lifecycle_manager() {
        let mut manager = LifecycleManager::new();
        
        let component1 = Box::new(MockComponent { started: false });
        let component2 = Box::new(MockComponent { started: false });
        
        manager.add(component1);
        manager.add(component2);
        
        // Start all
        manager.start_all().await.unwrap();
        
        // Stop all
        manager.stop_all().await.unwrap();
    }
}

