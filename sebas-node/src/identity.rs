//! 节点身份（add-remote-execution-node D3 / i2：**稳定 node id**）。
//!
//! 身份由操作者在配对时确认（或接受一个生成的默认值）并持久化在节点上：重装后用
//! 同一个 id 重新配对，项目条目仍指得中（项目身份是 `(节点, 路径)`），历史则靠
//! epoch 表达断裂——**身份稳定**与**时间线连续**是两个正交概念，不互相污染
//! （对比 i1「身份 = 一次安装」：那会让每次重装都要在主控侧手工重建项目）。
//!
//! 文件布局（都在状态目录下）：
//! - `node-id`：稳定节点标识（非机密，但状态目录整体按私有处理）；
//! - `credential`：配对后由主控签发的长期凭据（机密，0600；写入属于任务 2.2）。

use crate::error::NodeError;
use std::path::{Path, PathBuf};

/// 节点标识文件名。
pub const NODE_ID_FILE: &str = "node-id";
/// 节点凭据文件名（内容由配对流程写入，见任务 2.2）。
pub const CREDENTIAL_FILE: &str = "credential";
/// 生成的节点标识前缀。
const GENERATED_PREFIX: &str = "node-";
/// 生成标识的随机字节数（转 hex 后长度翻倍）。
const GENERATED_RANDOM_BYTES: usize = 6;

/// 稳定节点标识。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NodeId(String);

impl NodeId {
    /// 解析操作者给出的标识：非空、长度受限、仅 ASCII 字母数字与 `-` `_` `.`。
    ///
    /// 规则本身来自契约 crate（[`sebas_node_link::validate_node_id`]）——主控在握手时
    /// 校验的是同一条规则，避免"节点认为合法、主控认为非法"的假 bug。
    pub fn parse(raw: &str) -> Result<Self, NodeError> {
        sebas_node_link::validate_node_id(raw)
            .map(|id| Self(id.to_string()))
            .map_err(NodeError::config)
    }

    /// 生成一个默认标识：`node-<12 位 hex>`。真正的随机源来自操作系统 CSPRNG。
    pub fn generate() -> Self {
        let mut buf = [0u8; GENERATED_RANDOM_BYTES];
        getrandom::fill(&mut buf).expect("OS CSPRNG unavailable");
        Self(format!("{GENERATED_PREFIX}{}", hex::encode(buf)))
    }

    /// 标识本体。
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// 状态目录里的身份存取。
#[derive(Debug, Clone)]
pub struct IdentityStore {
    dir: PathBuf,
}

impl IdentityStore {
    /// 以状态目录构造。
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// 状态目录。
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// 节点标识文件路径。
    pub fn id_path(&self) -> PathBuf {
        self.dir.join(NODE_ID_FILE)
    }

    /// 凭据文件路径。
    pub fn credential_path(&self) -> PathBuf {
        self.dir.join(CREDENTIAL_FILE)
    }

    /// 读取已存标识；文件不存在或为空 → `None`。
    pub fn load_id(&self) -> Result<Option<NodeId>, NodeError> {
        let path = self.id_path();
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    return Err(NodeError::state_dir(format!(
                        "{} 为空；删除该文件可让节点重新生成标识",
                        path.display()
                    )));
                }
                NodeId::parse(trimmed).map(Some)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(NodeError::state_dir(format!(
                "无法读取 {}：{e}",
                path.display()
            ))),
        }
    }

    /// 确定节点身份：显式给出优先（并落盘采纳），否则复用已存，否则生成一个并落盘。
    pub fn load_or_create_id(&self, explicit: Option<&str>) -> Result<NodeId, NodeError> {
        if let Some(raw) = explicit {
            let id = NodeId::parse(raw)?;
            // 显式给出即操作者的选择：与已存不同时落盘采纳（这是重装接续、
            // 或有意更换身份的唯一入口）。
            if self.load_id()?.as_ref() != Some(&id) {
                self.persist_id(&id)?;
            }
            return Ok(id);
        }
        if let Some(id) = self.load_id()? {
            return Ok(id);
        }
        let id = NodeId::generate();
        self.persist_id(&id)?;
        Ok(id)
    }

    /// 是否已有配对凭据。
    ///
    /// 与 [`Self::load_credential`] **同源判断**：只有空白内容的文件不算已配对——
    /// 否则启动段会说"已配对"，而链路却以"未配对"致命失败，两处说法不一致。
    pub fn has_credential(&self) -> bool {
        matches!(self.load_credential(), Ok(Some(_)))
    }

    /// 读取长期凭据；未配对 → `None`。
    pub fn load_credential(&self) -> Result<Option<String>, NodeError> {
        let path = self.credential_path();
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    return Ok(None);
                }
                Ok(Some(trimmed.to_string()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(NodeError::state_dir(format!(
                "无法读取凭据 {}：{e}",
                path.display()
            ))),
        }
    }

    /// 保存主控签发的长期凭据（原子写 + 0600）。
    ///
    /// 凭据只在配对成功那一刻出现一次，因此必须**先落盘再对外声称已配对**：
    /// 落盘失败就当作配对失败（否则重启后节点会拿着主控不认的凭据重连）。
    pub fn save_credential(&self, secret: &str) -> Result<(), NodeError> {
        ensure_private_dir(&self.dir)?;
        write_private(&self.credential_path(), secret)
    }

    /// 原子写入标识（私有权限）。
    fn persist_id(&self, id: &NodeId) -> Result<(), NodeError> {
        ensure_private_dir(&self.dir)?;
        write_private(&self.id_path(), id.as_str())
    }
}

/// 目录不存在时创建并设为 0700；已存在则**不动它**（不覆盖运维给定的权限）。
fn ensure_private_dir(dir: &Path) -> Result<(), NodeError> {
    if dir.is_dir() {
        return Ok(());
    }
    std::fs::create_dir_all(dir)
        .map_err(|e| NodeError::state_dir(format!("无法创建 {}：{e}", dir.display())))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // 状态目录里有凭据文件，整体按私有处理。失败不致命（磁盘可能不支持）。
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

/// 原子写入：写临时兄弟文件 → 设 0600（unix）→ rename 覆盖目标。
/// 沿用 `sebas` 写 secret 文件的同一 idiom（崩溃不会留下半截文件）。
fn write_private(path: &Path, content: &str) -> Result<(), NodeError> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, content.as_bytes())
        .map_err(|e| NodeError::state_dir(format!("无法写入 {}：{e}", tmp.display())))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600)).map_err(|e| {
            NodeError::state_dir(format!("无法设置 {} 权限：{e}", tmp.display()))
        })?;
    }
    std::fs::rename(&tmp, path)
        .map_err(|e| NodeError::state_dir(format!("无法落盘 {}：{e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, IdentityStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = IdentityStore::new(dir.path().join("state"));
        (dir, store)
    }

    #[test]
    fn generated_id_has_prefix_and_is_hex() {
        let id = NodeId::generate();
        let suffix = id.as_str().strip_prefix(GENERATED_PREFIX).unwrap();
        assert_eq!(suffix.len(), GENERATED_RANDOM_BYTES * 2);
        assert!(suffix.chars().all(|c| c.is_ascii_hexdigit()), "{id}");
        // 两次生成不应相同（碰撞概率 2^-48）。
        assert_ne!(id, NodeId::generate());
    }

    #[test]
    fn parse_accepts_reasonable_ids_and_trims() {
        for raw in ["dev-box", "node_1", "box.example", "a"] {
            assert_eq!(NodeId::parse(raw).unwrap().as_str(), raw);
        }
        assert_eq!(NodeId::parse("  dev-box  ").unwrap().as_str(), "dev-box");
    }

    #[test]
    fn parse_rejects_empty_illegal_and_overlong() {
        assert!(NodeId::parse("").is_err());
        assert!(NodeId::parse("   ").is_err());
        assert!(NodeId::parse("dev box").is_err(), "空格应被拒");
        assert!(NodeId::parse("dev/box").is_err(), "斜杠应被拒");
        assert!(NodeId::parse("节点").is_err(), "非 ASCII 应被拒");
        assert!(NodeId::parse(&"a".repeat(sebas_node_link::MAX_NODE_ID_LEN + 1)).is_err());
        assert!(NodeId::parse(&"a".repeat(sebas_node_link::MAX_NODE_ID_LEN)).is_ok());
    }

    #[test]
    fn id_is_generated_once_and_then_reused() {
        let (_tmp, store) = store();
        assert_eq!(store.load_id().unwrap(), None);

        let first = store.load_or_create_id(None).unwrap();
        assert_eq!(store.load_id().unwrap().as_ref(), Some(&first));
        assert!(store.id_path().exists());

        // 再次调用复用同一个（不会重新生成）。
        let second = store.load_or_create_id(None).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn explicit_id_is_persisted_and_adopted() {
        let (_tmp, store) = store();
        let id = store.load_or_create_id(Some("dev-box")).unwrap();
        assert_eq!(id.as_str(), "dev-box");
        assert_eq!(store.load_id().unwrap().unwrap().as_str(), "dev-box");

        // 显式给出不同的 id：采纳并改写（操作者有意更换/重装接续）。
        let id = store.load_or_create_id(Some("dev-box-2")).unwrap();
        assert_eq!(id.as_str(), "dev-box-2");
        assert_eq!(store.load_id().unwrap().unwrap().as_str(), "dev-box-2");
    }

    #[test]
    fn invalid_explicit_id_is_rejected_without_touching_disk() {
        let (_tmp, store) = store();
        let err = store.load_or_create_id(Some("bad id")).unwrap_err();
        assert!(err.to_string().contains("非法字符"), "{err}");
        assert!(!store.id_path().exists(), "非法输入不应落盘");
    }

    #[test]
    fn empty_id_file_is_reported_not_silently_regenerated() {
        let (_tmp, store) = store();
        std::fs::create_dir_all(store.dir()).unwrap();
        std::fs::write(store.id_path(), "   \n").unwrap();
        let err = store.load_or_create_id(None).unwrap_err();
        assert!(err.to_string().contains("为空"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn id_file_is_0600_and_dir_is_0700() {
        use std::os::unix::fs::PermissionsExt;
        let (_tmp, store) = store();
        store.load_or_create_id(None).unwrap();
        let file_mode = std::fs::metadata(store.id_path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(file_mode, 0o600, "标识文件应私有");
        let dir_mode = std::fs::metadata(store.dir())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700, "状态目录应私有");
    }

    #[test]
    fn credential_presence_is_honest() {
        let (_tmp, store) = store();
        assert!(!store.has_credential(), "未配对时不应报告有凭据");
        std::fs::create_dir_all(store.dir()).unwrap();
        std::fs::write(store.credential_path(), "").unwrap();
        assert!(!store.has_credential(), "空凭据文件不算已配对");
        std::fs::write(store.credential_path(), "some-token").unwrap();
        assert!(store.has_credential());
    }

    #[test]
    fn credential_round_trips_and_is_private() {
        let (_tmp, store) = store();
        assert_eq!(store.load_credential().unwrap(), None, "未配对 → None");
        store.save_credential("s3cret-value").unwrap();
        assert_eq!(
            store.load_credential().unwrap().as_deref(),
            Some("s3cret-value")
        );
        assert!(store.has_credential());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(store.credential_path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600, "凭据必须 0600");
        }
    }

    #[test]
    fn blank_credential_file_reads_as_unpaired() {
        let (_tmp, store) = store();
        std::fs::create_dir_all(store.dir()).unwrap();
        std::fs::write(store.credential_path(), "  \n").unwrap();
        assert_eq!(store.load_credential().unwrap(), None);
        assert!(!store.has_credential());
    }

    #[test]
    fn existing_dir_permissions_are_left_alone() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let tmp = tempfile::tempdir().unwrap();
            let dir = tmp.path().join("state");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
            let store = IdentityStore::new(&dir);
            store.load_or_create_id(None).unwrap();
            let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o755, "已存在的目录权限不应被覆盖");
        }
    }
}
