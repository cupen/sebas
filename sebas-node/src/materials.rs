//! 操作者级材料在节点上的落地（add-remote-execution-node 7.3 / 7.5）。
//!
//! 分工（设计 D9）：**项目级材料**（`AGENTS.md` / `CLAUDE.md` / 项目内 skills）随项目树
//! 本来就在节点上，不过河；**操作者级材料**（全局 skills、跨项目 memory、全局 subagent
//! 定义）由节点在会话创建时向主控拉取，落到「该执行体认得的位置」，并**钉住版本**。
//!
//! 三条不变量：
//!
//! 1. **落盘路径必须安全**：每个路径都按契约 crate 的共享规则校验（拒绝绝对路径与
//!    `..`）——否则远端就拿到了一个"写到自己目录之外"的入口。
//! 2. **版本目录隔离**：每个版本落在 `<root>/<version>/`，先写临时目录再 rename。
//!    同版本重复安装是幂等的（内容相同就不动）。
//! 3. **落点由执行体约定决定**，不由 sebas 统一规定；没有落点约定的执行体**如实拒绝**，
//!    绝不"静默忽略"（那会让操作者以为材料生效了）。

use sebas_node_link::{MaterialFile, validate_material_path};
use std::path::{Path, PathBuf};

/// 材料落地错误。
#[derive(Debug, thiserror::Error)]
pub enum MaterialsError {
    /// 路径不安全（绝对路径 / `..`）。
    #[error("材料路径不安全：{cause}")]
    UnsafePath {
        /// 成因（含那个路径）。
        cause: String,
    },
    /// 落盘失败。
    #[error("材料落盘失败：{cause}")]
    Io {
        /// 成因。
        cause: String,
    },
    /// 该执行体没有材料落点约定。
    #[error("执行体 {execution_body} 没有材料落点约定：{cause}")]
    NoPlacement {
        /// 执行体。
        execution_body: String,
        /// 成因。
        cause: String,
    },
}

/// 一次钉住的材料版本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pinned {
    /// 版本号。
    pub version: String,
    /// 该版本在节点上的根目录。
    pub dir: PathBuf,
}

/// 节点侧的材料仓。
#[derive(Debug, Clone)]
pub struct MaterialStore {
    root: PathBuf,
}

impl MaterialStore {
    /// 以根目录构造（通常 `<state_dir>/materials`）。
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// 根目录。
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 某版本是否已经落盘（同版本重复拉取时直接复用，幂等）。
    pub fn existing(&self, version: &str) -> Option<Pinned> {
        let dir = self.root.join(version);
        if dir.is_dir() {
            Some(Pinned {
                version: version.to_string(),
                dir,
            })
        } else {
            None
        }
    }

    /// 落盘一个版本（先写临时目录再 rename）。
    pub fn install(
        &self,
        version: &str,
        files: &[MaterialFile],
    ) -> Result<Pinned, MaterialsError> {
        if version.trim().is_empty() {
            return Err(MaterialsError::Io {
                cause: "材料版本号不能为空".into(),
            });
        }
        // 全部路径先校验完再动手：写一半才发现路径非法会留下半份材料。
        for file in files {
            validate_material_path(&file.path).map_err(|cause| MaterialsError::UnsafePath {
                cause: format!("{}: {cause}", file.path),
            })?;
        }

        let target = self.root.join(version);
        if target.is_dir() {
            return Ok(Pinned {
                version: version.to_string(),
                dir: target,
            });
        }

        let staging = self.root.join(format!(".staging-{version}-{}", std::process::id()));
        if staging.exists() {
            let _ = std::fs::remove_dir_all(&staging);
        }
        std::fs::create_dir_all(&staging).map_err(|e| MaterialsError::Io {
            cause: format!("无法创建 {}：{e}", staging.display()),
        })?;

        for file in files {
            let rel = validate_material_path(&file.path)
                .map_err(|cause| MaterialsError::UnsafePath { cause })?;
            let path = staging.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| MaterialsError::Io {
                    cause: format!("无法创建 {}：{e}", parent.display()),
                })?;
            }
            std::fs::write(&path, file.content.as_bytes()).map_err(|e| MaterialsError::Io {
                cause: format!("无法写入 {}：{e}", path.display()),
            })?;
        }

        std::fs::create_dir_all(&self.root).map_err(|e| MaterialsError::Io {
            cause: format!("无法创建 {}：{e}", self.root.display()),
        })?;
        std::fs::rename(&staging, &target).map_err(|e| MaterialsError::Io {
            cause: format!("无法落盘 {}：{e}", target.display()),
        })?;
        Ok(Pinned {
            version: version.to_string(),
            dir: target,
        })
    }

    /// 把材料**放到**该执行体读的位置，并返回那个目录。
    ///
    /// 注意这是**动作**而不是纯查询：版本目录只是材料的来源，执行体读的是它自己的
    /// 约定目录（`<version>/<body>/`）。只算路径不落地，执行体那边就是空的——
    /// 那正是「看起来配好了、其实没生效」的坑。幂等：已放置过就直接复用。
    ///
    /// 落点由**执行体自己的约定**决定，sebas 不能替它选。ACP 类执行体（Claude Code 等）
    /// 的目录约定尚未接入 → **如实拒绝**，而不是把材料丢在一个它永远不会读的目录里。
    pub fn place_for(
        &self,
        execution_body: &str,
        pinned: &Pinned,
    ) -> Result<PathBuf, MaterialsError> {
        let target = match execution_body {
            "echo" | "native" => pinned.dir.join(execution_body),
            other => {
                return Err(MaterialsError::NoPlacement {
                    execution_body: other.to_string(),
                    cause: format!(
                        "该执行体（{other}）的材料落点约定尚未接入；\
                         在接入之前不落材料，以免操作者以为它生效了"
                    ),
                });
            }
        };
        if target.is_dir() {
            return Ok(target);
        }
        copy_tree(&pinned.dir, &target, Some(execution_body))?;
        Ok(target)
    }
}

/// 递归复制目录树；`skip_top` 跳过执行体专属子目录（否则会把自己复制进自己）。
fn copy_tree(src: &Path, dst: &Path, skip_top: Option<&str>) -> Result<(), MaterialsError> {
    std::fs::create_dir_all(dst).map_err(|e| MaterialsError::Io {
        cause: format!("无法创建 {}：{e}", dst.display()),
    })?;
    let entries = std::fs::read_dir(src).map_err(|e| MaterialsError::Io {
        cause: format!("无法读取 {}：{e}", src.display()),
    })?;
    for entry in entries.filter_map(|e| e.ok()) {
        let name = entry.file_name();
        if skip_top.is_some_and(|skip| name.to_string_lossy() == skip) {
            continue;
        }
        let from = entry.path();
        let to = dst.join(&name);
        if from.is_dir() {
            copy_tree(&from, &to, None)?;
        } else {
            std::fs::copy(&from, &to).map_err(|e| MaterialsError::Io {
                cause: format!("无法复制 {} → {}：{e}", from.display(), to.display()),
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, content: &str) -> MaterialFile {
        MaterialFile {
            path: path.into(),
            content: content.into(),
        }
    }

    #[test]
    fn install_writes_files_under_the_version_directory() {
        let dir = tempfile::tempdir().unwrap();
        let store = MaterialStore::new(dir.path().join("materials"));
        let files = vec![
            file("skills/beads/SKILL.md", "# beads"),
            file("memory/notes.md", "hi"),
        ];
        let pinned = store.install("v1", &files).unwrap();

        assert_eq!(pinned.version, "v1");
        assert_eq!(
            std::fs::read_to_string(pinned.dir.join("skills/beads/SKILL.md")).unwrap(),
            "# beads"
        );
        assert_eq!(
            std::fs::read_to_string(pinned.dir.join("memory/notes.md")).unwrap(),
            "hi"
        );
        // 临时目录已收走（不会留下 .staging-*）。
        let leftovers: Vec<_> = std::fs::read_dir(store.root())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(".staging"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn install_is_idempotent_for_the_same_version() {
        let dir = tempfile::tempdir().unwrap();
        let store = MaterialStore::new(dir.path());
        let first = store.install("v1", &[file("a.md", "one")]).unwrap();
        // 同版本再装：直接复用，不重写（内容保持第一次的）。
        let second = store.install("v1", &[file("a.md", "two")]).unwrap();
        assert_eq!(first.dir, second.dir);
        assert_eq!(
            std::fs::read_to_string(second.dir.join("a.md")).unwrap(),
            "one"
        );
        assert!(store.existing("v1").is_some());
        assert!(store.existing("v2").is_none());
    }

    #[test]
    fn install_rejects_unsafe_paths_before_writing_anything() {
        let dir = tempfile::tempdir().unwrap();
        let store = MaterialStore::new(dir.path());
        for bad in ["/etc/passwd", "../escape.md", "a/../../b"] {
            let err = store
                .install("v1", &[file("ok.md", "x"), file(bad, "evil")])
                .unwrap_err();
            assert!(matches!(err, MaterialsError::UnsafePath { .. }), "{bad}");
            // 关键：**一个文件都没写**。
            assert!(
                store.existing("v1").is_none(),
                "非法路径必须在动手前被拦住（{bad}）"
            );
            assert!(
                assert_no_escape(dir.path()),
                "不得在材料根之外留下文件（{bad}）"
            );
        }
    }

    fn assert_no_escape(root: &Path) -> bool {
        // 材料根之外（父目录）不得出现我们写的东西。
        std::fs::read_dir(root)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .all(|e| !e.file_name().to_string_lossy().starts_with(".staging"))
            })
            .unwrap_or(true)
    }

    #[test]
    fn two_different_versions_coexist() {
        let dir = tempfile::tempdir().unwrap();
        let store = MaterialStore::new(dir.path());
        let v1 = store.install("v1", &[file("a.md", "one")]).unwrap();
        let v2 = store.install("v2", &[file("a.md", "two")]).unwrap();
        assert_ne!(v1.dir, v2.dir, "版本目录隔离：旧会话继续读旧版本");
        assert_eq!(std::fs::read_to_string(v1.dir.join("a.md")).unwrap(), "one");
        assert_eq!(std::fs::read_to_string(v2.dir.join("a.md")).unwrap(), "two");
    }

    #[test]
    fn placement_materialises_the_files_where_the_body_reads_them() {
        let dir = tempfile::tempdir().unwrap();
        let store = MaterialStore::new(dir.path());
        let pinned = store
            .install("v1", &[file("skills/a.md", "A"), file("top.md", "T")])
            .unwrap();

        // 落点是**动作**：调用之后执行体目录里真的有文件（只算路径会留一个空目录）。
        let echo = store.place_for("echo", &pinned).unwrap();
        let native = store.place_for("native", &pinned).unwrap();
        assert_eq!(echo, pinned.dir.join("echo"));
        assert_eq!(native, pinned.dir.join("native"));
        assert_ne!(echo, native, "两个执行体不共用落点");
        assert_eq!(
            std::fs::read_to_string(echo.join("skills/a.md")).unwrap(),
            "A"
        );
        assert_eq!(std::fs::read_to_string(echo.join("top.md")).unwrap(), "T");
        assert_eq!(
            std::fs::read_to_string(native.join("skills/a.md")).unwrap(),
            "A"
        );
        // 幂等，且不自嵌套。
        assert_eq!(store.place_for("echo", &pinned).unwrap(), echo);
        assert!(!echo.join("echo").exists());

        // ACP 类执行体：落点约定未接入 → 如实拒绝（绝不静默忽略）。
        let err = store.place_for("claude", &pinned).unwrap_err();
        match err {
            MaterialsError::NoPlacement {
                execution_body,
                cause,
            } => {
                assert_eq!(execution_body, "claude");
                assert!(cause.contains("尚未接入"), "{cause}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_empty_version_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let store = MaterialStore::new(dir.path());
        assert!(store.install("  ", &[file("a.md", "x")]).is_err());
    }
}
