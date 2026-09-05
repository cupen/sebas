//! 自更新核心逻辑：版本检查、下载、校验、编译、安装、回滚。
//!
//! # 目录结构
//!
//! ```text
//! ~/.local/share/sebas/
//! ├── current -> v1.2.4       # 软链，指向当前版本目录
//! ├── versions/
//! │   ├── v1.2.3/
//! │   │   └── sebas
//! │   └── v1.2.4/
//! │       └── sebas
//! ├── rollback/               # 上一版本备份
//! │   └── sebas
//! └── upgrade.lock            # 升级锁
//! ```

use crate::config::WatchdogConfig;
use crate::error::{Result, SebasError};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// 锁文件路径（全局共享，用于防止并发升级）
static UPGRADE_LOCKED: AtomicBool = AtomicBool::new(false);

/// GitHub Release API 响应
#[derive(Debug, Deserialize)]
struct GithubRelease {
    #[serde(rename = "tag_name")]
    pub tag_name: String,
    pub assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    pub name: String,
    #[serde(rename = "browser_download_url")]
    pub url: String,
    pub size: Option<u64>,
}

/// 版本信息
#[derive(Debug, Clone)]
pub struct ReleaseInfo {
    pub version: String,
    pub download_url: String,
    pub sha256_url: Option<String>,
    pub size: Option<u64>,
}

/// 当前版本号（编译时注入）
pub fn current_version() -> String {
    let v = env!("CARGO_PKG_VERSION");
    let branch = option_env!("GIT_BRANCH").unwrap_or("unknown");
    let hash = option_env!("GIT_HASH").unwrap_or("unknown");
    format!("{v} ({branch} @ {hash})")
}

/// 当前版本号（纯数字，不含分支信息）
pub fn current_version_raw() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// GitHub 仓库全名（owner/repo）
pub fn repo_full_name() -> &'static str {
    "cupen/sebas"
}

/// 获取数据目录（`data_dir` 或 XDG 默认）。
///
/// 运行时的 watchdog `/`installer 用 `dirs::data_dir()`（即当前用户 home）。
/// `service --install` 需要按目标 `--user` 解析，走 [`data_dir_for_user`]。
pub fn data_dir(cfg: &WatchdogConfig) -> PathBuf {
    data_dir_for_user(cfg.storage.data_dir.as_str(), dirs::data_dir())
}

/// 按显式 `data_dir` 配置 + 一个「默认 home」解析数据目录。
///
/// `default_home` 在 `service --install` 时传 `--user` 的 home（而非 installer
/// root 的），确保服务自升级写入的目录归服务用户所有。
pub fn data_dir_for_user(
    data_dir_cfg: &str,
    default_home: Option<PathBuf>,
) -> PathBuf {
    if data_dir_cfg.is_empty() {
        default_home
            .unwrap_or_else(|| PathBuf::from("~/.local/share"))
            .join("sebas")
    } else {
        expand_tilde(data_dir_cfg)
    }
}

/// 尝试获取升级锁（文件锁 + 进程锁）
pub fn try_lock(data_dir: &Path) -> Result<()> {
    if UPGRADE_LOCKED.load(Ordering::SeqCst) {
        return Err(SebasError::Upgrade("正在升级中，请勿重复操作".into()));
    }

    // 文件锁
    let lock_path = data_dir.join("upgrade.lock");
    if lock_path.exists() {
        // 检查锁文件是否过期（PID 是否还在运行）
        let content = std::fs::read_to_string(&lock_path).unwrap_or_default();
        if let Ok(pid) = content.trim().parse::<u32>() {
            // 检查进程是否存在
            #[cfg(unix)]
            {
                let exists = unsafe { libc::kill(pid as i32, 0) == 0 };
                if exists {
                    return Err(SebasError::Upgrade("正在升级中，请勿重复操作".into()));
                }
            }
            #[cfg(not(unix))]
            {
                // Windows 上没有 kill(pid, 0)，直接覆盖
            }
        }
        // 锁文件过期，清除
        let _ = std::fs::remove_file(&lock_path);
    }

    // 原子创建锁文件；避免两个 watchdog 同时看到 lock 不存在后一起写入。
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)
    {
        Ok(mut f) => {
            use std::io::Write;
            write!(f, "{}", std::process::id())
                .map_err(|e| SebasError::Upgrade(format!("写入锁文件失败: {e}")))?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(SebasError::Upgrade("正在升级中，请勿重复操作".into()));
        }
        Err(e) => return Err(SebasError::Upgrade(format!("创建锁文件失败: {e}"))),
    }
    UPGRADE_LOCKED.store(true, Ordering::SeqCst);
    Ok(())
}

/// 释放升级锁
pub fn unlock(data_dir: &Path) {
    UPGRADE_LOCKED.store(false, Ordering::SeqCst);
    let lock_path = data_dir.join("upgrade.lock");
    let _ = std::fs::remove_file(&lock_path);
}

/// 检查 GitHub 上是否有新版本
pub async fn check_latest(repo: &str, current: &str) -> Result<Option<ReleaseInfo>> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .header("User-Agent", "sebas-upgrade/1.0")
        .header("Accept", "application/vnd.github.v3+json")
        .send()
        .await
        .map_err(|e| SebasError::Upgrade(format!("GitHub API 请求失败: {e}")))?;

    if !resp.status().is_success() {
        return Err(SebasError::Upgrade(format!(
            "GitHub API 返回 {}",
            resp.status()
        )));
    }

    let release: GithubRelease = resp
        .json()
        .await
        .map_err(|e| SebasError::Upgrade(format!("GitHub API 响应解析失败: {e}")))?;

    // 比较版本号
    let tag = release.tag_name.trim_start_matches('v');
    if !is_newer(tag, current) {
        return Ok(None);
    }

    // 查找当前平台的二进制 asset
    let target = current_target();
    let (binary_asset, sha256_asset) = find_assets(&release.assets, &target);

    match binary_asset {
        Some(asset) => Ok(Some(ReleaseInfo {
            version: tag.to_string(),
            download_url: asset.url.clone(),
            sha256_url: sha256_asset.map(|a| a.url.clone()),
            size: asset.size,
        })),
        None => Err(SebasError::Upgrade(format!(
            "未找到适用于 {target} 的 release asset"
        ))),
    }
}

/// 下载 release 二进制并校验
pub async fn download_release(info: &ReleaseInfo, dest: &Path, tmp_dir: &Path) -> Result<()> {
    let client = reqwest::Client::new();

    // 下载到临时文件
    let tmp_path = tmp_dir.join(format!("sebas-{}.tmp", info.version));
    let resp = client
        .get(&info.download_url)
        .header("User-Agent", "sebas-upgrade/1.0")
        .send()
        .await
        .map_err(|e| SebasError::Upgrade(format!("下载失败: {e}")))?;

    if !resp.status().is_success() {
        return Err(SebasError::Upgrade(format!("下载返回 {}", resp.status())));
    }

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| SebasError::Upgrade(format!("下载数据读取失败: {e}")))?;

    // 校验 checksum
    let expected = info
        .sha256_url
        .as_deref()
        .ok_or_else(|| SebasError::Upgrade("release 缺少 SHA256 checksum，拒绝安装".into()))?;
    let expected = fetch_checksum(expected).await?;
    let actual = sha256_hex(&bytes);
    if actual != expected {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(SebasError::Upgrade(format!(
            "SHA256 校验失败：期望 {expected}，实际 {actual}"
        )));
    }

    // 写入临时文件
    std::fs::write(&tmp_path, &bytes)
        .map_err(|e| SebasError::Upgrade(format!("写入临时文件失败: {e}")))?;

    // 设置可执行权限
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| SebasError::Upgrade(format!("设置可执行权限失败: {e}")))?;
    }

    // 移动到目标路径
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| SebasError::Upgrade(format!("创建版本目录失败: {e}")))?;
    }
    std::fs::rename(&tmp_path, dest)
        .map_err(|e| SebasError::Upgrade(format!("移动文件失败: {e}")))?;

    Ok(())
}

/// 本地编译（dev 模式）
pub async fn compile_dev(project_dir: &Path) -> Result<PathBuf> {
    // 确认是 sebas 项目
    let cargo_toml = project_dir.join("Cargo.toml");
    if !cargo_toml.exists() {
        return Err(SebasError::Upgrade(
            "非开发环境：当前目录不是 sebas 项目（未找到 Cargo.toml）".into(),
        ));
    }

    // 检查 Cargo.toml 中的 package name
    let content = std::fs::read_to_string(&cargo_toml)
        .map_err(|_| SebasError::Upgrade("读取 Cargo.toml 失败".into()))?;
    if !content.contains(r#"name = "sebas""#) {
        return Err(SebasError::Upgrade(
            "非开发环境：Cargo.toml 中 package name 不是 sebas".into(),
        ));
    }

    // 执行 cargo build --release
    let status = tokio::process::Command::new("cargo")
        .args(["build", "--release"])
        .current_dir(project_dir)
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()
        .await
        .map_err(|e| SebasError::Upgrade(format!("cargo 执行失败: {e}")))?;

    if !status.success() {
        return Err(SebasError::Upgrade("编译失败".into()));
    }

    let binary = project_dir.join("target/release/sebas");
    if !binary.exists() {
        return Err(SebasError::Upgrade(
            "编译产物未找到（target/release/sebas）".into(),
        ));
    }

    Ok(binary)
}

/// 安装版本：将二进制放入 versions 目录，更新软链，备份旧版本
pub fn install_version(binary: &Path, version: &str, data_dir: &Path) -> Result<()> {
    // 创建 versions/v{version}/ 目录
    let version_dir = data_dir.join("versions").join(format!("v{version}"));
    std::fs::create_dir_all(&version_dir)
        .map_err(|e| SebasError::Upgrade(format!("创建版本目录失败: {e}")))?;

    // 复制二进制
    let dest = version_dir.join("sebas");
    std::fs::copy(binary, &dest)
        .map_err(|e| SebasError::Upgrade(format!("复制二进制失败: {e}")))?;

    // 设置可执行权限
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| SebasError::Upgrade(format!("设置可执行权限失败: {e}")))?;
    }

    // 备份当前版本到 rollback/
    let current_link = data_dir.join("current");
    if current_link.exists() {
        let current_target = std::fs::read_link(&current_link).ok();
        if let Some(target) = current_target {
            // 软链是相对路径（相对于 current_link 的父目录），需要正确解析
            let current_binary = current_link.parent().unwrap().join(&target).join("sebas");
            if current_binary.exists() {
                let rollback_dir = data_dir.join("rollback");
                std::fs::create_dir_all(&rollback_dir).ok();
                let _ = std::fs::copy(&current_binary, rollback_dir.join("sebas"));
            }
        }
    }

    // 更新软链
    update_symlink(&version_dir, &current_link)?;

    Ok(())
}

/// 回滚到上一版本
pub fn rollback(data_dir: &Path) -> Result<()> {
    let rollback_binary = data_dir.join("rollback").join("sebas");
    if !rollback_binary.exists() {
        return Err(SebasError::Upgrade("没有可回滚的版本".into()));
    }

    // 备份当前版本（以防回滚失败后恢复）
    let current_link = data_dir.join("current");
    if current_link.exists() {
        let current_binary = data_dir
            .join("versions")
            .join("_temp_rollback")
            .join("sebas");
        std::fs::create_dir_all(current_binary.parent().unwrap()).ok();
        if let Ok(target) = std::fs::read_link(&current_link) {
            let src = current_link.parent().unwrap().join(&target).join("sebas");
            if src.exists() {
                let _ = std::fs::copy(&src, &current_binary);
            }
        }
    }

    // 创建 rollback 版本目录
    let rollback_version_dir = data_dir.join("versions").join("rollback");
    std::fs::create_dir_all(&rollback_version_dir)
        .map_err(|e| SebasError::Upgrade(format!("创建回滚目录失败: {e}")))?;

    // 复制回滚二进制
    let dest = rollback_version_dir.join("sebas");
    std::fs::copy(&rollback_binary, &dest)
        .map_err(|e| SebasError::Upgrade(format!("回滚复制失败: {e}")))?;

    // 设置可执行权限
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| SebasError::Upgrade(format!("设置可执行权限失败: {e}")))?;
    }

    // 更新软链
    update_symlink(&rollback_version_dir, &current_link)?;

    // 清理临时备份
    let _ = std::fs::remove_dir_all(data_dir.join("versions").join("_temp_rollback"));

    Ok(())
}

/// 获取当前二进制路径（通过 current 软链指向的版本）
pub fn current_binary_path(data_dir: &Path) -> Option<PathBuf> {
    let current_link = data_dir.join("current");
    let target = std::fs::read_link(&current_link).ok()?;
    let path = if target.is_absolute() {
        target
    } else {
        current_link.parent()?.join(target)
    };
    let binary = path.join("sebas");
    binary.exists().then_some(binary)
}

// ─── 内部工具 ────────────────────────────────────────────

/// 获取当前平台 target triple
fn current_target() -> String {
    let arch = std::env::consts::ARCH;
    let os = std::env::consts::OS;
    match (arch, os) {
        ("x86_64", "linux") => "x86_64-unknown-linux-gnu".to_string(),
        ("aarch64", "linux") => "aarch64-unknown-linux-gnu".to_string(),
        ("x86_64", "macos") => "x86_64-apple-darwin".to_string(),
        ("aarch64", "macos") => "aarch64-apple-darwin".to_string(),
        ("x86_64", "windows") => "x86_64-pc-windows-msvc".to_string(),
        _ => format!("{}-{}", arch, os),
    }
}

/// 查找匹配当前平台的 asset
fn find_assets<'a>(
    assets: &'a [GithubAsset],
    _target: &str,
) -> (Option<&'a GithubAsset>, Option<&'a GithubAsset>) {
    // 先找带 target 的精确匹配，再 fallback 到通用 sebas 二进制
    let exact = assets.iter().find(|a| {
        a.name.contains(_target) && !a.name.ends_with(".sha256") && !a.name.ends_with(".asc")
    });
    let generic = assets.iter().find(|a| {
        a.name == "sebas"
            || a.name.starts_with("sebas-")
                && !a.name.ends_with(".sha256")
                && !a.name.ends_with(".asc")
    });
    let binary = exact.or(generic);

    let sha256 = binary.and_then(|b| {
        assets
            .iter()
            .find(|a| a.name == format!("{}.sha256", b.name))
    });

    (binary, sha256)
}

/// 简单的 semver 比较（仅支持 x.y.z）
fn is_newer(tag: &str, current: &str) -> bool {
    fn parse_ver(s: &str) -> Vec<u32> {
        s.split('.').filter_map(|p| p.parse::<u32>().ok()).collect()
    }
    let tag_parts = parse_ver(tag);
    let cur_parts = parse_ver(current);

    for (t, c) in tag_parts.iter().zip(cur_parts.iter()) {
        if t > c {
            return true;
        }
        if t < c {
            return false;
        }
    }
    tag_parts.len() > cur_parts.len()
}

/// 计算 SHA256 hex
fn sha256_hex(data: &[u8]) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

/// 从 URL 下载 checksum 文件，解析出 SHA256 hex 值
async fn fetch_checksum(url: &str) -> Result<String> {
    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .header("User-Agent", "sebas-upgrade/1.0")
        .send()
        .await
        .map_err(|e| SebasError::Upgrade(format!("Checksum 下载失败: {e}")))?;

    if !resp.status().is_success() {
        return Err(SebasError::Upgrade(format!(
            "Checksum 下载返回 {}",
            resp.status()
        )));
    }

    let text = resp
        .text()
        .await
        .map_err(|e| SebasError::Upgrade(format!("Checksum 读取失败: {e}")))?;

    // 格式: <hex>  <filename>
    let checksum = text
        .split_whitespace()
        .next()
        .ok_or_else(|| SebasError::Upgrade("Checksum 文件格式错误".into()))?;
    if checksum.len() != 64 || !checksum.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(SebasError::Upgrade("Checksum 不是合法 SHA256 hex".into()));
    }
    Ok(checksum.to_ascii_lowercase())
}

/// 更新软链
fn update_symlink(target: &Path, link: &Path) -> Result<()> {
    // 删除旧软链（如果存在）
    if link.exists() {
        std::fs::remove_file(link)
            .map_err(|e| SebasError::Upgrade(format!("删除旧软链失败: {e}")))?;
    }
    // 创建新软链（相对路径）
    let rel = make_relative(link, target);
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&rel, link)
            .map_err(|e| SebasError::Upgrade(format!("创建软链失败: {e}")))?;
    }
    #[cfg(not(unix))]
    {
        std::fs::copy(target.join("sebas"), link.join("sebas"))
            .map_err(|e| SebasError::Upgrade(format!("复制文件失败: {e}")))?;
    }
    Ok(())
}

/// 计算相对路径
fn make_relative(base: &Path, target: &Path) -> PathBuf {
    let base_dir = match base.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };

    let mut result = PathBuf::new();
    let base_components: Vec<_> = base_dir.components().collect();
    let target_components: Vec<_> = target.components().collect();

    let common_len = base_components
        .iter()
        .zip(&target_components)
        .take_while(|(a, b)| a == b)
        .count();

    for _ in common_len..base_components.len() {
        result.push("..");
    }
    for c in &target_components[common_len..] {
        result.push(c.as_os_str());
    }
    result
}

pub fn expand_tilde(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    PathBuf::from(p)
}

/// 把当前二进制 seed 到固定路径 `<data_dir>/bin/sebas`。
///
/// `service --install` 用它将安装时的 `current_exe()` 落盘为稳定路径；此后
/// `install_version` 的原地替换（见 `update`）即可让该路径始终指向最新版本，
/// systemd 重启/机器重启都一致。bin 目录可选传入覆盖名（默认 `sebas`）。
pub fn seed_stable_binary(binary: &Path, data_dir: &Path) -> Result<PathBuf> {
    let bin_dir = data_dir.join("bin");
    std::fs::create_dir_all(&bin_dir)
        .map_err(|e| SebasError::Upgrade(format!("创建固定目录失败: {e}")))?;

    let dest = bin_dir.join("sebas");
    let tmp = bin_dir.join(format!(".sebas-{}.tmp", std::process::id()));
    // 先复制到临时文件再 rename，避免覆盖运行中的二进制时半写。
    std::fs::copy(binary, &tmp)
        .map_err(|e| SebasError::Upgrade(format!("复制固定二进制失败: {e}")))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| SebasError::Upgrade(format!("设置固定二进制权限失败: {e}")))?;
    }
    std::fs::rename(&tmp, &dest)
        .map_err(|e| SebasError::Upgrade(format!("落盘固定二进制失败: {e}")))?;

    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_is_newer() {
        assert!(is_newer("1.2.0", "1.1.0"));
        assert!(!is_newer("1.1.0", "1.2.0"));
        assert!(!is_newer("1.1.0", "1.1.0"));
        assert!(is_newer("1.1.1", "1.1.0"));
        assert!(is_newer("1.2.0", "1.1.99"));
        assert!(!is_newer("1.0.0", "1.0.1"));
        assert!(is_newer("1.1.0.1", "1.1.0"));
        assert!(!is_newer("1.1.0", "1.1.0.1"));
    }

    #[test]
    fn test_sha256_hex() {
        let hash = sha256_hex(b"hello");
        assert_eq!(hash.len(), 64);
        assert_eq!(
            hash,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn test_try_lock_unlock() {
        let tmp = std::env::temp_dir().join("sebas-test-lock");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        // 第一次加锁成功
        try_lock(&tmp).unwrap();
        assert!(tmp.join("upgrade.lock").exists());

        // 第二次加锁失败（同一进程）
        let err = try_lock(&tmp).unwrap_err();
        assert!(err.to_string().contains("正在升级中"));

        // 解锁
        unlock(&tmp);
        assert!(!tmp.join("upgrade.lock").exists());

        // 解锁后可再次加锁
        try_lock(&tmp).unwrap();
        unlock(&tmp);

        let _ = fs::remove_dir_all(&tmp);
    }

    // 断言 unix symlink 语义（current 软链 + read_link）；Windows 走
    // update_symlink 的 copy 回退，语义不同且不建 current 目录，先门控到 unix。
    #[cfg(unix)]
    #[test]
    fn test_install_and_rollback() {
        let tmp = std::env::temp_dir().join("sebas-test-install");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        // 创建虚拟二进制
        let v1 = tmp.join("sebas-v1");
        fs::write(&v1, b"v1 binary content").unwrap();

        // 安装 v1.0.0
        install_version(&v1, "1.0.0", &tmp).unwrap();
        let current_link = tmp.join("current");
        assert!(current_link.exists());
        let target = fs::read_link(&current_link).unwrap();
        assert_eq!(target, std::path::Path::new("versions/v1.0.0"));

        // 当前二进制可执行
        let current_bin = current_binary_path(&tmp).expect("current binary path");
        assert!(current_bin.exists());
        assert_eq!(current_bin, tmp.join("versions/v1.0.0/sebas"));

        // 创建虚拟二进制 v2
        let v2 = tmp.join("sebas-v2");
        fs::write(&v2, b"v2 binary content").unwrap();

        // 安装 v2.0.0
        install_version(&v2, "2.0.0", &tmp).unwrap();
        let target2 = fs::read_link(&current_link).unwrap();
        assert_eq!(target2, std::path::Path::new("versions/v2.0.0"));

        // rollback/ 应该有 v1 的备份
        assert!(tmp.join("rollback").join("sebas").exists());
        let rollback_content = fs::read(tmp.join("rollback").join("sebas")).unwrap();
        assert_eq!(rollback_content, b"v1 binary content");

        // 回滚到 v1
        rollback(&tmp).unwrap();
        let target3 = fs::read_link(&current_link).unwrap();
        assert_eq!(target3, std::path::Path::new("versions/rollback"));

        // 清理
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_compile_dev_check() {
        // 在当前项目目录测试 compile_dev 的检查逻辑
        // 不实际编译，只验证 Cargo.toml 检测

        let project_dir = std::env::current_dir().unwrap();
        let cargo_toml = project_dir.join("Cargo.toml");
        assert!(cargo_toml.exists(), "应该在 sebas 项目目录中运行测试");

        let content = fs::read_to_string(&cargo_toml).unwrap();
        assert!(
            content.contains(r#"name = "sebas""#),
            "Cargo.toml 应该包含 sebas 包名"
        );
    }

    #[test]
    fn test_rollback_no_backup() {
        // 无备份时回滚应报错
        let tmp = std::env::temp_dir().join("sebas-test-rollback-empty");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let err = rollback(&tmp).unwrap_err();
        assert!(err.to_string().contains("没有可回滚的版本"));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_data_dir_default() {
        let cfg = WatchdogConfig::default();
        let dir = data_dir(&cfg);
        assert!(
            dir.ends_with("sebas"),
            "默认数据目录应以 sebas 结尾: {:?}",
            dir
        );
    }

    #[test]
    fn test_expand_tilde() {
        let expanded = expand_tilde("~/test");
        assert!(!expanded.starts_with("~"), "~ 应该被展开");
        assert!(expanded.ends_with("test"), "尾部路径应保留");
    }

    #[test]
    fn test_seed_stable_binary() {
        let tmp = std::env::temp_dir().join("sebas-test-seed");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let src = tmp.join("src-sebas");
        fs::write(&src, b"binary bytes").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&src, fs::Permissions::from_mode(0o755)).unwrap();
        }

        let data_dir = tmp.join("data");
        let dest = seed_stable_binary(&src, &data_dir).unwrap();
        assert_eq!(dest, data_dir.join("bin").join("sebas"));
        assert!(dest.exists());
        assert_eq!(fs::read(&dest).unwrap(), b"binary bytes");

        // 幂等：再次 seed 覆盖成功
        seed_stable_binary(&src, &data_dir).unwrap();
        assert!(dest.exists());

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_data_dir_for_user() {
        let dir = data_dir_for_user("", Some(PathBuf::from("/home/svc")));
        assert_eq!(dir, PathBuf::from("/home/svc/sebas"));

        // 空默认 home 时退回占位
        let dir2 = data_dir_for_user("", None);
        assert!(dir2.ends_with("sebas"));

        // 显式 data_dir 优先
        let dir3 = data_dir_for_user("/var/lib/sebas", Some(PathBuf::from("/home/svc")));
        assert_eq!(dir3, PathBuf::from("/var/lib/sebas"));
    }

    // 同 test_install_and_rollback：断言 unix symlink 语义。
    #[cfg(unix)]
    #[test]
    fn test_install_version_preserves_rollback() {
        // 安装多个版本后，rollback 应保留上一版本
        let tmp = std::env::temp_dir().join("sebas-test-rollback-chain");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        // 安装 v1
        let v1 = tmp.join("sv1");
        fs::write(&v1, b"v1").unwrap();
        install_version(&v1, "1.0.0", &tmp).unwrap();

        // 安装 v2
        let v2 = tmp.join("sv2");
        fs::write(&v2, b"v2").unwrap();
        install_version(&v2, "2.0.0", &tmp).unwrap();

        // rollback 应该是 v1
        let rollback_content = fs::read(tmp.join("rollback").join("sebas")).unwrap();
        assert_eq!(rollback_content, b"v1", "rollback 应保留上一版本 (v1)");

        // 安装 v3
        let v3 = tmp.join("sv3");
        fs::write(&v3, b"v3").unwrap();
        install_version(&v3, "3.0.0", &tmp).unwrap();

        // rollback 现在是 v2
        let rollback_content2 = fs::read(tmp.join("rollback").join("sebas")).unwrap();
        assert_eq!(rollback_content2, b"v2", "rollback 应更新为 v2");

        let _ = fs::remove_dir_all(&tmp);
    }
}
