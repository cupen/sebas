//! Slash Commands system for Claude Agent SDK
//!
//! Provides a flexible command registration and execution system.

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Error type for command operations
#[derive(Debug, Clone)]
pub enum CommandError {
    /// Command not found in registry
    NotFound(String),
    /// Command execution failed
    ExecutionFailed(String),
    /// Invalid command name
    InvalidName(String),
    /// Command already registered
    AlreadyRegistered(String),
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandError::NotFound(name) => write!(f, "Command not found: {}", name),
            CommandError::ExecutionFailed(msg) => write!(f, "Command execution failed: {}", msg),
            CommandError::InvalidName(name) => write!(f, "Invalid command name: {}", name),
            CommandError::AlreadyRegistered(name) => {
                write!(f, "Command already registered: {}", name)
            }
        }
    }
}

impl std::error::Error for CommandError {}

/// Type alias for async command handlers
///
/// Commands receive:
/// - The command name (for handlers that serve multiple commands)
/// - Command arguments as a vector of strings
///
/// Commands return:
/// - A result containing either a String output or a CommandError
pub type CommandHandler = Arc<
    dyn Fn(&str, Vec<String>) -> Pin<Box<dyn Future<Output = Result<String, CommandError>> + Send>>
        + Send
        + Sync,
>;

/// A slash command with metadata and handler
#[derive(Clone)]
pub struct SlashCommand {
    /// Unique command name (e.g., "help", "status", "deploy")
    pub name: String,
    /// Human-readable description
    pub description: String,
    /// Async handler function
    pub handler: CommandHandler,
}

impl SlashCommand {
    /// Create a new slash command
    ///
    /// # Arguments
    /// * `name` - Unique command identifier
    /// * `description` - Human-readable description
    /// * `handler` - Async function handling command execution
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        handler: CommandHandler,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            handler,
        }
    }

    /// Validate command name
    fn validate_name(name: &str) -> Result<(), CommandError> {
        if name.is_empty() {
            return Err(CommandError::InvalidName("Command name cannot be empty".to_string()));
        }
        if name.contains(' ') {
            return Err(CommandError::InvalidName(
                "Command name cannot contain spaces".to_string(),
            ));
        }
        if !name.chars().next().unwrap().is_alphabetic() {
            return Err(CommandError::InvalidName(
                "Command name must start with a letter".to_string(),
            ));
        }
        Ok(())
    }
}

/// Registry for managing slash commands
#[derive(Default)]
pub struct CommandRegistry {
    commands: HashMap<String, SlashCommand>,
}

impl CommandRegistry {
    /// Create a new empty command registry
    pub fn new() -> Self {
        Self {
            commands: HashMap::new(),
        }
    }

    /// Register a new command
    ///
    /// # Arguments
    /// * `command` - SlashCommand to register
    ///
    /// # Returns
    /// * `Ok(())` if registration successful
    /// * `Err(CommandError)` if name is invalid or already registered
    pub fn register(&mut self, command: SlashCommand) -> Result<(), CommandError> {
        SlashCommand::validate_name(&command.name)?;

        if self.commands.contains_key(&command.name) {
            return Err(CommandError::AlreadyRegistered(command.name));
        }

        self.commands.insert(command.name.clone(), command);
        Ok(())
    }

    /// Execute a command by name
    ///
    /// # Arguments
    /// * `name` - Command name to execute
    /// * `args` - Command arguments
    ///
    /// # Returns
    /// * `Ok(String)` - Command output
    /// * `Err(CommandError)` - If command not found or execution fails
    pub async fn execute(&self, name: &str, args: Vec<String>) -> Result<String, CommandError> {
        let command = self
            .commands
            .get(name)
            .ok_or_else(|| CommandError::NotFound(name.to_string()))?;

        (command.handler)(name, args).await
    }

    /// Check if a command exists
    pub fn exists(&self, name: &str) -> bool {
        self.commands.contains_key(name)
    }

    /// Get a command by name
    pub fn get(&self, name: &str) -> Option<&SlashCommand> {
        self.commands.get(name)
    }

    /// Get all registered command names
    pub fn list_names(&self) -> Vec<String> {
        self.commands.keys().cloned().collect()
    }

    /// Get all commands
    pub fn list_all(&self) -> Vec<&SlashCommand> {
        self.commands.values().collect()
    }

    /// Get the number of registered commands
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    /// Check if registry is empty
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Unregister a command
    ///
    /// # Returns
    /// * `Ok(())` if command was removed
    /// * `Err(CommandError::NotFound)` if command doesn't exist
    pub fn unregister(&mut self, name: &str) -> Result<(), CommandError> {
        self.commands
            .remove(name)
            .ok_or_else(|| CommandError::NotFound(name.to_string()))?;
        Ok(())
    }

    /// Clear all commands
    pub fn clear(&mut self) {
        self.commands.clear();
    }
}

impl fmt::Debug for SlashCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SlashCommand")
            .field("name", &self.name)
            .field("description", &self.description)
            .field("handler", &"<function>")
            .finish()
    }
}

impl fmt::Debug for CommandRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CommandRegistry")
            .field("commands_count", &self.commands.len())
            .field("command_names", &self.list_names())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper function to create a test command
    fn create_test_command(name: &str, description: &str) -> SlashCommand {
        SlashCommand::new(
            name,
            description,
            Arc::new(|_name, args| {
                Box::pin(async move {
                    Ok(format!("Executed with args: {:?}", args))
                })
            }),
        )
    }

    #[test]
    fn test_registry_creation() {
        let registry = CommandRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn test_registry_default() {
        let registry = CommandRegistry::default();
        assert!(registry.is_empty());
    }

    #[test]
    fn test_register_command() {
        let mut registry = CommandRegistry::new();
        let cmd = create_test_command("test", "A test command");

        assert!(registry.register(cmd).is_ok());
        assert_eq!(registry.len(), 1);
        assert!(registry.exists("test"));
    }

    #[test]
    fn test_register_duplicate_fails() {
        let mut registry = CommandRegistry::new();
        let cmd1 = create_test_command("test", "First command");
        let cmd2 = create_test_command("test", "Duplicate command");

        assert!(registry.register(cmd1).is_ok());
        let result = registry.register(cmd2);
        assert!(matches!(result, Err(CommandError::AlreadyRegistered(_))));
    }

    #[test]
    fn test_invalid_name_empty() {
        let cmd = SlashCommand::new(
            "",
            "description",
            Arc::new(|_name, _args| Box::pin(async { Ok(String::new()) })),
        );

        let result = SlashCommand::validate_name(&cmd.name);
        assert!(matches!(result, Err(CommandError::InvalidName(_))));
    }

    #[test]
    fn test_invalid_name_contains_space() {
        let cmd = SlashCommand::new(
            "test command",
            "description",
            Arc::new(|_name, _args| Box::pin(async { Ok(String::new()) })),
        );

        let result = SlashCommand::validate_name(&cmd.name);
        assert!(matches!(result, Err(CommandError::InvalidName(_))));
    }

    #[test]
    fn test_invalid_name_starts_with_number() {
        let cmd = SlashCommand::new(
            "123test",
            "description",
            Arc::new(|_name, _args| Box::pin(async { Ok(String::new()) })),
        );

        let result = SlashCommand::validate_name(&cmd.name);
        assert!(matches!(result, Err(CommandError::InvalidName(_))));
    }

    #[test]
    fn test_valid_name() {
        assert!(SlashCommand::validate_name("test").is_ok());
        assert!(SlashCommand::validate_name("test_command").is_ok());
        assert!(SlashCommand::validate_name("test-command").is_ok());
        assert!(SlashCommand::validate_name("TestCommand").is_ok());
    }

    #[test]
    fn test_execute_command() {
        let mut registry = CommandRegistry::new();
        let cmd = create_test_command("echo", "Echo arguments");
        registry.register(cmd).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(registry.execute("echo", vec!["hello".to_string()]));

        assert!(result.is_ok());
        assert!(result.unwrap().contains("hello"));
    }

    #[test]
    fn test_execute_nonexistent_command() {
        let registry = CommandRegistry::new();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(registry.execute("nonexistent", vec![]));

        assert!(matches!(result, Err(CommandError::NotFound(_))));
    }

    #[test]
    fn test_get_command() {
        let mut registry = CommandRegistry::new();
        let cmd = create_test_command("test", "A test command");
        registry.register(cmd).unwrap();

        let retrieved = registry.get("test");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().name, "test");
    }

    #[test]
    fn test_get_nonexistent_command() {
        let registry = CommandRegistry::new();
        assert!(registry.get("nonexistent").is_none());
    }

    #[test]
    fn test_list_names() {
        let mut registry = CommandRegistry::new();
        registry.register(create_test_command("cmd1", "First")).unwrap();
        registry.register(create_test_command("cmd2", "Second")).unwrap();
        registry.register(create_test_command("cmd3", "Third")).unwrap();

        let names = registry.list_names();
        assert_eq!(names.len(), 3);
        assert!(names.contains(&"cmd1".to_string()));
        assert!(names.contains(&"cmd2".to_string()));
        assert!(names.contains(&"cmd3".to_string()));
    }

    #[test]
    fn test_list_all() {
        let mut registry = CommandRegistry::new();
        registry.register(create_test_command("cmd1", "First")).unwrap();
        registry.register(create_test_command("cmd2", "Second")).unwrap();

        let commands = registry.list_all();
        assert_eq!(commands.len(), 2);
    }

    #[test]
    fn test_unregister_command() {
        let mut registry = CommandRegistry::new();
        registry.register(create_test_command("test", "A test command")).unwrap();

        assert!(registry.unregister("test").is_ok());
        assert!(!registry.exists("test"));
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn test_unregister_nonexistent_command() {
        let mut registry = CommandRegistry::new();
        let result = registry.unregister("nonexistent");
        assert!(matches!(result, Err(CommandError::NotFound(_))));
    }

    #[test]
    fn test_clear_commands() {
        let mut registry = CommandRegistry::new();
        registry.register(create_test_command("cmd1", "First")).unwrap();
        registry.register(create_test_command("cmd2", "Second")).unwrap();

        registry.clear();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn test_command_error_display() {
        assert!(format!("{}", CommandError::NotFound("test".to_string())).contains("test"));
        assert!(format!("{}", CommandError::ExecutionFailed("error".to_string())).contains("error"));
        assert!(format!("{}", CommandError::InvalidName("bad".to_string())).contains("bad"));
        assert!(format!("{}", CommandError::AlreadyRegistered("cmd".to_string())).contains("cmd"));
    }

    #[test]
    fn test_complex_command_handler() {
        let mut registry = CommandRegistry::new();

        let cmd = SlashCommand::new(
            "sum",
            "Sum numbers",
            Arc::new(|_name, args| {
                Box::pin(async move {
                    let sum: i32 = args
                        .iter()
                        .map(|s| s.parse::<i32>().unwrap_or(0))
                        .sum();
                    Ok(format!("Sum: {}", sum))
                })
            }),
        );

        registry.register(cmd).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(registry.execute(
            "sum",
            vec!["10".to_string(), "20".to_string(), "30".to_string()],
        ));

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "Sum: 60");
    }

    #[test]
    fn test_async_error_handling() {
        let mut registry = CommandRegistry::new();

        let cmd = SlashCommand::new(
            "failing",
            "Always fails",
            Arc::new(|_name, _args| {
                Box::pin(async move {
                    Err(CommandError::ExecutionFailed("Intentional failure".to_string()))
                })
            }),
        );

        registry.register(cmd).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(registry.execute("failing", vec![]));

        assert!(matches!(result, Err(CommandError::ExecutionFailed(_))));
    }
}
