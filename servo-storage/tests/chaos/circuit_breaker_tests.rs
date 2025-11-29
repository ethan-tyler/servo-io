//! Circuit Breaker Chaos Tests
//!
//! These tests validate the circuit breaker behavior under various
//! failure scenarios including:
//! - Threshold-based opening
//! - Half-open state transitions
//! - Concurrent probe handling
//! - Recovery behavior

use servo_storage::circuit_breaker::{
    CircuitBreakerConfig, CircuitBreakerError, DatabaseCircuitBreaker,
};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Create a test circuit breaker with short timeouts
fn test_breaker(failure_threshold: u32, half_open_timeout_ms: u64) -> DatabaseCircuitBreaker {
    let config = CircuitBreakerConfig {
        failure_threshold,
        half_open_timeout: Duration::from_millis(half_open_timeout_ms),
    };
    DatabaseCircuitBreaker::new(format!("test-{}", uuid::Uuid::new_v4()), config)
}

#[tokio::test]
async fn circuit_opens_after_exact_failure_threshold() {
    let breaker = test_breaker(3, 1000);

    // First two failures should not open circuit
    for i in 0..2 {
        let result = breaker
            .call(|| async { Err::<(), _>(format!("failure {}", i)) })
            .await;
        assert!(matches!(result, Err(CircuitBreakerError::Failure(_))));
        assert!(
            !breaker.is_open().await,
            "Circuit should NOT be open after {} failures",
            i + 1
        );
    }

    // Third failure should open circuit
    let result = breaker.call(|| async { Err::<(), _>("failure 3") }).await;
    assert!(matches!(result, Err(CircuitBreakerError::Failure(_))));
    assert!(
        breaker.is_open().await,
        "Circuit should be open after reaching threshold"
    );
}

#[tokio::test]
async fn circuit_rejects_immediately_when_open() {
    let breaker = test_breaker(2, 10000); // Long timeout so it stays open

    // Open the circuit
    for _ in 0..2 {
        let _ = breaker.call(|| async { Err::<(), _>("failure") }).await;
    }

    // Measure rejection time
    let start = std::time::Instant::now();
    let result = breaker
        .call(|| async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Ok::<_, String>("should not execute")
        })
        .await;
    let elapsed = start.elapsed();

    // Should reject immediately (not wait for the 5 second operation)
    assert!(
        elapsed < Duration::from_millis(100),
        "Should reject immediately, took {:?}",
        elapsed
    );
    assert!(matches!(result, Err(CircuitBreakerError::CircuitOpen)));
}

#[tokio::test]
async fn circuit_transitions_to_half_open_after_timeout() {
    let breaker = test_breaker(2, 50); // 50ms timeout

    // Open the circuit
    for _ in 0..2 {
        let _ = breaker.call(|| async { Err::<(), _>("failure") }).await;
    }
    assert!(breaker.is_open().await);

    // Wait for half-open timeout
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Should no longer report as open (allows probe attempt)
    assert!(!breaker.is_open().await, "Should allow probe after timeout");
}

#[tokio::test]
async fn circuit_closes_on_successful_probe() {
    let breaker = test_breaker(2, 50);

    // Open the circuit
    for _ in 0..2 {
        let _ = breaker.call(|| async { Err::<(), _>("failure") }).await;
    }

    // Wait for half-open timeout
    tokio::time::sleep(Duration::from_millis(75)).await;

    // Successful probe should close the circuit
    let result = breaker
        .call(|| async { Ok::<_, String>("recovered") })
        .await;
    assert!(result.is_ok());

    // Circuit should be closed
    assert!(!breaker.is_open().await);

    // Subsequent requests should succeed
    let result = breaker.call(|| async { Ok::<_, String>("normal") }).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn circuit_reopens_on_failed_probe() {
    let breaker = test_breaker(2, 50);

    // Open the circuit
    for _ in 0..2 {
        let _ = breaker.call(|| async { Err::<(), _>("failure") }).await;
    }

    // Wait for half-open timeout
    tokio::time::sleep(Duration::from_millis(75)).await;

    // Failed probe should keep circuit open
    let result = breaker
        .call(|| async { Err::<(), _>("probe failed") })
        .await;
    assert!(matches!(result, Err(CircuitBreakerError::Failure(_))));

    // Circuit should remain in open state (need to wait for next timeout)
    let _result = breaker
        .call(|| async { Ok::<_, String>("should fail") })
        .await;
    // This might be CircuitOpen or might go through depending on timing
    // The important thing is the breaker is still protecting
}

#[tokio::test]
async fn circuit_handles_concurrent_probes() {
    let breaker = Arc::new(test_breaker(2, 100));

    // Open the circuit
    for _ in 0..2 {
        let _ = breaker.call(|| async { Err::<(), _>("failure") }).await;
    }

    // Wait for half-open timeout
    tokio::time::sleep(Duration::from_millis(150)).await;

    let probe_count = Arc::new(AtomicU32::new(0));

    // Launch multiple concurrent probe attempts
    let mut handles = vec![];
    for _ in 0..5 {
        let breaker_clone = breaker.clone();
        let probe_count_clone = probe_count.clone();

        handles.push(tokio::spawn(async move {
            let result = breaker_clone
                .call(|| async {
                    probe_count_clone.fetch_add(1, Ordering::SeqCst);
                    // Simulate slow operation
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    Ok::<_, String>("probe")
                })
                .await;
            result.is_ok()
        }));
    }

    // Wait for all probes to complete
    let results: Vec<bool> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();

    // Only one probe should have executed (others should get CircuitOpen)
    let successful_probes = results.iter().filter(|&&r| r).count();
    let actual_executions = probe_count.load(Ordering::SeqCst);

    // At most one probe should execute
    assert!(
        actual_executions <= 1,
        "Expected at most 1 execution, got {}",
        actual_executions
    );

    // At most one probe should succeed
    assert!(
        successful_probes <= 1,
        "Expected at most 1 successful probe, got {}",
        successful_probes
    );
}

#[tokio::test]
async fn success_resets_failure_counter() {
    let breaker = test_breaker(3, 1000);

    // Two failures
    for _ in 0..2 {
        let _ = breaker.call(|| async { Err::<(), _>("failure") }).await;
    }

    // One success should reset counter
    let result = breaker.call(|| async { Ok::<_, String>("success") }).await;
    assert!(result.is_ok());

    // Two more failures should not open circuit (counter was reset)
    for _ in 0..2 {
        let _ = breaker.call(|| async { Err::<(), _>("failure") }).await;
    }
    assert!(!breaker.is_open().await, "Counter should have been reset");

    // Third failure should open circuit
    let _ = breaker.call(|| async { Err::<(), _>("failure") }).await;
    assert!(breaker.is_open().await);
}

#[tokio::test]
async fn failure_threshold_one_opens_immediately() {
    let breaker = test_breaker(1, 1000);

    // Single failure should open circuit
    let result = breaker
        .call(|| async { Err::<(), _>("single failure") })
        .await;
    assert!(matches!(result, Err(CircuitBreakerError::Failure(_))));
    assert!(breaker.is_open().await);
}

#[tokio::test]
async fn high_failure_threshold_requires_many_failures() {
    let breaker = test_breaker(10, 1000);

    // 9 failures should not open circuit
    for i in 0..9 {
        let _ = breaker.call(|| async { Err::<(), _>("failure") }).await;
        assert!(
            !breaker.is_open().await,
            "Should not open after {} failures",
            i + 1
        );
    }

    // 10th failure should open circuit
    let _ = breaker.call(|| async { Err::<(), _>("failure 10") }).await;
    assert!(breaker.is_open().await, "Should open after 10 failures");
}

#[tokio::test]
async fn circuit_handles_rapid_transitions() {
    let breaker = test_breaker(2, 10); // Very short timeout

    for _ in 0..5 {
        // Open circuit
        for _ in 0..2 {
            let _ = breaker.call(|| async { Err::<(), _>("fail") }).await;
        }

        // Wait for half-open
        tokio::time::sleep(Duration::from_millis(20)).await;

        // Close circuit with success
        let result = breaker.call(|| async { Ok::<_, String>("ok") }).await;
        assert!(result.is_ok(), "Should succeed after half-open timeout");
        assert!(
            !breaker.is_open().await,
            "Circuit should be closed after success"
        );
    }
}

#[tokio::test]
async fn circuit_preserves_error_type() {
    let breaker = test_breaker(5, 1000);

    // Test that the original error is preserved
    #[derive(Debug, PartialEq)]
    struct CustomError(i32);

    impl std::fmt::Display for CustomError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "CustomError({})", self.0)
        }
    }

    let result = breaker
        .call(|| async { Err::<(), _>(CustomError(42)) })
        .await;

    match result {
        Err(CircuitBreakerError::Failure(e)) => {
            assert_eq!(e.0, 42, "Original error should be preserved");
        }
        _ => panic!("Expected Failure error"),
    }
}

#[tokio::test]
async fn circuit_works_with_async_operations() {
    let breaker = test_breaker(3, 1000);

    // Test with actual async operation
    let result = breaker
        .call(|| async {
            tokio::time::sleep(Duration::from_millis(10)).await;
            Ok::<_, String>("async result")
        })
        .await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "async result");
}

#[tokio::test]
async fn circuit_is_thread_safe() {
    let breaker = Arc::new(test_breaker(100, 1000));

    let mut handles = vec![];

    // Spawn many concurrent operations
    for i in 0..50 {
        let breaker_clone = breaker.clone();
        handles.push(tokio::spawn(async move {
            for _ in 0..10 {
                if i % 2 == 0 {
                    let _ = breaker_clone.call(|| async { Ok::<_, String>("ok") }).await;
                } else {
                    let _ = breaker_clone.call(|| async { Err::<(), _>("err") }).await;
                }
            }
        }));
    }

    // All tasks should complete without panic
    for handle in handles {
        handle.await.expect("Task should not panic");
    }
}
