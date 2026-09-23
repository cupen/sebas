//! `SessionMapRow` — session_map 表的 ActiveRecord struct（复合主键）+ 域查询。
//!
//! struct 即表结构事实源；复合主键 `PRIMARY KEY (chat_id, thread_id)` 只在
//! 根 crate 注册表的 DDL 里表达，这里用 `#[active_record(pk = …)]` 声明
//! CRUD 的定位键（生成 `find_by` / `delete_by`）。
//!
//! persist-session-map 1.1：表按会话映射的**完整目标形状**重建——一行即一个
//! 持久化映射实例（与 dispatch 的 `Mapping` 一一对应）。键列承载
//! [`sebas_channels`] 的 ChannelKey 身份：`chat_id` = channel（会话键命名
//! 空间），`thread_id` = reference（通道自己的引用，飞书含 `chat\0thread`
//! 组合）——主键 `(chat_id, thread_id)` 因此就是完整会话键（列名是历史
//! 沿革，寻址语义不变）。新增的映射列均为 NOT NULL 无常量默认或可空列：
//! 旧形状的库在打开时按「缺列非空且无默认」走隔离重置（产品未发布，
//! D2/D5 接受一次重置，隔离保留痕迹）。
//!
//! 持久化语义（persist-session-map D2）：**按变更落盘**——生命周期事件处
//! 一次 `entry.save(&conn)`（ActiveRecord 生成，无手写 SQL），删除走
//! `delete_by`；不再有关停全量快照，故全量替换式写入不复存在。

use sebas_db::rusqlite::Connection;
use sebas_schema_derive::{ActiveRecord, SchemaColumns};

/// session_map 行：一行 = 一个持久化映射（Active/Dormant/0-turn 占位）。
/// in-flight spawn 占位与 SpawnFailed 终态**不入表**（state-store spec：
/// in-flight spawn 占位不得持久化）。
#[derive(Debug, Clone, PartialEq, Eq, SchemaColumns, ActiveRecord)]
#[active_record(table = "session_map")]
#[active_record(pk = "chat_id")]
#[active_record(pk = "thread_id")]
pub struct SessionMapRow {
    /// ChannelKey.channel：会话键命名空间（feishu / web / …）。
    pub chat_id: String,
    /// ChannelKey.reference：通道自己的引用（飞书 = `chat\0thread` 组合）。
    pub thread_id: Option<String>,
    /// 路由 id；空串 = 0-turn 占位（`awaiting_first_prompt = true`，
    /// 子进程从未存在）。
    pub session_id: String,
    pub last_active_unix: i64,
    /// 项目目录（非飞书会话的从属不变量；0-turn 占位的 spawn 目标）。
    pub project_dir: Option<String>,
    /// agent 侧真实 ACP 会话 id（native-ACP agent；resume 按它 load）。
    pub acp_session_id: Option<String>,
    /// 会话当前模型 id。
    pub current_model: Option<String>,
    /// 创建时绑定的执行后端 kind（占位记住，spawn 时消费）。
    pub pending_kind: Option<String>,
    /// 创建时请求的模型 id（占位记住，spawn 时消费）。
    pub pending_model: Option<String>,
    /// 创建时请求的 mode（占位记住，spawn 时消费）。
    pub pending_mode: Option<String>,
    /// 操作者期望的会话 mode（控制面词 ask/edit/allow/auto，非空）。
    pub desired_mode: String,
    /// 操作者设置的会话 label（跨重启保持）。
    pub label: Option<String>,
    /// 首条 prompt 预览（命名来源迁移位）。
    pub prompt_preview: Option<String>,
    /// 0-turn 占位身份标记（true + 空 session_id = 等待首条消息的占位）。
    pub awaiting_first_prompt: bool,
}

/// 加载会话映射 (用于恢复；无排序语义，与既有行为一致)。
pub fn load_session_map(conn: &mut Connection) -> Result<Vec<SessionMapRow>, String> {
    SessionMapRow::all(conn).map_err(|e| format!("查询 session_map 失败: {e}"))
}
