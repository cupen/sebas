//! 节点注册表与配对令牌（主控侧持久状态）。
//!
//! ## 为什么单独一个文件
//!
//! 与项目注册表同一取舍（`agent-workbench`「Project registry persistence is WebUI-owned」）：
//! 这份状态有自己的写者与生命周期，塞进别的写者会互相覆盖的文件（如 core 全量重写的
//! router state 文件）会丢更新。因此它持久化到**自己的 JSON 文件**，由持有它的进程独占写。
//!
//! ## 配对的语义（设计 D3 / i2）
//!
//! - **join token**：一次性、带过期。签发时返回原始 token（只出现这一次），文件里只留
//!   哈希。已消费/已过期都不能再用。
//! - **凭据**：配对成功后签发长期凭据，同样只留哈希。
//! - **同 id 再配对**：离线节点用同一个 id 重新配对**允许**（这是「重装后用同一 id
//!   接续」的实现，凭据轮换、旧凭据立即失效）；已在线或已吊销的 id 再配对**拒绝**
//!   ——前者防两台机器冒用同一身份，后者保证吊销不可被配对绕过。
//!
//! ## 损坏容忍
//!
//! 与 `session-lifecycle`「Restart recovery with corruption tolerance」同款口径：
//! 文件缺失 → 空表；文件损坏 → 隔离改名并以空表启动，**绝不因此拒绝启动**。

use crate::node_link::Rejection;
use sebas_node_link::RejectCode;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 注册表读写失败（I/O、落盘）。
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    /// 读写注册表文件失败。
    #[error("节点注册表不可用：{cause}")]
    Io {
        /// 成因（含路径）。
        cause: String,
    },
}

impl RegistryError {
    fn io(cause: impl Into<String>) -> Self {
        Self::Io {
            cause: cause.into(),
        }
    }
}

/// 节点状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    /// 当前有活跃链路。
    Online,
    /// 已配对但当前无链路（或从未连上）。
    Offline,
    /// 已吊销：不得再接入，也不能靠重新配对绕过。
    Revoked,
}

/// 注册表里的一个节点条目。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeEntry {
    /// 稳定节点标识。
    pub id: String,
    /// 长期凭据的哈希（原始凭据只在签发时出现一次）。
    credential_hash: String,
    /// 当前状态。
    pub status: NodeStatus,
    /// 最后一次成功握手的时间（unix 秒）。
    #[serde(default)]
    pub last_seen_unix: Option<i64>,
    /// 首次配对时间（unix 秒）。
    pub created_unix: i64,
}

impl NodeEntry {
    /// 节点标识。
    pub fn id(&self) -> &str {
        &self.id
    }

    /// 状态。
    pub fn status(&self) -> NodeStatus {
        self.status
    }

    /// 最后出现时间。
    pub fn last_seen_unix(&self) -> Option<i64> {
        self.last_seen_unix
    }

    /// 首次配对时间。
    pub fn created_unix(&self) -> i64 {
        self.created_unix
    }
}

/// 尚未使用的 join token 的对外视图（原始 token 不可再读出——只签发时给一次）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinTokenView {
    /// 哈希前缀（便于操作者对照日志排查）。
    pub hash_prefix: String,
    /// 签发时间。
    pub created_unix: i64,
    /// 过期时间。
    pub expires_unix: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JoinTokenEntry {
    token_hash: String,
    created_unix: i64,
    expires_unix: i64,
    /// 已被哪个节点消费（消费即不可再用）。
    #[serde(default)]
    consumed_by: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct RegistryData {
    #[serde(default)]
    join_tokens: Vec<JoinTokenEntry>,
    #[serde(default)]
    nodes: Vec<NodeEntry>,
}

/// 节点注册表（持有独占写权）。
#[derive(Debug, Clone)]
pub struct NodeRegistry {
    path: PathBuf,
    data: RegistryData,
}

impl NodeRegistry {
    /// 打开（或创建）注册表。
    ///
    /// 文件缺失 → 空表；文件损坏 → 隔离改名 + 空表（不拒绝启动）。
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, RegistryError> {
        let path = path.into();
        let data = match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<RegistryData>(&text) {
                Ok(data) => data,
                Err(e) => {
                    let quarantine = quarantine_path(&path);
                    std::fs::rename(&path, &quarantine).map_err(|rename_err| {
                        RegistryError::io(format!(
                            "{} 解析失败（{e}）且无法隔离到 {}：{rename_err}",
                            path.display(),
                            quarantine.display()
                        ))
                    })?;
                    eprintln!(
                        "warning: 节点注册表 {} 无法解析（{e}）；已隔离到 {}，以空表启动",
                        path.display(),
                        quarantine.display()
                    );
                    RegistryData::default()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => RegistryData::default(),
            Err(e) => {
                return Err(RegistryError::io(format!(
                    "无法读取 {}：{e}",
                    path.display()
                )));
            }
        };
        Ok(Self { path, data })
    }

    /// 注册表文件路径。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 签发一个一次性 join token，返回**原始 token**（此后不可再读出）。
    pub fn issue_join_token(
        &mut self,
        now_unix: i64,
        ttl_secs: i64,
    ) -> Result<String, RegistryError> {
        let raw = random_secret();
        self.data.join_tokens.push(JoinTokenEntry {
            token_hash: hash_hex(&raw),
            created_unix: now_unix,
            expires_unix: now_unix + ttl_secs.max(0),
            consumed_by: None,
        });
        self.save()?;
        Ok(raw)
    }

    /// 列出仍可用（未消费、未过期）的 join token。
    pub fn pending_join_tokens(&self, now_unix: i64) -> Vec<JoinTokenView> {
        self.data
            .join_tokens
            .iter()
            .filter(|t| t.consumed_by.is_none() && t.expires_unix > now_unix)
            .map(|t| JoinTokenView {
                hash_prefix: t.token_hash.chars().take(8).collect(),
                created_unix: t.created_unix,
                expires_unix: t.expires_unix,
            })
            .collect()
    }

    /// 消费一个 join token（一次性、带过期）。
    pub fn consume_join_token(
        &mut self,
        raw: &str,
        node_id: &str,
        now_unix: i64,
    ) -> Result<(), Rejection> {
        let wanted = hash_hex(raw);
        let entry = self
            .data
            .join_tokens
            .iter_mut()
            .find(|t| constant_time_eq(t.token_hash.as_bytes(), wanted.as_bytes()));
        let Some(entry) = entry else {
            return Err(Rejection::new(
                RejectCode::InvalidJoinToken,
                "join token 不认识（可能打错，或在另一台主控上签发）",
            ));
        };
        if entry.consumed_by.is_some() {
            return Err(Rejection::new(
                RejectCode::JoinTokenConsumed,
                "join token 已被消费（一次性）",
            ));
        }
        if entry.expires_unix <= now_unix {
            return Err(Rejection::new(
                RejectCode::JoinTokenExpired,
                format!("join token 已于 {} 过期", entry.expires_unix),
            ));
        }
        entry.consumed_by = Some(node_id.to_string());
        // 消费必须立刻落盘：否则主控崩溃后同一个 token 可以被再次消费。
        // 落盘失败对节点而言是「主控暂时不可用」（瞬时可重试），而不是令牌问题。
        self.save().map_err(|e| persist_rejection(e))
    }

    /// 配对：签发长期凭据并登记节点。返回**原始凭据**（只出现这一次）。
    ///
    /// 同 id 的在线节点或已吊销节点都会被拒绝（见模块文档）。
    pub fn pair(&mut self, node_id: &str, now_unix: i64) -> Result<String, Rejection> {
        if let Some(existing) = self.data.nodes.iter().find(|n| n.id == node_id) {
            match existing.status {
                NodeStatus::Online => {
                    return Err(Rejection::new(
                        RejectCode::NodeIdConflict,
                        format!("节点 {node_id} 已在线；同一身份不能被两台机器同时使用"),
                    ));
                }
                NodeStatus::Revoked => {
                    return Err(Rejection::new(
                        RejectCode::NodeIdConflict,
                        format!(
                            "节点 {node_id} 已被吊销；先在主控侧删除该条目（或在解除吊销前换一个 id）"
                        ),
                    ));
                }
                NodeStatus::Offline => { /* 允许重新配对：凭据轮换、旧凭据立即失效 */ }
            }
        }

        let raw = random_secret();
        let hash = hash_hex(&raw);
        match self.data.nodes.iter_mut().find(|n| n.id == node_id) {
            Some(existing) => {
                existing.credential_hash = hash;
                existing.status = NodeStatus::Offline;
                existing.last_seen_unix = None;
            }
            None => self.data.nodes.push(NodeEntry {
                id: node_id.to_string(),
                credential_hash: hash,
                status: NodeStatus::Offline,
                last_seen_unix: None,
                created_unix: now_unix,
            }),
        }
        // 凭据签发必须落盘后才算数：否则节点会拿着一份主控不认识的凭据重连。
        self.save().map_err(|e| persist_rejection(e))?;
        Ok(raw)
    }

    /// 校验长期凭据。返回节点当前状态（未吊销即可接入）。
    ///
    /// 「节点不存在」与「凭据不对」返回**同一个**拒绝码：不泄露某个 id 是否已注册。
    pub fn authenticate(&self, node_id: &str, secret: &str) -> Result<NodeStatus, Rejection> {
        let Some(entry) = self.data.nodes.iter().find(|n| n.id == node_id) else {
            return Err(invalid_credential());
        };
        if !constant_time_eq(
            entry.credential_hash.as_bytes(),
            hash_hex(secret).as_bytes(),
        ) {
            return Err(invalid_credential());
        }
        if entry.status == NodeStatus::Revoked {
            return Err(Rejection::new(
                RejectCode::CredentialRevoked,
                format!("节点 {node_id} 的凭据已被吊销"),
            ));
        }
        Ok(entry.status)
    }

    /// 标记节点在线并刷新最后出现时间。
    pub fn mark_online(&mut self, node_id: &str, now_unix: i64) -> Result<(), RegistryError> {
        if let Some(entry) = self.data.nodes.iter_mut().find(|n| n.id == node_id) {
            entry.status = NodeStatus::Online;
            entry.last_seen_unix = Some(now_unix);
        }
        self.save()
    }

    /// 标记节点离线（链路断开；凭据仍有效）。
    pub fn mark_offline(&mut self, node_id: &str) -> Result<(), RegistryError> {
        if let Some(entry) = self.data.nodes.iter_mut().find(|n| n.id == node_id)
            && entry.status == NodeStatus::Online
        {
            entry.status = NodeStatus::Offline;
        }
        self.save()
    }

    /// 吊销节点凭据：此后不能再接入，也不能靠重新配对绕过。
    pub fn revoke(&mut self, node_id: &str) -> Result<bool, RegistryError> {
        let mut found = false;
        if let Some(entry) = self.data.nodes.iter_mut().find(|n| n.id == node_id) {
            entry.status = NodeStatus::Revoked;
            found = true;
        }
        self.save()?;
        Ok(found)
    }

    /// 全部节点条目。
    pub fn nodes(&self) -> &[NodeEntry] {
        &self.data.nodes
    }

    /// 按 id 取节点。
    pub fn node(&self, node_id: &str) -> Option<&NodeEntry> {
        self.data.nodes.iter().find(|n| n.id == node_id)
    }

    /// 原子落盘（私有权限：文件里有凭据哈希，按敏感处理）。
    fn save(&self) -> Result<(), RegistryError> {
        if let Some(parent) = self.path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|e| {
                RegistryError::io(format!("无法创建 {}：{e}", parent.display()))
            })?;
        }
        let text = serde_json::to_string_pretty(&self.data)
            .map_err(|e| RegistryError::io(format!("无法序列化注册表：{e}")))?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, text.as_bytes())
            .map_err(|e| RegistryError::io(format!("无法写入 {}：{e}", tmp.display())))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(&tmp, &self.path)
            .map_err(|e| RegistryError::io(format!("无法落盘 {}：{e}", self.path.display())))
    }
}

fn invalid_credential() -> Rejection {
    Rejection::new(
        RejectCode::CredentialInvalid,
        "凭据不正确或该节点未注册",
    )
}

/// 主控侧落盘失败 → 节点侧应把它当作**瞬时**问题退避重试，而不是把令牌/凭据判死。
fn persist_rejection(e: RegistryError) -> Rejection {
    Rejection::new(RejectCode::ControlPlaneUnavailable, e.to_string())
}

/// 隔离文件名：`<原名>.corrupt-<unix>`。
fn quarantine_path(path: &Path) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".corrupt-{stamp}"));
    path.with_file_name(name)
}

/// 32 字节随机 → hex。join token 与长期凭据同源（都是不可猜的 bearer）。
fn random_secret() -> String {
    let mut buf = [0u8; 32];
    getrandom::fill(&mut buf).expect("OS CSPRNG unavailable");
    hex::encode(buf)
}

/// SHA-256 → hex。文件里只存哈希，原始 token/凭据不落盘。
fn hash_hex(raw: &str) -> String {
    use sha2::Digest;
    let digest: [u8; 32] = sha2::Sha256::digest(raw.as_bytes()).into();
    hex::encode(digest)
}

/// 常数时间比较（与 `sebas-webui::auth` 同一取向：不因比较提前返回而泄露前缀）。
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_700_000_000;

    fn registry() -> (tempfile::TempDir, NodeRegistry) {
        let dir = tempfile::tempdir().unwrap();
        let reg = NodeRegistry::open(dir.path().join("nodes.json")).unwrap();
        (dir, reg)
    }

    #[test]
    fn join_token_is_one_time_and_hashed_at_rest() {
        let (_tmp, mut reg) = registry();
        let raw = reg.issue_join_token(T0, 600).unwrap();
        assert_eq!(raw.len(), 64, "token 应为 32 字节 hex");
        // 文件里不得出现原始 token。
        let text = std::fs::read_to_string(reg.path()).unwrap();
        assert!(!text.contains(&raw), "原始 token 不应落盘");
        assert!(text.contains(&hash_hex(&raw)), "应存哈希");

        reg.consume_join_token(&raw, "dev-box", T0 + 1).unwrap();
        let err = reg.consume_join_token(&raw, "dev-box-2", T0 + 2).unwrap_err();
        assert_eq!(err.code, RejectCode::JoinTokenConsumed);
    }

    #[test]
    fn expired_and_unknown_tokens_are_distinguished() {
        let (_tmp, mut reg) = registry();
        let raw = reg.issue_join_token(T0, 60).unwrap();
        let err = reg.consume_join_token(&raw, "n", T0 + 61).unwrap_err();
        assert_eq!(err.code, RejectCode::JoinTokenExpired);

        let err = reg.consume_join_token("not-a-token", "n", T0).unwrap_err();
        assert_eq!(err.code, RejectCode::InvalidJoinToken);
    }

    #[test]
    fn pending_tokens_exclude_consumed_and_expired() {
        let (_tmp, mut reg) = registry();
        let a = reg.issue_join_token(T0, 600).unwrap();
        let _b = reg.issue_join_token(T0, 60).unwrap();
        assert_eq!(reg.pending_join_tokens(T0).len(), 2);
        reg.consume_join_token(&a, "n1", T0).unwrap();
        assert_eq!(reg.pending_join_tokens(T0).len(), 1, "已消费的不再列出");
        assert_eq!(reg.pending_join_tokens(T0 + 61).len(), 0, "过期的不再列出");
        // 视图只给哈希前缀，不给可用 token。
        let views = reg.pending_join_tokens(T0 + 61);
        assert!(views.is_empty());
    }

    #[test]
    fn pairing_issues_a_credential_and_authenticates() {
        let (_tmp, mut reg) = registry();
        let token = reg.issue_join_token(T0, 600).unwrap();
        reg.consume_join_token(&token, "dev-box", T0).unwrap();
        let secret = reg.pair("dev-box", T0).unwrap();
        assert_eq!(secret.len(), 64);

        let status = reg.authenticate("dev-box", &secret).unwrap();
        assert_eq!(status, NodeStatus::Offline, "配对后仍需一次握手才算在线");

        reg.mark_online("dev-box", T0 + 5).unwrap();
        assert_eq!(reg.authenticate("dev-box", &secret).unwrap(), NodeStatus::Online);
        let entry = reg.node("dev-box").unwrap();
        assert_eq!(entry.last_seen_unix(), Some(T0 + 5));
        assert_eq!(entry.created_unix(), T0);

        reg.mark_offline("dev-box").unwrap();
        assert_eq!(reg.authenticate("dev-box", &secret).unwrap(), NodeStatus::Offline);
    }

    #[test]
    fn wrong_or_unknown_credentials_share_one_code() {
        let (_tmp, mut reg) = registry();
        let secret = reg.pair("dev-box", T0).unwrap();

        let wrong = reg.authenticate("dev-box", "not-the-secret").unwrap_err();
        let unknown = reg.authenticate("other-box", &secret).unwrap_err();
        assert_eq!(wrong.code, RejectCode::CredentialInvalid);
        assert_eq!(unknown.code, RejectCode::CredentialInvalid);
        // 不泄露「某个 id 是否已注册」：两者码与文案一致。
        assert_eq!(wrong.cause, unknown.cause);
    }

    #[test]
    fn revoked_node_cannot_authenticate_or_re_presence() {
        let (_tmp, mut reg) = registry();
        let secret = reg.pair("dev-box", T0).unwrap();
        reg.mark_online("dev-box", T0).unwrap();
        assert!(reg.revoke("dev-box").unwrap());

        let err = reg.authenticate("dev-box", &secret).unwrap_err();
        assert_eq!(err.code, RejectCode::CredentialRevoked);

        // 吊销不可被重新配对绕过。
        let err = reg.pair("dev-box", T0 + 10).unwrap_err();
        assert_eq!(err.code, RejectCode::NodeIdConflict);
        assert!(err.cause.contains("吊销"), "{}", err.cause);
        assert!(!reg.revoke("nonexistent").unwrap());
    }

    #[test]
    fn same_id_cannot_be_online_twice() {
        let (_tmp, mut reg) = registry();
        let first = reg.pair("dev-box", T0).unwrap();
        reg.mark_online("dev-box", T0).unwrap();

        // 已在线 → 拒绝（防两台机器冒用同一身份）。
        let err = reg.pair("dev-box", T0 + 1).unwrap_err();
        assert_eq!(err.code, RejectCode::NodeIdConflict);
        // 原凭据仍然有效（拒绝没有破坏现状）。
        assert_eq!(reg.authenticate("dev-box", &first).unwrap(), NodeStatus::Online);
    }

    #[test]
    fn re_pairing_an_offline_node_rotates_the_credential() {
        let (_tmp, mut reg) = registry();
        let old = reg.pair("dev-box", T0).unwrap();
        reg.mark_online("dev-box", T0).unwrap();
        reg.mark_offline("dev-box").unwrap();

        // 离线节点用同一 id 重新配对：允许（重装接续），凭据轮换。
        let new = reg.pair("dev-box", T0 + 100).unwrap();
        assert_ne!(old, new);
        assert_eq!(
            reg.authenticate("dev-box", &old).unwrap_err().code,
            RejectCode::CredentialInvalid,
            "旧凭据应立即失效"
        );
        assert!(reg.authenticate("dev-box", &new).is_ok());
        assert_eq!(reg.nodes().len(), 1, "轮换不应产生第二条节点");
    }

    #[test]
    fn registry_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nodes.json");
        let secret = {
            let mut reg = NodeRegistry::open(&path).unwrap();
            let secret = reg.pair("dev-box", T0).unwrap();
            reg.mark_online("dev-box", T0 + 1).unwrap();
            let token = reg.issue_join_token(T0, 600).unwrap();
            reg.consume_join_token(&token, "n2", T0).unwrap();
            secret
        };
        let reg = NodeRegistry::open(&path).unwrap();
        assert_eq!(reg.nodes().len(), 1);
        assert_eq!(reg.authenticate("dev-box", &secret).unwrap(), NodeStatus::Online);
        assert_eq!(reg.pending_join_tokens(T0).len(), 0, "消费状态也应持久");
    }

    #[test]
    fn missing_file_starts_empty() {
        let dir = tempfile::tempdir().unwrap();
        let reg = NodeRegistry::open(dir.path().join("nope/nodes.json")).unwrap();
        assert!(reg.nodes().is_empty());
        assert!(reg.pending_join_tokens(T0).is_empty());
    }

    #[test]
    fn corrupt_file_is_quarantined_and_startup_survives() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nodes.json");
        std::fs::write(&path, "{ this is not json").unwrap();

        let reg = NodeRegistry::open(&path).unwrap();
        assert!(reg.nodes().is_empty(), "损坏文件 → 空表启动");
        assert!(!path.exists(), "损坏文件应被移走");
        let quarantined: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".corrupt-"))
            .collect();
        assert_eq!(quarantined.len(), 1, "应留下一份隔离文件供排查");
    }

    #[test]
    fn rejection_maps_onto_the_protocol_ack() {
        let rejection = Rejection::new(RejectCode::JoinTokenConsumed, "已被消费");
        let ack = rejection.to_ack();
        let (code, cause) = sebas_node_link::rejection_of(&ack).unwrap();
        assert_eq!(code, RejectCode::JoinTokenConsumed);
        assert_eq!(cause, "已被消费");
        assert_eq!(ack.protocol_version, sebas_node_link::PROTOCOL_VERSION);
        assert!(rejection.to_string().contains("join_token_consumed"));
    }

    #[cfg(unix)]
    #[test]
    fn registry_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let (_tmp, mut reg) = registry();
        reg.pair("dev-box", T0).unwrap();
        let mode = std::fs::metadata(reg.path()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "注册表含凭据哈希，应按敏感文件处理");
    }
}
