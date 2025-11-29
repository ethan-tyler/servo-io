//! Timeout Chaos Tests
//!
//! These tests validate timeout behavior and ensure operations
//! don't hang indefinitely under failure conditions.

use std::time::Duration;

#[tokio::test]
async fn timeout_wrapper_cancels_slow_operations() {
    let start = std::time::Instant::now();

    let result = tokio::time::timeout(Duration::from_millis(50), async {
        // Simulate a slow operation
        tokio::time::sleep(Duration::from_secs(10)).await;
        "completed"
    })
    .await;

    let elapsed = start.elapsed();

    // Should timeout, not complete
    assert!(result.is_err(), "Should have timed out");
    assert!(
        elapsed < Duration::from_millis(100),
        "Should timeout quickly, took {:?}",
        elapsed
    );
}

#[tokio::test]
async fn timeout_allows_fast_operations() {
    let result = tokio::time::timeout(Duration::from_secs(1), async {
        tokio::time::sleep(Duration::from_millis(10)).await;
        "completed"
    })
    .await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "completed");
}

#[tokio::test]
async fn timeout_propagates_result() {
    // Success case
    let result: Result<Result<i32, &str>, _> =
        tokio::time::timeout(Duration::from_secs(1), async { Ok(42) }).await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap().unwrap(), 42);

    // Error case
    let result: Result<Result<i32, &str>, _> =
        tokio::time::timeout(Duration::from_secs(1), async { Err("error") }).await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap().unwrap_err(), "error");
}

#[tokio::test]
async fn nested_timeouts_respect_inner() {
    let start = std::time::Instant::now();

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::time::timeout(Duration::from_millis(50), async {
            tokio::time::sleep(Duration::from_secs(10)).await;
            "completed"
        }),
    )
    .await;

    let elapsed = start.elapsed();

    // Inner timeout should trigger first
    assert!(result.is_ok()); // Outer didn't timeout
    assert!(result.unwrap().is_err()); // Inner did timeout
    assert!(elapsed < Duration::from_millis(200));
}

#[tokio::test]
async fn concurrent_timeouts_are_independent() {
    let fast = tokio::time::timeout(Duration::from_secs(1), async {
        tokio::time::sleep(Duration::from_millis(10)).await;
        "fast"
    });

    let slow = tokio::time::timeout(Duration::from_millis(50), async {
        tokio::time::sleep(Duration::from_secs(10)).await;
        "slow"
    });

    let (fast_result, slow_result) = tokio::join!(fast, slow);

    assert!(fast_result.is_ok());
    assert_eq!(fast_result.unwrap(), "fast");

    assert!(slow_result.is_err());
}

#[tokio::test]
async fn deadline_enforcement_with_select() {
    use tokio::select;

    let start = std::time::Instant::now();
    let deadline = Duration::from_millis(100);

    let result = select! {
        _ = tokio::time::sleep(deadline) => {
            Err("deadline exceeded")
        }
        result = async {
            tokio::time::sleep(Duration::from_secs(10)).await;
            Ok("completed")
        } => {
            result
        }
    };

    let elapsed = start.elapsed();

    assert!(result.is_err());
    assert!(elapsed < Duration::from_millis(200));
}

#[tokio::test]
async fn cancellation_safety() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let cleanup_ran = Arc::new(AtomicBool::new(false));
    let cleanup_clone = cleanup_ran.clone();

    let result = tokio::time::timeout(Duration::from_millis(50), async move {
        // Simulate long operation with cleanup
        struct Guard {
            flag: Arc<AtomicBool>,
        }

        impl Drop for Guard {
            fn drop(&mut self) {
                self.flag.store(true, Ordering::SeqCst);
            }
        }

        let _guard = Guard {
            flag: cleanup_clone,
        };
        tokio::time::sleep(Duration::from_secs(10)).await;
        "completed"
    })
    .await;

    // Timeout occurred
    assert!(result.is_err());

    // Give time for cleanup
    tokio::time::sleep(Duration::from_millis(10)).await;

    // Cleanup should have run when the future was dropped
    assert!(
        cleanup_ran.load(Ordering::SeqCst),
        "Cleanup should run on cancellation"
    );
}
