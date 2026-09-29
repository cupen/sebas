//! Example 1: Basic Hello World
//!
//! This example demonstrates basic usage of the Claude Agent SDK to write
//! a simple Python "Hello, World!" program using ClaudeClient for full
//! tool call support.
//!
//! What it does:
//! 1. Creates a ClaudeClient with Write tool allowed
//! 2. Asks Claude to write a Python hello world script
//! 3. Saves it to ./fixtures/hello.py
//! 4. Runs the script to verify it works

use claude_agent_sdk::{ClaudeAgentOptions, ClaudeClient};
use futures::stream::StreamExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("=== Example 1: Hello World ===\n");

    // Create output directory
    std::fs::create_dir_all("./fixtures")?;

    // Configure options to allow Write tool using the builder pattern
    let options = ClaudeAgentOptions::builder()
        .allowed_tools(vec!["Write".to_string()])
        .permission_mode(claude_agent_sdk::PermissionMode::AcceptEdits)
        .max_turns(5)
        .stderr_callback(std::sync::Arc::new(|msg| {
            eprintln!("STDERR: {}", msg);
        }))
        .build();

    // Create and connect the client
    let mut client = ClaudeClient::new(options);
    client.connect().await?;

    println!("Asking Claude to write a Python hello world script...\n");

    // Query Claude using the client (which handles tool calls)
    client
        .query("Write a simple Python hello world script to ./fixtures/hello.py")
        .await?;

    // Receive the response stream
    let mut response_stream = client.receive_response();

    // Collect all messages from the stream
    while let Some(result) = response_stream.next().await {
        match result {
            Ok(message) => {
                println!("Received: {:?}", message);
            }
            Err(e) => {
                eprintln!("Error: {:?}", e);
                break;
            }
        }
    }

    // Verify the file was created
    let filepath = "./fixtures/hello.py";
    if std::path::Path::new(filepath).exists() {
        println!("\n✓ File created: {}", filepath);

        // Read and display the file
        let content = std::fs::read_to_string(filepath)?;
        println!("\nFile contents:");
        println!("---");
        println!("{}", content);
        println!("---");

        // Try to run it
        println!("\nRunning the script...");
        let output = std::process::Command::new("python3")
            .arg(filepath)
            .output()?;

        if output.status.success() {
            println!("✓ Script executed successfully!");
            println!("Output: {}", String::from_utf8_lossy(&output.stdout));
        } else {
            println!("✗ Script failed to execute");
            println!("Error: {}", String::from_utf8_lossy(&output.stderr));
        }
    } else {
        println!("\n✗ File was not created");
    }

    Ok(())
}
