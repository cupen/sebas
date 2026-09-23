//! `sebas-domain` — 中立共享域层（add-domain-layer）。
//!
//! 根 crate `sebas` 依赖几乎全部成员，是依赖图的死端；域概念若定义在根里，
//! 对 `sebas-webui` / `sebas-dispatch` / `sebas-router` / `sebas-im` /
//! `sebas-node` 一律不可见，复制成了唯一出路。本 crate 是**每个人都能依赖的
//! 叶子**：每个跨角色域概念在这里唯一定义，各角色经普通 path 依赖取用；
//! 原 crate 以 `pub use` 原位再导出保持既有公开路径（design D3）。
//!
//! # 准入规则
//!
//! 一个类型/函数进入本 crate 必须同时满足：
//!
//! 1. **≥2 个 crate 需要**它（单消费方 = 还没有共享的理由）；
//! 2. **角色中立**——不携带 core / webui / router / im / node 任何一个角色
//!    的实现关注点（依赖图上机械可核对：`cargo tree -p sebas-domain` 不得
//!    出现任何角色 crate 或 sebas-node，见根 crate 的叶子属性断言测试）。
//!
//! 已知**拒收**清单（design「Risks」）：`SessionRow`（展示派生）、
//! `HostedSession`（节点内部）、`ProviderProfile`（节点自有语义）、
//! watchdog 的 `OperationStatus`（服务监督域）。
//!
//! # 模块
//!
//! - [`session`]：会话身份、状态与事件流形状（SessionInfo 及其同伴）；
//! - [`project`]：项目注册表条目（webui 注册表与持久行之外的 JSON 形状）；
//! - [`provider`]：provider 状态词表（仍是词表的部分）+ providers.json
//!   overlay 的读取器；
//! - [`node`]：执行节点的管理面视图（NodeView，core 通道与 webui 共用）；
//! - [`prim`]：中立原语（路径展开、时间戳）——轻微异味，design D7 承认；
//!   增长到需要自己的依赖时拆出。
//! - [`state_paths`]：状态路径映射表（single-state-dir）——逻辑名 → 所属库
//!   → 文件名 → 覆盖变量的唯一规则表，全部落点从单一状态目录派生。
//!
//! 模块划分是实现便利（design「Open Questions」），不改变准入规则。
//!
//! # 不变的契约
//!
//! 本 crate 只承载形状；所有线格式与磁盘形状逐字节不变（spec「Wire and
//! on-disk compatibility preserved」）。改字段名/标签/列名的需求一律是
//! 独立的显式 breaking change，不经由本 crate 的搬迁顺手发生。

pub mod node;
pub mod prim;
pub mod project;
pub mod provider;
pub mod session;
pub mod state_paths;
