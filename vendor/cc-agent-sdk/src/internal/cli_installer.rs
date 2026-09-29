//! Claude Code CLI 自动安装器
//!
//! 提供跨平台的 Claude CLI 自动下载和安装功能
//!
//! # 功能特性
//!
//! - ✅ 自动检测平台和架构
//! - ✅ 优先使用 npm 安装（最可靠）
//! - ✅ 失败时回退到直接下载
//! - ✅ 用户本地安装，无需 sudo
//! - ✅ 进度回调支持
//! - ✅ 完善的错误处理

use std::path::PathBuf;
use std::sync::Arc;
use tokio::process::Command;
use tracing::{info, warn, debug};

use crate::errors::{ClaudeError, Result};

/// 安装进度事件
///
/// 用于实时报告安装进度
#[derive(Debug, Clone)]
pub enum InstallProgress {
    /// 检查中
    Checking(String),
    /// 下载中
    Downloading {
        /// 已下载字节数
        current: u64,
        /// 总字节数（如果已知）
        total: Option<u64>,
    },
    /// 安装中
    Installing(String),
    /// 安装完成
    Done(PathBuf),
    /// 安装失败
    Failed(String),
}

/// CLI 安装器
///
/// 负责自动下载和安装 Claude Code CLI
pub struct CliInstaller {
    /// 是否自动安装
    pub auto_install: bool,
    /// 进度回调
    progress_callback: Option<Arc<dyn Fn(InstallProgress) + Send + Sync>>,
}

impl CliInstaller {
    /// 创建新的安装器
    pub fn new(auto_install: bool) -> Self {
        Self {
            auto_install,
            progress_callback: None,
        }
    }

    /// 设置进度回调
    pub fn with_progress_callback(
        mut self,
        callback: Arc<dyn Fn(InstallProgress) + Send + Sync>,
    ) -> Self {
        self.progress_callback = Some(callback);
        self
    }

    /// 报告进度
    fn report_progress(&self, event: InstallProgress) {
        if let Some(ref callback) = self.progress_callback {
            callback(event);
        }
    }

    /// 如果需要则安装 CLI
    ///
    /// # 返回
    ///
    /// 成功时返回 CLI 可执行文件的路径
    ///
    /// # 错误
    ///
    /// 如果安装失败或自动安装未启用，返回错误
    pub async fn install_if_needed(&self) -> Result<PathBuf> {
        // 如果未启用自动安装，直接返回错误
        if !self.auto_install {
            return Err(ClaudeError::InternalError(
                "Auto-install is disabled".to_string(),
            ));
        }

        self.report_progress(InstallProgress::Checking(
            "Checking if Claude CLI is already installed...".to_string(),
        ));

        // 首先检查是否已经安装
        if let Ok(path) = Self::find_existing_cli().await {
            info!("Claude CLI already installed at: {}", path.display());
            return Ok(path);
        }

        info!("Claude CLI not found, attempting auto-install...");
        self.report_progress(InstallProgress::Checking(
            "Claude CLI not found, starting installation...".to_string(),
        ));

        // 尝试通过 npm 安装
        match self.install_via_npm().await {
            Ok(path) => {
                self.report_progress(InstallProgress::Done(path.clone()));
                return Ok(path);
            }
            Err(e) => {
                warn!("npm installation failed: {}, trying direct download...", e);
            }
        }

        // 尝试直接下载
        match self.install_via_direct_download().await {
            Ok(path) => {
                self.report_progress(InstallProgress::Done(path.clone()));
                Ok(path)
            }
            Err(e) => {
                let error_msg = format!(
                    "Failed to install Claude CLI automatically. \
                     Please install manually: npm install -g @anthropic-ai/claude-code\n\nError: {}",
                    e
                );
                self.report_progress(InstallProgress::Failed(error_msg.clone()));
                Err(ClaudeError::InternalError(error_msg))
            }
        }
    }

    /// 查找已存在的 CLI
    async fn find_existing_cli() -> Result<PathBuf> {
        // 尝试执行 claude --version
        if let Ok(output) = Command::new("claude")
            .arg("--version")
            .output()
            .await
        {
            if output.status.success() {
                return Ok(PathBuf::from("claude"));
            }
        }

        Err(ClaudeError::InternalError("CLI not found".to_string()))
    }

    /// 通过 npm 安装
    ///
    /// 这是首选方法，因为 npm 处理了：
    /// - 平台特定的二进制文件
    /// - 版本管理
    /// - PATH 配置
    /// - 更新和卸载
    async fn install_via_npm(&self) -> Result<PathBuf> {
        info!("Attempting installation via npm...");

        self.report_progress(InstallProgress::Installing(
            "Installing via npm...".to_string(),
        ));

        // 检查 npm 是否可用
        let npm_check = Command::new("npm")
            .arg("--version")
            .output()
            .await;

        let npm_available = match npm_check {
            Ok(output) => output.status.success(),
            Err(_) => false,
        };

        if !npm_available {
            return Err(ClaudeError::InternalError(
                "npm is not available".to_string(),
            ));
        }

        debug!("npm is available, proceeding with installation");

        // 尝试执行全局安装
        let output = Command::new("npm")
            .args(["install", "-g", "@anthropic-ai/claude-code"])
            .output()
            .await
            .map_err(|e| ClaudeError::InternalError(format!("Failed to run npm: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ClaudeError::InternalError(format!(
                "npm install failed: {}",
                stderr
            )));
        }

        info!("npm install completed successfully");

        // 等待文件系统更新
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        // 验证安装
        if let Ok(output) = Command::new("claude")
            .arg("--version")
            .output()
            .await
        {
            if output.status.success() {
                let version = String::from_utf8_lossy(&output.stdout);
                info!("✅ Claude CLI installed successfully via npm: {}", version.trim());
                return Ok(PathBuf::from("claude"));
            }
        }

        // npm 安装成功但找不到命令，可能是 PATH 问题
        // 尝试查找 npm 全局安装路径
        if let Ok(npm_path) = Self::find_npm_global_path().await {
            info!("Found CLI at npm global path: {}", npm_path.display());
            return Ok(npm_path);
        }

        Err(ClaudeError::InternalError(
            "Installation appeared to succeed but CLI not found in PATH".to_string(),
        ))
    }

    /// 查找 npm 全局安装路径
    async fn find_npm_global_path() -> Result<PathBuf> {
        let output = Command::new("npm")
            .args(["config", "get", "prefix"])
            .output()
            .await
            .map_err(|e| ClaudeError::InternalError(format!("Failed to get npm config: {}", e)))?;

        if !output.status.success() {
            return Err(ClaudeError::InternalError(
                "Failed to get npm global prefix".to_string(),
            ));
        }

        let prefix_str = String::from_utf8_lossy(&output.stdout);
        let prefix = prefix_str.trim();
        let bin_path = if cfg!(windows) {
            PathBuf::from(prefix).join("claude.cmd")
        } else {
            PathBuf::from(prefix).join("bin").join("claude")
        };

        if bin_path.exists() {
            Ok(bin_path)
        } else {
            Err(ClaudeError::InternalError(format!(
                "CLI not found at npm path: {}",
                bin_path.display()
            )))
        }
    }

    /// 通过直接下载安装
    ///
    /// 从 GitHub Releases 下载预编译的二进制文件
    ///
    /// 注意：此功能依赖于 GitHub Releases 的可用性
    async fn install_via_direct_download(&self) -> Result<PathBuf> {
        info!("Attempting installation via direct download...");

        self.report_progress(InstallProgress::Downloading {
            current: 0,
            total: None,
        });

        // 检测平台
        let (platform, arch) = Self::detect_platform();

        if platform == "unknown" || arch == "unknown" {
            return Err(ClaudeError::InternalError(format!(
                "Unsupported platform: {}-{}",
                platform, arch
            )));
        }

        // 构建下载 URL
        // 注意：这里使用通用的 URL 格式，实际可能需要调整
        let filename = if cfg!(windows) {
            format!("claude-{}-{}.exe", platform, arch)
        } else {
            format!("claude-{}-{}", platform, arch)
        };

        let url = format!(
            "https://github.com/anthropics/claude-code/releases/latest/download/{}",
            filename
        );

        info!("Downloading from: {}", url);
        self.report_progress(InstallProgress::Downloading {
            current: 0,
            total: None,
        });

        // 使用 reqwest 下载
        let response = reqwest::get(&url).await.map_err(|e| {
            ClaudeError::InternalError(format!("Failed to download CLI: {}", e))
        })?;

        if !response.status().is_success() {
            return Err(ClaudeError::InternalError(format!(
                "Download failed with status: {}",
                response.status()
            )));
        }

        let total_bytes = response.content_length();
        let bytes = response.bytes().await.map_err(|e| {
            ClaudeError::InternalError(format!("Failed to download bytes: {}", e))
        })?;

        self.report_progress(InstallProgress::Downloading {
            current: bytes.len() as u64,
            total: total_bytes,
        });

        // 确定安装路径
        let install_dir = Self::get_install_dir()?;
        std::fs::create_dir_all(&install_dir).map_err(|e| {
            ClaudeError::InternalError(format!("Failed to create install directory: {}", e))
        })?;

        let exe_name = if cfg!(windows) { "claude.exe" } else { "claude" };
        let install_path = install_dir.join(exe_name);

        // 写入文件
        std::fs::write(&install_path, bytes)
            .map_err(|e| ClaudeError::InternalError(format!("Failed to write CLI: {}", e)))?;

        // 设置可执行权限（Unix）
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&install_path)
                .map_err(|e| ClaudeError::InternalError(format!("Failed to get metadata: {}", e)))?
                .permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&install_path, perms).map_err(|e| {
                ClaudeError::InternalError(format!("Failed to set permissions: {}", e))
            })?;
        }

        info!("✅ Claude CLI installed to: {}", install_path.display());
        Ok(install_path)
    }

    /// 检测平台和架构
    ///
    /// 返回 (platform, arch) 元组
    fn detect_platform() -> (&'static str, &'static str) {
        let platform = if cfg!(target_os = "macos") {
            "darwin"
        } else if cfg!(target_os = "linux") {
            "linux"
        } else if cfg!(target_os = "windows") {
            "windows"
        } else {
            "unknown"
        };

        let arch = if cfg!(target_arch = "x86_64") {
            "x64"
        } else if cfg!(target_arch = "aarch64") {
            "arm64"
        } else if cfg!(target_arch = "x86") {
            "ia32"
        } else {
            "unknown"
        };

        (platform, arch)
    }

    /// 获取用户本地安装目录
    ///
    /// 返回无需权限即可写入的目录
    fn get_install_dir() -> Result<PathBuf> {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .map_err(|_| ClaudeError::InternalError("Cannot determine home directory".to_string()))?;

        let home_path = PathBuf::from(home);

        let dir = if cfg!(windows) {
            // Windows: %USERPROFILE%\AppData\Local\Programs\Claude
            home_path.join("AppData\\Local\\Programs\\Claude")
        } else {
            // Unix: ~/.local/bin
            home_path.join(".local/bin")
        };

        Ok(dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_platform_detection() {
        let (platform, arch) = CliInstaller::detect_platform();
        println!("Detected platform: {}-{}", platform, arch);

        // 验证不是 unknown
        if cfg!(any(
            target_os = "macos",
            target_os = "linux",
            target_os = "windows"
        )) {
            assert_ne!(platform, "unknown");
        }
        if cfg!(any(
            target_arch = "x86_64",
            target_arch = "aarch64",
            target_arch = "x86"
        )) {
            assert_ne!(arch, "unknown");
        }
    }

    #[test]
    fn test_install_dir() {
        let dir = CliInstaller::get_install_dir().unwrap();
        println!("Install directory: {}", dir.display());

        // 验证路径包含用户主目录
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap();
        assert!(dir.starts_with(home));
    }
}
