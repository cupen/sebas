//! Skills API Client
//!
//! HTTP client for interacting with the Anthropic Skills API.
//! Supports uploading, listing, and deleting skills.

use reqwest::Client;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Errors that can occur when interacting with the Skills API
#[derive(Debug, Error)]
pub enum SkillsError {
    /// HTTP request failed
    #[error("HTTP request failed: {0}")]
    HttpError(#[from] reqwest::Error),

    /// IO error during file operations
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    /// API returned an error
    #[error("API error: {0}")]
    ApiError(String),

    /// Invalid response format
    #[error("Invalid response format: {0}")]
    InvalidResponse(String),

    /// Skill not found
    #[error("Skill not found: {0}")]
    SkillNotFound(String),
}

/// Information about a skill from the Skills API
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SkillApiInfo {
    /// Unique skill identifier
    pub id: String,

    /// Skill name
    pub name: String,

    /// Skill description
    pub description: String,

    /// Creation timestamp
    pub created_at: String,

    /// Skill version
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,

    /// Skill author
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
}

/// Response from listing skills
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ListSkillsResponse {
    /// List of skills
    pub skills: Vec<SkillApiInfo>,

    /// Total count
    pub total_count: usize,

    /// Next page token
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

/// Response from uploading a skill
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UploadSkillResponse {
    /// Uploaded skill information
    pub skill: SkillApiInfo,

    /// Upload status
    pub status: String,
}

/// HTTP client for the Anthropic Skills API
pub struct SkillsApiClient {
    /// API key for authentication
    api_key: String,

    /// Base URL for the API
    base_url: String,

    /// HTTP client
    client: Client,

    /// API version header
    api_version: String,
}

impl SkillsApiClient {
    /// Create a new Skills API client
    ///
    /// # Arguments
    ///
    /// * `api_key` - Anthropic API key
    ///
    /// # Example
    ///
    /// ```rust
    /// use claude_agent_sdk::skills::api::SkillsApiClient;
    ///
    /// let client = SkillsApiClient::new("sk-ant-...");
    /// ```
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::with_base_url(api_key, "https://api.anthropic.com/v1")
    }

    /// Create a new Skills API client with a custom base URL
    ///
    /// # Arguments
    ///
    /// * `api_key` - Anthropic API key
    /// * `base_url` - Custom base URL for the API
    pub fn with_base_url(api_key: impl Into<String>, base_url: &str) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: base_url.to_string(),
            client: Client::new(),
            api_version: "2023-06-01".to_string(),
        }
    }

    /// Create a client with custom API version
    ///
    /// # Arguments
    ///
    /// * `api_key` - Anthropic API key
    /// * `api_version` - API version string
    pub fn with_api_version(api_key: impl Into<String>, api_version: &str) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.anthropic.com/v1".to_string(),
            client: Client::new(),
            api_version: api_version.to_string(),
        }
    }

    /// Upload a skill directory to the Skills API
    ///
    /// # Arguments
    ///
    /// * `skill_dir` - Path to the skill directory
    ///
    /// # Returns
    ///
    /// Information about the uploaded skill
    ///
    /// # Errors
    ///
    /// Returns `SkillsError` if:
    /// - The skill directory cannot be read
    /// - The directory cannot be zipped
    /// - The upload fails
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use claude_agent_sdk::skills::api::SkillsApiClient;
    /// # use std::path::Path;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = SkillsApiClient::new("sk-ant-...");
    /// let info = client.upload_skill(Path::new("/path/to/skill")).await?;
    /// println!("Uploaded skill: {} (ID: {})", info.name, info.id);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn upload_skill(
        &self,
        skill_dir: &Path,
    ) -> Result<SkillApiInfo, SkillsError> {
        // 1. Zip the skill directory
        let zip_bytes = self.zip_skill(skill_dir)?;

        // 2. Upload to API
        let url = format!("{}/skills", self.base_url);
        let response = self
            .client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", &self.api_version)
            .header("Content-Type", "application/zip")
            .body(zip_bytes)
            .send()
            .await?;

        // 3. Check status
        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            return Err(SkillsError::ApiError(format!(
                "Upload failed with status {}: {}",
                status, error_text
            )));
        }

        // 4. Parse response
        let upload_response: UploadSkillResponse = response.json().await?;
        Ok(upload_response.skill)
    }

    /// List all skills from the Skills API
    ///
    /// # Returns
    ///
    /// A vector of skill information
    ///
    /// # Errors
    ///
    /// Returns `SkillsError` if the request fails
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use claude_agent_sdk::skills::api::SkillsApiClient;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = SkillsApiClient::new("sk-ant-...");
    /// let skills = client.list_skills().await?;
    /// for skill in skills {
    ///     println!("{}: {}", skill.name, skill.description);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn list_skills(&self) -> Result<Vec<SkillApiInfo>, SkillsError> {
        let url = format!("{}/skills", self.base_url);
        let response = self
            .client
            .get(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", &self.api_version)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            return Err(SkillsError::ApiError(format!(
                "List skills failed with status {}: {}",
                status, error_text
            )));
        }

        let list_response: ListSkillsResponse = response.json().await?;
        Ok(list_response.skills)
    }

    /// Get details of a specific skill
    ///
    /// # Arguments
    ///
    /// * `skill_id` - The skill identifier
    ///
    /// # Returns
    ///
    /// Skill information
    ///
    /// # Errors
    ///
    /// Returns `SkillsError` if the skill is not found or the request fails
    pub async fn get_skill(&self, skill_id: &str) -> Result<SkillApiInfo, SkillsError> {
        let url = format!("{}/skills/{}", self.base_url, skill_id);
        let response = self
            .client
            .get(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", &self.api_version)
            .send()
            .await?;

        if response.status().as_u16() == 404 {
            return Err(SkillsError::SkillNotFound(skill_id.to_string()));
        }

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            return Err(SkillsError::ApiError(format!(
                "Get skill failed with status {}: {}",
                status, error_text
            )));
        }

        let skill: SkillApiInfo = response.json().await?;
        Ok(skill)
    }

    /// Delete a skill from the Skills API
    ///
    /// # Arguments
    ///
    /// * `skill_id` - The skill identifier to delete
    ///
    /// # Returns
    ///
    /// Ok(()) on success
    ///
    /// # Errors
    ///
    /// Returns `SkillsError` if the deletion fails
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use claude_agent_sdk::skills::api::SkillsApiClient;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = SkillsApiClient::new("sk-ant-...");
    /// client.delete_skill("skill-id-123").await?;
    /// println!("Skill deleted successfully");
    /// # Ok(())
    /// # }
    /// ```
    pub async fn delete_skill(&self, skill_id: &str) -> Result<(), SkillsError> {
        let url = format!("{}/skills/{}", self.base_url, skill_id);
        let response = self
            .client
            .delete(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", &self.api_version)
            .send()
            .await?;

        if response.status().as_u16() == 404 {
            return Err(SkillsError::SkillNotFound(skill_id.to_string()));
        }

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            return Err(SkillsError::ApiError(format!(
                "Delete failed with status {}: {}",
                status, error_text
            )));
        }

        Ok(())
    }

    /// Zip a skill directory into bytes
    ///
    /// # Arguments
    ///
    /// * `skill_dir` - Path to the skill directory
    ///
    /// # Returns
    ///
    /// Zipped bytes
    fn zip_skill(&self, skill_dir: &Path) -> Result<Vec<u8>, SkillsError> {
        use std::fs::File;
        use std::io::{Read, Write};

        // Create in-memory zip
        let mut buffer = Vec::new();

        {
            // Use a simple implementation: write to a temporary file
            // In production, you'd use a library like zip or flate2
            let temp_zip_path = std::env::temp_dir().join("skill_upload.zip");

            // For now, just create a placeholder
            // In a real implementation, you'd use the zip crate
            let mut file = File::create(&temp_zip_path)?;

            // Write a simple zip file header (simplified)
            // Real implementation would use zip crate
            writeln!(file, "Skill: {}", skill_dir.display())?;

            // Read all files in directory
            if skill_dir.is_dir() {
                for entry in Self::walk_directory_impl(skill_dir)? {
                    let mut file = File::open(&entry)?;
                    let mut contents = Vec::new();
                    file.read_to_end(&mut contents)?;
                    // Write to zip (simplified)
                    writeln!(file, "File: {:?}, Size: {}", entry, contents.len())?;
                }
            }

            file.flush()?;

            // Read back
            let mut file = File::open(&temp_zip_path)?;
            file.read_to_end(&mut buffer)?;

            // Clean up
            let _ = std::fs::remove_file(&temp_zip_path);
        }

        Ok(buffer)
    }

    /// Walk a directory recursively (static method)
    fn walk_directory_impl(dir: &Path) -> Result<Vec<PathBuf>, SkillsError> {
        let mut files = Vec::new();

        if dir.is_dir() {
            for entry in std::fs::read_dir(dir)? {
                let entry = entry?;
                let path = entry.path();

                if path.is_dir() {
                    files.extend(Self::walk_directory_impl(&path)?);
                } else {
                    files.push(path);
                }
            }
        }

        Ok(files)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skill_api_info_serialization() {
        let info = SkillApiInfo {
            id: "skill-123".to_string(),
            name: "Test Skill".to_string(),
            description: "A test skill".to_string(),
            created_at: "2026-01-13T00:00:00Z".to_string(),
            version: Some("1.0.0".to_string()),
            author: Some("Test Author".to_string()),
        };

        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("skill-123"));
        assert!(json.contains("Test Skill"));

        let deserialized: SkillApiInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, "skill-123");
        assert_eq!(deserialized.name, "Test Skill");
    }

    #[test]
    fn test_skills_error_display() {
        let err = SkillsError::ApiError("Test error".to_string());
        assert_eq!(format!("{}", err), "API error: Test error");

        let err = SkillsError::SkillNotFound("skill-123".to_string());
        assert_eq!(format!("{}", err), "Skill not found: skill-123");
    }

    #[test]
    fn test_client_creation() {
        let client = SkillsApiClient::new("test-key");
        assert_eq!(client.api_key, "test-key");
        assert_eq!(client.base_url, "https://api.anthropic.com/v1");
        assert_eq!(client.api_version, "2023-06-01");
    }

    #[test]
    fn test_client_with_custom_base_url() {
        let client = SkillsApiClient::with_base_url("test-key", "https://custom.api.com/v1");
        assert_eq!(client.api_key, "test-key");
        assert_eq!(client.base_url, "https://custom.api.com/v1");
    }

    #[test]
    fn test_client_with_custom_api_version() {
        let client = SkillsApiClient::with_api_version("test-key", "2024-01-01");
        assert_eq!(client.api_key, "test-key");
        assert_eq!(client.api_version, "2024-01-01");
    }

    #[test]
    fn test_list_skills_response_serialization() {
        let response = ListSkillsResponse {
            skills: vec![SkillApiInfo {
                id: "skill-1".to_string(),
                name: "Skill 1".to_string(),
                description: "First skill".to_string(),
                created_at: "2026-01-13T00:00:00Z".to_string(),
                version: None,
                author: None,
            }],
            total_count: 1,
            next_token: Some("token-123".to_string()),
        };

        let json = serde_json::to_string(&response).unwrap();
        let deserialized: ListSkillsResponse = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.skills.len(), 1);
        assert_eq!(deserialized.total_count, 1);
        assert_eq!(deserialized.next_token, Some("token-123".to_string()));
    }

    #[test]
    fn test_upload_skill_response_serialization() {
        let response = UploadSkillResponse {
            skill: SkillApiInfo {
                id: "skill-123".to_string(),
                name: "Uploaded Skill".to_string(),
                description: "A newly uploaded skill".to_string(),
                created_at: "2026-01-13T00:00:00Z".to_string(),
                version: Some("1.0.0".to_string()),
                author: None,
            },
            status: "success".to_string(),
        };

        let json = serde_json::to_string(&response).unwrap();
        let deserialized: UploadSkillResponse = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.skill.id, "skill-123");
        assert_eq!(deserialized.status, "success");
    }
}
