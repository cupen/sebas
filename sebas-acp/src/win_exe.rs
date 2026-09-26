//! Windows CLI 可执行解析（npm 全局安装形态）。
//!
//! npm 在 Windows 上安装的 CLI（claude 等）是「无扩展名 sh 脚本 + 同名
//! `.cmd`/`.ps1` 包装」——`CreateProcess` 对无扩展名程序只找 `.exe`，把
//! 配置里的裸名字（`path = "claude"`）直接交给 SDK spawn 会得到
//! "program not found"。本模块把配置的名字/路径解析成可直接 spawn 的
//! 形态（`.exe` → `.cmd` → `.bat`）；解析不到时**原样返回**，让下游报出
//! 与平台一致的原始错误，而不是在这里编造失败原因。

/// Windows 可直接 spawn 的扩展名（按 PATHEXT 惯例排序；`.ps1` 不能被
/// `CreateProcess` 直接执行，故意不在列）。
#[cfg(windows)]
const SPAWNABLE_EXTS: [&str; 3] = ["exe", "cmd", "bat"];

/// 解析单条 CLI 命令（argv[0]）。Unix 上恒等返回——扩展名语义只属于
/// Windows。
pub fn resolve_windows_executable(cmd: &str) -> String {
    #[cfg(not(windows))]
    {
        let _ = cmd;
        cmd.to_string()
    }
    #[cfg(windows)]
    resolve_impl(cmd)
}

#[cfg(windows)]
fn resolve_impl(cmd: &str) -> String {
    let p = std::path::Path::new(cmd);
    // 已带可直接 spawn 的扩展名：原样信任（绝对/相对路径皆可）。
    if p.extension()
        .is_some_and(|e| {
            SPAWNABLE_EXTS.contains(&e.to_string_lossy().to_ascii_lowercase().as_str())
        })
    {
        return cmd.to_string();
    }
    // 路径形态（带分隔符）：在自身所在目录找同名可执行。
    if p.parent().is_some_and(|d| !d.as_os_str().is_empty()) {
        return find_beside(p).unwrap_or_else(|| cmd.to_string());
    }
    // 裸名字：扫 PATH（逐目录按 exe → cmd → bat，PATHEXT 惯例）。
    let dirs = std::env::var_os("PATH")
        .map(|v| std::env::split_paths(&v).collect::<Vec<_>>())
        .unwrap_or_default();
    find_in_dirs(cmd, &dirs).unwrap_or_else(|| cmd.to_string())
}

#[cfg(windows)]
fn find_beside(p: &std::path::Path) -> Option<String> {
    let dir = p.parent()?;
    let stem = p.file_stem()?;
    for ext in SPAWNABLE_EXTS {
        let cand = dir.join(format!("{}.{}", stem.to_string_lossy(), ext));
        if cand.is_file() {
            return Some(cand.to_string_lossy().into_owned());
        }
    }
    None
}

#[cfg(windows)]
fn find_in_dirs(name: &str, dirs: &[std::path::PathBuf]) -> Option<String> {
    for dir in dirs {
        for ext in SPAWNABLE_EXTS {
            let cand = dir.join(format!("{name}.{ext}"));
            if cand.is_file() {
                return Some(cand.to_string_lossy().into_owned());
            }
        }
    }
    None
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn spawnable_extension_passes_through() {
        assert_eq!(
            resolve_windows_executable(r"C:\bin\tool.EXE"),
            r"C:\bin\tool.EXE"
        );
    }

    #[test]
    fn extensionless_path_resolves_beside_itself() {
        let dir = tempfile::tempdir().expect("tempdir");
        let shim = dir.path().join("tool.cmd");
        std::fs::write(&shim, b"@echo off").expect("write shim");
        let bare = dir.path().join("tool");
        assert_eq!(
            resolve_windows_executable(bare.to_string_lossy().as_ref()),
            shim.to_string_lossy()
        );
    }

    #[test]
    fn missing_name_stays_as_is_for_honest_downstream_error() {
        let name = "definitely-not-on-path-<impossible>";
        assert_eq!(resolve_windows_executable(name), name);
    }
}
