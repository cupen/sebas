//! Advanced error handling and recovery patterns.
//!
//! This example demonstrates comprehensive error handling strategies including:
//! - Retry logic with exponential backoff
//! - Circuit breaker pattern
//! - Error aggregation and reporting
//! - Graceful degradation

use claude_agent_sdk::{ContentBlock, Message, query, types::config::ClaudeAgentOptions};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::time::sleep;

/// Circuit breaker state
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
enum CircuitState {
    Closed,   // Normal operation
    Open,     // Failing, reject requests
    HalfOpen, // Testing if service recovered
}

/// Circuit breaker for fault tolerance
struct CircuitBreaker {
    state: Arc<AtomicBool>,
    failure_count: Arc<std::sync::atomic::AtomicUsize>,
    threshold: usize,
}

impl CircuitBreaker {
    fn new(threshold: usize) -> Self {
        Self {
            state: Arc::new(AtomicBool::new(false)), // false = Closed
            failure_count: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            threshold,
        }
    }

    async fn call<F, T, E>(&self, f: F) -> Result<T, E>
    where
        F: std::future::Future<Output = Result<T, E>>,
    {
        // Check circuit state
        if self.state.load(Ordering::Acquire) {
            return Err(/* circuit open error */ unsafe { std::mem::zeroed() });
        }

        // Execute function
        match f.await {
            Ok(result) => {
                self.failure_count.store(0, Ordering::Release);
                Ok(result)
            },
            Err(e) => {
                let count = self.failure_count.fetch_add(1, Ordering::AcqRel) + 1;
                if count >= self.threshold {
                    self.state.store(true, Ordering::Release);
                    eprintln!("⚠️  Circuit breaker opened after {} failures", count);
                }
                Err(e)
            },
        }
    }

    async fn reset(&self) {
        sleep(Duration::from_secs(5)).await;
        self.state.store(false, Ordering::Release);
        self.failure_count.store(0, Ordering::Release);
        println!("✅ Circuit breaker reset");
    }
}

/// Retry with exponential backoff
async fn retry_with_backoff<F, T, E>(
    mut operation: F,
    max_retries: usize,
    initial_delay: Duration,
) -> Result<T, E>
where
    F: FnMut() -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, E>> + Send>>,
{
    let mut delay = initial_delay;
    let mut attempt = 0;

    loop {
        attempt += 1;
        println!("🔄 Attempt {}/{}", attempt, max_retries + 1);

        match operation().await {
            Ok(result) => {
                if attempt > 1 {
                    println!("✅ Success on attempt {}", attempt);
                }
                return Ok(result);
            },
            Err(e) => {
                if attempt > max_retries {
                    eprintln!("❌ Max retries ({}) exceeded", max_retries);
                    return Err(e);
                }

                eprintln!("⚠️  Attempt {} failed, retrying in {:?}...", attempt, delay);
                sleep(delay).await;

                // Exponential backoff with jitter
                delay = delay * 2 + Duration::from_millis(100);
            },
        }
    }
}

/// Error aggregation for batch operations
struct ErrorAggregator {
    errors: Vec<ErrorReport>,
}

#[allow(dead_code)]
struct ErrorReport {
    operation: String,
    error_message: String,
    timestamp: chrono::DateTime<chrono::Utc>,
    recovered: bool,
}

impl ErrorAggregator {
    fn new() -> Self {
        Self { errors: Vec::new() }
    }

    fn report(&mut self, operation: &str, error: &str, recovered: bool) {
        self.errors.push(ErrorReport {
            operation: operation.to_string(),
            error_message: error.to_string(),
            timestamp: chrono::Utc::now(),
            recovered,
        });
    }

    fn summary(&self) -> String {
        let total = self.errors.len();
        let recovered = self.errors.iter().filter(|e| e.recovered).count();
        let failed = total - recovered;

        format!(
            "📊 Error Summary:\n  Total: {}\n  Recovered: {}\n  Failed: {}\n  Recovery Rate: {:.1}%",
            total,
            recovered,
            failed,
            (recovered as f64 / total as f64) * 100.0
        )
    }
}

/// Graceful degradation: fallback to simpler approach
async fn query_with_fallback(prompt: &str) -> anyhow::Result<Vec<Message>> {
    // Try full-featured query first
    let result = query(prompt, None).await;

    match result {
        Ok(messages) => Ok(messages),
        Err(e) => {
            eprintln!(
                "⚠️  Full query failed: {}, trying simplified approach...",
                e
            );

            // Simplified query with minimal options
            let simple_options = ClaudeAgentOptions::builder().build();

            sleep(Duration::from_millis(500)).await;
            query(prompt, Some(simple_options)).await.map_err(|e2| {
                anyhow::anyhow!("Both full and simplified queries failed: {} | {}", e, e2)
            })
        },
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("🛡️  Advanced Error Handling Examples\n");

    // Example 1: Retry with exponential backoff
    println!("📡 Example 1: Retry with Exponential Backoff");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    let query_result = retry_with_backoff(
        || {
            Box::pin(async {
                // Simulate flaky operation
                static ATTEMPT: std::sync::atomic::AtomicUsize =
                    std::sync::atomic::AtomicUsize::new(0);

                let attempt = ATTEMPT.fetch_add(1, Ordering::SeqCst);
                if attempt < 2 {
                    eprintln!("  Simulating failure...");
                    Err(anyhow::anyhow!("Simulated failure").into())
                } else {
                    println!("  Success on attempt {}!", attempt + 1);
                    query("What is 2 + 2?", None).await
                }
            })
        },
        5,
        Duration::from_millis(100),
    )
    .await;

    match query_result {
        Ok(messages) => {
            println!("✅ Query succeeded after retries");
            if let Some(Message::Assistant(msg)) = messages.first() {
                for block in &msg.message.content {
                    if let ContentBlock::Text(text) = block {
                        println!("  Response: {}\n", text.text);
                    }
                }
            }
        },
        Err(e) => {
            eprintln!("❌ Query failed after all retries: {}\n", e);
        },
    }

    // Example 2: Circuit breaker pattern
    println!("⚡ Example 2: Circuit Breaker Pattern");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    let circuit_breaker = CircuitBreaker::new(3);

    for i in 1..=5 {
        println!("Request {}:", i);

        let result = circuit_breaker
            .call(async {
                if i <= 3 {
                    Err(anyhow::anyhow!("Service unavailable"))
                } else {
                    println!("  Service responding normally");
                    Ok("Success".to_string())
                }
            })
            .await;

        match result {
            Ok(_) => println!("  ✅ Request succeeded"),
            Err(_) => eprintln!("  ❌ Request rejected by circuit breaker"),
        }

        sleep(Duration::from_millis(100)).await;
    }

    println!();
    circuit_breaker.reset().await;

    // Example 3: Error aggregation
    println!("📊 Example 3: Error Aggregation");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    let mut aggregator = ErrorAggregator::new();
    let queries = vec![
        "What is 1 + 1?",
        "What is 2 + 2?",
        "What is 3 + 3?",
        "What is 4 + 4?",
    ];

    for (i, prompt) in queries.iter().enumerate() {
        println!("Query {}: {}", i + 1, prompt);

        match query(*prompt, None).await {
            Ok(_) => {
                println!("  ✅ Success");
            },
            Err(e) => {
                let error_msg = e.to_string();
                eprintln!("  ❌ Error: {}", error_msg);

                // Try recovery
                let recovered = query_with_fallback(prompt).await.is_ok();
                aggregator.report(*prompt, &error_msg, recovered);

                if recovered {
                    println!("  ✅ Recovered with fallback");
                }
            },
        }

        sleep(Duration::from_millis(200)).await;
    }

    println!("\n{}\n", aggregator.summary());

    // Example 4: Graceful degradation
    println!("🎯 Example 4: Graceful Degradation");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    let complex_query =
        "Explain quantum computing in detail with examples, applications, and future prospects";

    println!("Attempting complex query...");
    match query_with_fallback(complex_query).await {
        Ok(messages) => {
            println!("✅ Query succeeded (possibly with degradation)");
            if let Some(Message::Assistant(msg)) = messages.first() {
                for block in &msg.message.content {
                    if let ContentBlock::Text(text) = block {
                        println!("  Response length: {} characters", text.text.len());
                        // Print first 100 chars
                        let preview: String = text.text.chars().take(100).collect();
                        println!("  Preview: {}...\n", preview);
                    }
                }
            }
        },
        Err(e) => {
            eprintln!("❌ All attempts failed: {}", e);
        },
    }

    // Summary
    println!("{}", "=".repeat(50));
    println!("✅ Advanced Error Handling Examples Completed");
    println!("{}", "=".repeat(50));
    println!("\nKey Patterns:");
    println!("  🔄 Retry with exponential backoff for transient failures");
    println!("  ⚡ Circuit breaker to prevent cascading failures");
    println!("  📊 Error aggregation for batch operations");
    println!("  🎯 Graceful degradation for partial functionality");

    Ok(())
}
