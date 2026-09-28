//! 落点映射表（unify-sebas-home D1/D2/D6）：**逻辑名 → 相对主目录的固定
//! 路径 → 覆盖变量**，全部 sebas 自有落点从单一 sebas home 派生的唯一规则
//! 表。
//!
//! # 解析优先级（unify-sebas-home D1）
//!
//! ```text
//! 逐落点显式覆盖（env / config 键）> sebas home（env SEBAS_HOME）> 默认（~/.sebas）
//! ```
//!
//! `SEBAS_HOME` 是唯一正名；`SEBAS_STATE_DIR` 降为**仍生效的兼容别名**——
//! 单独设置时行为与 `SEBAS_HOME` 完全等价，但启动日志会 warn 点名正名；
//! 二者同设时 `SEBAS_HOME` 赢（unify-sebas-home 的 BREAKING 优先级反转：
//! single-state-dir 时代是 `SEBAS_STATE_DIR` 优先）。[`legacy_alias_set`]
//! / [`alias_conflict_set`] 供启动日志做 warn 检出。
//!
//! 既有部署与沙箱菜谱的逐落点变量（`SEBAS_ARCHIVE_PATH` 等）保持原语义；
//! 未设置覆盖时行为确定。被否备选：别名压过正名（「统一」语义打折）。
//!
//! # 默认 sebas home
//!
//! `SEBAS_HOME`，其次 `SEBAS_STATE_DIR`（别名），最后默认 `~/.sebas`
//! （`dirs::home_dir()`，与 `expand_tilde` 同源）。解析只读环境变量，
//! **不依赖任何配置文件**——沙箱在写 config.toml 之前就能钉住全部落点。
//!
//! # 布局（unify-sebas-home D2）
//!
//! ```text
//! <home>/                       四库 + archive/services/nodes json 平铺根（零迁移）
//! <home>/config.toml            config 缺省（显式 -c 在消费点覆盖）
//! <home>/core.secret            secret 缺省（跟 config 同目录的规则在消费点）
//! <home>/run/core.sock          core 会话通道 socket（易逝）
//! <home>/run/control.sock       watchdog 控制面 socket（易逝）
//! <home>/cache/downloads        media 下载缓存
//! <home>/node/                  sebas-node 状态（整目录语义）
//! <home>/upgrade/               watchdog 升级数据（upgrade.lock/versions/…）
//! ```
//!
//! # 分层规则（两级，语义不变）
//!
//! ```text
//! 第一级（写入进程）：core → settings.db / projects.db；webui → auth.db；
//!                     router → usage.db；node → 无库
//! 第二级（core 内，增长特征）：
//!   settings.db  有界：providers / model_aliases / settings
//!   projects.db  增长：projects / session_map（+ 后续会话与消息）
//! ```
//!
//! # 退休变量
//!
//! `SEBAS_STATE_DB` 随单库退休：不再被任何解析路径读取（导出与否行为完全
//! 一致），[`retired_env_vars_present`] 供启动日志对残留值给出明确提示。
//!
//! `SEBAS_PROJECTS_PATH` 随 `migrate-project-registry` 退休：项目注册表落
//! `projects.db` 的 `projects` 表，**没有** `projects.json` 这个文件，覆盖
//! 变量同样不再被读取。逻辑名仍留在映射表里（[`StatePath::ProjectRegistry`]），
//! 但它不再指向任何会被写入或读取的落点。
//!
//! # 测试纪律
//!
//! 环境变量是进程全局的；本模块测试用互斥锁串行并在用例前后保存/恢复。

use std::path::PathBuf;

/// sebas home 正名（unify-sebas-home D1：唯一正名，派生全部自有落点）。
pub const HOME_VAR: &str = "SEBAS_HOME";

/// 旧状态目录变量：语义与 `SEBAS_HOME` 相同（single-state-dir 时代的正名）。
/// **仍被完整采纳**（单独设置时行为等价），但降为兼容别名——启动 warn 点名
/// 正名；与 `SEBAS_HOME` 同设时被忽略（正名赢）。
pub const LEGACY_STATE_DIR_VAR: &str = "SEBAS_STATE_DIR";

/// 已退休的单库变量（`sebas.db` 不复存在）：不被读取，出现时只提示。
pub const RETIRED_STATE_DB_VAR: &str = "SEBAS_STATE_DB";

/// 已退休的项目注册表覆盖变量（`migrate-project-registry`：注册表落
/// `projects.db`，`projects.json` 不复存在）：不被读取，出现时只提示。
pub const RETIRED_PROJECTS_PATH_VAR: &str = "SEBAS_PROJECTS_PATH";

/// 按用途分层的数据库（写入者见模块文档分层规则第一级）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Database {
    /// core：有界系统配置（providers / model_aliases / settings）。
    Settings,
    /// core：增长的用户数据（projects / session_map，后续会话与消息）。
    Projects,
    /// webui：用户库（写入者是 webui，不并入 core 的库）。
    Auth,
    /// router：用量库（存储形态归 `persist-router-usage`；本表只定落点）。
    Usage,
}

impl Database {
    /// 库文件名（固定平铺在 sebas home 根下）。
    pub fn file_name(self) -> &'static str {
        match self {
            Database::Settings => "settings.db",
            Database::Projects => "projects.db",
            Database::Auth => "auth.db",
            Database::Usage => "usage.db",
        }
    }

    /// 逐库显式覆盖变量（single-state-dir D8：`SEBAS_STATE_DB` 由四个逐库
    /// 变量取代）。
    pub fn override_var(self) -> &'static str {
        match self {
            Database::Settings => "SEBAS_SETTINGS_DB",
            Database::Projects => "SEBAS_PROJECTS_DB",
            // 既有变量名原样保留（add-webui-multiuser-rbac 起就是它）。
            Database::Auth => "SEBAS_WEBUI_AUTH_DB",
            Database::Usage => "SEBAS_ROUTER_USAGE_DB",
        }
    }

    /// 库路径：逐库覆盖（tilde 展开）> sebas home 派生。
    pub fn resolve(self) -> PathBuf {
        override_path(self.override_var()).unwrap_or_else(|| sebas_home().join(self.file_name()))
    }
}

/// 落点逻辑名。每个变体 = 一条映射表行：所属库（文件类为 `None`）、相对
/// home 的固定路径、逐落点覆盖变量。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatePath {
    /// core 设置库（有界系统配置）。
    SettingsDb,
    /// core 用户数据库（增长数据）。
    ProjectsDb,
    /// webui 用户库。
    AuthDb,
    /// router 用量库。
    UsageDb,
    /// WebUI 会话归档登记册（含完整转录）。
    Archive,
    /// WebUI 项目注册表逻辑名。`migrate-project-registry` 后注册表落在
    /// `projects.db` 的 `projects` 表：本行只为逻辑名表完整保留，**文件已
    /// 退休**（无写入者、无读取者），覆盖变量也已退休（见
    /// [`RETIRED_PROJECTS_PATH_VAR`]）。
    ProjectRegistry,
    /// 节点链路注册表。显式覆盖走既有配置键 `[node_link] registry_file`
    /// （优先级不变），无环境变量。
    NodeRegistry,
    /// watchdog 的服务期望态覆盖层（`services.json`：操作员配置，保持为
    /// 文件——一个文件一个写入者，watchdog 自己写）。
    ServicesOverride,
    // ── unify-sebas-home 新族（D2：四个子目录 + 两个根平铺缺省）──
    /// config.toml 的**缺省**派生（`<home>/config.toml`）。显式 `-c` 是
    /// 消费点特例（clap 旗标），不在本表内——枚举项只表示缺省形态
    /// （design D6）。
    ConfigDefault,
    /// 核心通道 secret 文件的**缺省**派生（`<home>/core.secret`）。「与
    /// config 文件同目录」的规则原样保留在消费点：显式 `-c` 时 secret 跟
    /// 显式目录，缺省时一起落 home 根；`[service.core] secret_file` 键照旧
    /// 最优先。
    CoreSecretDefault,
    /// core 会话通道 socket（`<home>/run/core.sock`，易逝）。不再查
    /// `XDG_RUNTIME_DIR`；`SEBAS_CORE_SOCKET` 与 `[service.core]
    /// channel_path` 覆盖照旧（config 键在消费点优先于本表 env）。
    ChannelSocket,
    /// watchdog 控制面 socket（`<home>/run/control.sock`，易逝）。不再查
    /// `XDG_RUNTIME_DIR`；`SEBAS_CONTROL_SOCKET` 与 CLI `--socket` 覆盖照旧
    /// （`--socket` 在消费点优先于本表 env）。
    ControlSocket,
    /// media 下载缓存目录（`<home>/cache/downloads`）。`[media]
    /// download_dir` 键照旧优先。
    MediaDownloads,
    /// sebas-node 状态目录（`<home>/node/`，整目录语义，身份随目录搬移）。
    /// `--state-dir` > `SEBAS_NODE_DIR` > `[node] state_dir` 链序不变。
    NodeStateDir,
    /// watchdog 升级数据目录（`<home>/upgrade/`，内部布局原样）。
    /// `[watchdog.storage] data_dir` 键照旧优先。
    UpgradeDataDir,
}

impl StatePath {
    /// 该落点属于哪个库；文件类落点为 `None`。
    pub fn database(self) -> Option<Database> {
        match self {
            StatePath::SettingsDb => Some(Database::Settings),
            StatePath::ProjectsDb => Some(Database::Projects),
            StatePath::AuthDb => Some(Database::Auth),
            StatePath::UsageDb => Some(Database::Usage),
            StatePath::Archive
            | StatePath::ProjectRegistry
            | StatePath::NodeRegistry
            | StatePath::ServicesOverride
            | StatePath::ConfigDefault
            | StatePath::CoreSecretDefault
            | StatePath::ChannelSocket
            | StatePath::ControlSocket
            | StatePath::MediaDownloads
            | StatePath::NodeStateDir
            | StatePath::UpgradeDataDir => None,
        }
    }

    /// 相对 sebas home 的固定路径（派生形态 = home + 本路径；含子目录——
    /// 平铺名册在根，新收编落点进 `run/`、`cache/`、`node/`、`upgrade/`，
    /// design D2）。
    pub fn rel_path(self) -> &'static str {
        match self {
            StatePath::SettingsDb => Database::Settings.file_name(),
            StatePath::ProjectsDb => Database::Projects.file_name(),
            StatePath::AuthDb => Database::Auth.file_name(),
            StatePath::UsageDb => Database::Usage.file_name(),
            StatePath::Archive => "archive.json",
            StatePath::ProjectRegistry => "projects.json",
            StatePath::NodeRegistry => "nodes.json",
            StatePath::ServicesOverride => "services.json",
            StatePath::ConfigDefault => "config.toml",
            StatePath::CoreSecretDefault => "core.secret",
            StatePath::ChannelSocket => "run/core.sock",
            StatePath::ControlSocket => "run/control.sock",
            StatePath::MediaDownloads => "cache/downloads",
            StatePath::NodeStateDir => "node",
            StatePath::UpgradeDataDir => "upgrade",
        }
    }

    /// 逐落点覆盖变量；`None` = 无环境变量入口——显式覆盖走 config 键
    /// （`[node_link] registry_file` / `[media] download_dir` /
    /// `[watchdog.storage] data_dir` / `[service.core] secret_file`）、CLI
    /// 旗标（`-c` / `--socket`），或该覆盖变量已退休
    /// （[`StatePath::ProjectRegistry`]）。
    pub fn override_var(self) -> Option<&'static str> {
        match self {
            StatePath::SettingsDb => Some(Database::Settings.override_var()),
            StatePath::ProjectsDb => Some(Database::Projects.override_var()),
            StatePath::AuthDb => Some(Database::Auth.override_var()),
            StatePath::UsageDb => Some(Database::Usage.override_var()),
            StatePath::Archive => Some("SEBAS_ARCHIVE_PATH"),
            // 退休：注册表落 projects.db，没有 projects.json 可覆盖。
            StatePath::ProjectRegistry => None,
            StatePath::NodeRegistry => None,
            StatePath::ServicesOverride => Some("SEBAS_SERVICES_FILE"),
            // 收编的三个 env 入口（unify-sebas-home 1.1）：其余新族落点的
            // 显式覆盖走 config 键 / CLI 旗标，不新增变量。
            StatePath::ChannelSocket => Some("SEBAS_CORE_SOCKET"),
            StatePath::ControlSocket => Some("SEBAS_CONTROL_SOCKET"),
            StatePath::NodeStateDir => Some("SEBAS_NODE_DIR"),
            StatePath::ConfigDefault
            | StatePath::CoreSecretDefault
            | StatePath::MediaDownloads
            | StatePath::UpgradeDataDir => None,
        }
    }

    /// 在给定主目录下的派生路径（不经任何覆盖——机械断言用）。
    pub fn derived_in(self, dir: &std::path::Path) -> PathBuf {
        dir.join(self.rel_path())
    }

    /// 解析落点：逐落点覆盖（tilde 展开）> sebas home 派生（design D1）。
    pub fn resolve(self) -> PathBuf {
        match self.override_var() {
            Some(var) => override_path(var).unwrap_or_else(|| sebas_home().join(self.rel_path())),
            None => sebas_home().join(self.rel_path()),
        }
    }
}

/// 读一个覆盖变量并展开 `~/` 前缀（state-store spec「Paths SHALL expand a
/// leading `~/`」）。空串视同未设置（与各既有读取点的空值忽略语义一致）。
fn override_path(var: &str) -> Option<PathBuf> {
    std::env::var(var)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(|v| PathBuf::from(crate::prim::expand_tilde(&v)))
}

fn env_nonempty(var: &str) -> Option<String> {
    std::env::var(var)
        .ok()
        .filter(|v| !v.trim().is_empty())
}

/// sebas home：`SEBAS_HOME` > `SEBAS_STATE_DIR`（兼容别名）> `~/.sebas`。
/// 纯环境变量解析，不依赖配置文件（unify-sebas-home 1.2：正名反转——同设
/// 时正名赢，[`alias_conflict_set`] 供启动 warn）。
pub fn sebas_home() -> PathBuf {
    for var in [HOME_VAR, LEGACY_STATE_DIR_VAR] {
        if let Some(v) = env_nonempty(var) {
            return PathBuf::from(crate::prim::expand_tilde(&v));
        }
    }
    PathBuf::from(crate::prim::expand_tilde("~/.sebas"))
}

/// config.toml 的缺省路径（`<sebas home>/config.toml`）：`-c` 缺席时全部
/// 子命令共用的解析器（unify-sebas-home D3——clap 静态缺省解析不了 env，
/// 缺省收敛到本函数；显式 `-c` 在旗标层覆盖）。
pub fn default_config_path() -> PathBuf {
    StatePath::ConfigDefault.resolve()
}

/// 兼容别名 `SEBAS_STATE_DIR` 当前**单独生效**（非空设置、正名缺席）。
/// 启动日志据此 warn 点名 `SEBAS_HOME` 为正名（unify-sebas-home 1.2）。
pub fn legacy_alias_set() -> bool {
    env_nonempty(LEGACY_STATE_DIR_VAR).is_some() && env_nonempty(HOME_VAR).is_none()
}

/// 正名与别名**同时**非空设置（冲突态：`SEBAS_HOME` 赢，别名被忽略）。
/// 启动日志据此 warn 说明优先级（unify-sebas-home 1.2）。
pub fn alias_conflict_set() -> bool {
    env_nonempty(LEGACY_STATE_DIR_VAR).is_some() && env_nonempty(HOME_VAR).is_some()
}

/// 检出当前进程环境里**已退休仍被导出**的变量（启动日志提示用；只报告，
/// 不读取其值——退休变量的语义是「无效果」）。
pub fn retired_env_vars_present() -> Vec<&'static str> {
    [RETIRED_STATE_DB_VAR, RETIRED_PROJECTS_PATH_VAR]
        .into_iter()
        .filter(|v| std::env::var(v).is_ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 进程级 env 串行锁 + 保存/恢复护栏。所有动 env 的用例必须持有。
    struct EnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        saved: Vec<(&'static str, Option<String>)>,
    }

    impl EnvGuard {
        /// 清掉本模块涉及的全部变量后进入干净环境。
        fn clean() -> Self {
            static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
            let lock = LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let vars: &[&'static str] = &[
                HOME_VAR,
                LEGACY_STATE_DIR_VAR,
                RETIRED_STATE_DB_VAR,
                "SEBAS_SETTINGS_DB",
                "SEBAS_PROJECTS_DB",
                "SEBAS_WEBUI_AUTH_DB",
                "SEBAS_ROUTER_USAGE_DB",
                "SEBAS_ARCHIVE_PATH",
                RETIRED_PROJECTS_PATH_VAR,
                "SEBAS_SERVICES_FILE",
                "SEBAS_CORE_SOCKET",
                "SEBAS_CONTROL_SOCKET",
                "SEBAS_NODE_DIR",
            ];
            let saved = vars
                .iter()
                .map(|v| (*v, std::env::var(v).ok()))
                .collect::<Vec<_>>();
            for v in vars {
                unsafe { std::env::remove_var(v) };
            }
            Self { _lock: lock, saved }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (v, prev) in &self.saved {
                match prev {
                    Some(val) => unsafe { std::env::set_var(v, val) },
                    None => unsafe { std::env::remove_var(v) },
                }
            }
        }
    }

    /// 全部逻辑名（含 unify-sebas-home 新族）——机械断言的枚举基准。
    const ALL: &[StatePath] = &[
        StatePath::SettingsDb,
        StatePath::ProjectsDb,
        StatePath::AuthDb,
        StatePath::UsageDb,
        StatePath::Archive,
        StatePath::ProjectRegistry,
        StatePath::NodeRegistry,
        StatePath::ServicesOverride,
        StatePath::ConfigDefault,
        StatePath::CoreSecretDefault,
        StatePath::ChannelSocket,
        StatePath::ControlSocket,
        StatePath::MediaDownloads,
        StatePath::NodeStateDir,
        StatePath::UpgradeDataDir,
    ];

    // ---- 任务 1.1：全部逻辑名的相对路径、所属库与覆盖变量名 ----

    #[test]
    fn every_logical_name_maps_to_rel_path_database_and_override() {
        // (逻辑名, 所属库, 相对 home 的固定路径, 覆盖变量)
        let want: &[(StatePath, Option<Database>, &str, Option<&str>)] = &[
            (
                StatePath::SettingsDb,
                Some(Database::Settings),
                "settings.db",
                Some("SEBAS_SETTINGS_DB"),
            ),
            (
                StatePath::ProjectsDb,
                Some(Database::Projects),
                "projects.db",
                Some("SEBAS_PROJECTS_DB"),
            ),
            (
                StatePath::AuthDb,
                Some(Database::Auth),
                "auth.db",
                Some("SEBAS_WEBUI_AUTH_DB"),
            ),
            (
                StatePath::UsageDb,
                Some(Database::Usage),
                "usage.db",
                Some("SEBAS_ROUTER_USAGE_DB"),
            ),
            (StatePath::Archive, None, "archive.json", Some("SEBAS_ARCHIVE_PATH")),
            (StatePath::ProjectRegistry, None, "projects.json", None),
            // 节点注册表的显式覆盖是配置键 [node_link] registry_file，
            // 不设环境变量（优先级不变）。
            (StatePath::NodeRegistry, None, "nodes.json", None),
            (
                StatePath::ServicesOverride,
                None,
                "services.json",
                Some("SEBAS_SERVICES_FILE"),
            ),
            // ── unify-sebas-home 新族（D2 布局）──
            (StatePath::ConfigDefault, None, "config.toml", None),
            (StatePath::CoreSecretDefault, None, "core.secret", None),
            (
                StatePath::ChannelSocket,
                None,
                "run/core.sock",
                Some("SEBAS_CORE_SOCKET"),
            ),
            (
                StatePath::ControlSocket,
                None,
                "run/control.sock",
                Some("SEBAS_CONTROL_SOCKET"),
            ),
            (StatePath::MediaDownloads, None, "cache/downloads", None),
            (StatePath::NodeStateDir, None, "node", Some("SEBAS_NODE_DIR")),
            (StatePath::UpgradeDataDir, None, "upgrade", None),
        ];
        for (path, db, rel, var) in want {
            assert_eq!(path.database(), *db, "{path:?} 所属库");
            assert_eq!(path.rel_path(), *rel, "{path:?} 相对路径");
            assert_eq!(path.override_var(), *var, "{path:?} 覆盖变量");
        }
        // 逐库覆盖变量两两不同（single-state-dir D8：四库各一个，互不串库）。
        let vars = [
            Database::Settings,
            Database::Projects,
            Database::Auth,
            Database::Usage,
        ]
        .map(|db| db.override_var());
        for (i, v) in vars.iter().enumerate() {
            for (j, w) in vars.iter().enumerate() {
                if i != j {
                    assert_ne!(v, w, "库 {v} 与 {w} 的覆盖变量不得相同");
                }
            }
        }
    }

    #[test]
    fn database_layering_follows_two_level_rule() {
        // 两级分层（写入进程 + core 内增长特征）：
        // providers/model_aliases/settings → settings.db（有界）；
        // projects/session_map → projects.db（增长）；auth.db 写入者是 webui、
        // usage.db 写入者是 router——都不属于 core。
        assert_eq!(Database::Settings.file_name(), "settings.db");
        assert_eq!(Database::Projects.file_name(), "projects.db");
        assert_ne!(Database::Settings, Database::Projects);
        assert_ne!(Database::Auth, Database::Settings);
        assert_ne!(Database::Usage, Database::Settings);
    }

    // ---- 任务 1.2：优先级（覆盖 > home > 默认）与正名反转 ----

    #[test]
    fn neither_home_nor_override_set_falls_back_to_default_home() {
        let _g = EnvGuard::clean();
        // 默认 sebas home = ~/.sebas（tilde 同源展开）。
        let home = sebas_home();
        assert!(home.ends_with(".sebas"), "默认 home 应是 ~/.sebas: {home:?}");
        for p in ALL {
            assert_eq!(
                p.resolve(),
                home.join(p.rel_path()),
                "{p:?} 默认落点 = home + 固定相对路径"
            );
        }
    }

    #[test]
    fn home_variable_alone_relocates_every_logical_name() {
        let _g = EnvGuard::clean();
        let pin = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var(HOME_VAR, pin.path()) };
        for p in ALL {
            let resolved = p.resolve();
            assert!(
                resolved.starts_with(pin.path()),
                "{p:?} 必须落在钉住的 home 内: {resolved:?}"
            );
        }
    }

    #[test]
    fn per_file_override_wins_over_home_derivation() {
        let _g = EnvGuard::clean();
        let pin = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var(HOME_VAR, pin.path()) };
        unsafe {
            std::env::set_var(
                "SEBAS_ARCHIVE_PATH",
                elsewhere.path().join("my-archive.json"),
            )
        };
        // 被覆盖的那一个文件去别处；其余照常在 home 内。
        assert_eq!(
            StatePath::Archive.resolve(),
            elsewhere.path().join("my-archive.json")
        );
        let other = StatePath::ProjectRegistry.resolve();
        assert!(other.starts_with(pin.path()), "{other:?}");
    }

    #[test]
    fn per_database_override_wins_and_others_stay_in_home() {
        let _g = EnvGuard::clean();
        let pin = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var(HOME_VAR, pin.path()) };
        unsafe {
            std::env::set_var("SEBAS_PROJECTS_DB", elsewhere.path().join("proj.db"))
        };
        assert_eq!(
            Database::Projects.resolve(),
            elsewhere.path().join("proj.db"),
            "逐库覆盖优先于 home 派生（spec「Environment override relocates the database」）"
        );
        let settings = Database::Settings.resolve();
        assert!(settings.starts_with(pin.path()), "{settings:?}");
        assert_eq!(settings, pin.path().join("settings.db"));
    }

    /// 收编落点的逐落点 env 覆盖（任务 1.1）：SEBAS_CORE_SOCKET /
    /// SEBAS_CONTROL_SOCKET / SEBAS_NODE_DIR 各自只改自己那一行，其余落点
    /// 照常在钉住的 home 内（spec「a per-file override wins over the derived
    /// path」）。
    #[test]
    fn collected_env_overrides_redirect_only_their_own_row() {
        let _g = EnvGuard::clean();
        let pin = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var(HOME_VAR, pin.path()) };
        unsafe {
            std::env::set_var("SEBAS_CORE_SOCKET", elsewhere.path().join("my.sock"));
            std::env::set_var("SEBAS_CONTROL_SOCKET", elsewhere.path().join("ctl.sock"));
            std::env::set_var("SEBAS_NODE_DIR", elsewhere.path().join("node-state"));
        };
        assert_eq!(
            StatePath::ChannelSocket.resolve(),
            elsewhere.path().join("my.sock")
        );
        assert_eq!(
            StatePath::ControlSocket.resolve(),
            elsewhere.path().join("ctl.sock")
        );
        assert_eq!(
            StatePath::NodeStateDir.resolve(),
            elsewhere.path().join("node-state")
        );
        // 其余落点不受牵连。
        for p in [
            StatePath::SettingsDb,
            StatePath::ConfigDefault,
            StatePath::MediaDownloads,
            StatePath::UpgradeDataDir,
        ] {
            let resolved = p.resolve();
            assert!(
                resolved.starts_with(pin.path()),
                "{p:?} 不得被别的逐落点覆盖牵走: {resolved:?}"
            );
        }
    }

    #[test]
    fn empty_override_value_is_ignored_like_unset() {
        let _g = EnvGuard::clean();
        let pin = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var(HOME_VAR, pin.path()) };
        unsafe { std::env::set_var("SEBAS_ARCHIVE_PATH", "   ") };
        assert_eq!(
            StatePath::Archive.resolve(),
            pin.path().join("archive.json"),
            "空串/空白覆盖视同未设置"
        );
        // 收编变量的空值同语义（消费点原有行为等价）。
        unsafe { std::env::set_var("SEBAS_CORE_SOCKET", "   ") };
        assert_eq!(
            StatePath::ChannelSocket.resolve(),
            pin.path().join("run/core.sock")
        );
    }

    // ---- 任务 1.3：sebas home 是纯环境变量，不依赖配置文件 ----

    #[test]
    fn home_resolves_from_env_alone_without_any_config_file() {
        let _g = EnvGuard::clean();
        let pin = tempfile::tempdir().unwrap();
        // 不存在任何 config.toml；只设变量即可解析（解析函数不触盘）。
        unsafe { std::env::set_var(HOME_VAR, pin.path().join("home")) };
        assert_eq!(sebas_home(), pin.path().join("home"));
        assert_eq!(
            StatePath::AuthDb.resolve(),
            pin.path().join("home").join("auth.db")
        );
    }

    /// 任务 1.2 三例之一：仅别名生效——行为与正名逐路径等价，且可被检出。
    #[test]
    fn legacy_alias_alone_is_honored_and_detectable() {
        let _g = EnvGuard::clean();
        let alias_dir = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var(LEGACY_STATE_DIR_VAR, alias_dir.path()) };
        // 仅别名：解析结果与设置 SEBAS_HOME 完全一致（spec「the legacy alias
        // still works but warns」）。
        assert_eq!(sebas_home(), alias_dir.path());
        for p in ALL {
            assert_eq!(
                p.resolve(),
                alias_dir.path().join(p.rel_path()),
                "{p:?} 经别名解析必须等价于正名"
            );
        }
        assert!(legacy_alias_set(), "仅别名生效必须可检出");
        assert!(!alias_conflict_set(), "仅别名不是冲突");
    }

    /// 任务 1.2 三例之二：同设冲突——SEBAS_HOME 赢（优先级反转，BREAKING），
    /// 且冲突可检出供启动 warn。
    #[test]
    fn both_set_home_wins_and_conflict_is_detectable() {
        let _g = EnvGuard::clean();
        let home_dir = tempfile::tempdir().unwrap();
        let alias_dir = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var(HOME_VAR, home_dir.path()) };
        unsafe { std::env::set_var(LEGACY_STATE_DIR_VAR, alias_dir.path()) };
        assert_eq!(sebas_home(), home_dir.path(), "正名必须赢");
        for p in ALL {
            let resolved = p.resolve();
            assert!(resolved.starts_with(home_dir.path()), "{resolved:?}");
            assert!(
                !resolved.starts_with(alias_dir.path()),
                "{p:?} 不得落在别名目录"
            );
        }
        assert!(alias_conflict_set(), "同设冲突必须可检出");
        assert!(!legacy_alias_set(), "冲突时不是「仅别名生效」");
    }

    /// 任务 1.2 三例之三：两者皆未设——无 warn 可检出。
    #[test]
    fn neither_set_produces_no_alias_notice() {
        let _g = EnvGuard::clean();
        assert!(!legacy_alias_set());
        assert!(!alias_conflict_set());
    }

    #[test]
    fn override_paths_expand_tilde() {
        let _g = EnvGuard::clean();
        let home = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("HOME", home.path()) };
        unsafe { std::env::set_var("SEBAS_SERVICES_FILE", "~/svc.json") };
        assert_eq!(
            StatePath::ServicesOverride.resolve(),
            home.path().join("svc.json"),
            "覆盖值展开 ~/ 前缀（state-store spec）"
        );
    }

    // ---- 任务 6.1：退休变量无效果 + 提示 ----

    #[test]
    fn retired_state_db_var_has_no_effect_on_any_resolution() {
        let _g = EnvGuard::clean();
        let pin = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var(HOME_VAR, pin.path()) };
        // 基线（退休变量未导出）。
        let without: Vec<PathBuf> = ALL.iter().map(|p| p.resolve()).collect();

        // 导出退休变量：行为与不导出时逐路径一致（spec「the retired
        // database variable has no effect」），绝无任何库开到它的路径上。
        unsafe {
            std::env::set_var(RETIRED_STATE_DB_VAR, pin.path().join("sebas.db"))
        };
        let with: Vec<PathBuf> = ALL.iter().map(|p| p.resolve()).collect();
        assert_eq!(with, without);
        assert!(!with.iter().any(|p| p.ends_with("sebas.db")));

        // 提示器检出残留值（启动日志据此点名）。
        assert_eq!(
            retired_env_vars_present(),
            vec![RETIRED_STATE_DB_VAR],
            "退休变量在场必须可被检出"
        );
        unsafe { std::env::remove_var(RETIRED_STATE_DB_VAR) };
        assert!(retired_env_vars_present().is_empty());
    }

    /// `migrate-project-registry` 6.1：`SEBAS_PROJECTS_PATH` 退休——导出它
    /// 与不导出行为完全一致（注册表落 `projects.db`，没有 projects.json 可
    /// 指向），且启动提示器会点名残留值。
    #[test]
    fn retired_projects_path_var_has_no_effect_on_any_resolution() {
        let _g = EnvGuard::clean();
        let pin = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var(HOME_VAR, pin.path()) };
        let without: Vec<PathBuf> = ALL.iter().map(|p| p.resolve()).collect();

        // 导出退休变量（指向一个别处的 projects.json）：逐路径与不导出一致。
        unsafe {
            std::env::set_var(
                RETIRED_PROJECTS_PATH_VAR,
                elsewhere.path().join("projects.json"),
            )
        };
        let with: Vec<PathBuf> = ALL.iter().map(|p| p.resolve()).collect();
        assert_eq!(with, without, "退休变量不得改变任何落点");
        assert!(
            !with.iter().any(|p| p.starts_with(elsewhere.path())),
            "退休变量不得把任何落点搬到它指的地方: {with:?}"
        );
        // 项目注册表逻辑名不再有覆盖入口，且落点仍在钉住的目录内。
        assert_eq!(StatePath::ProjectRegistry.override_var(), None);
        assert_eq!(
            StatePath::ProjectRegistry.resolve(),
            pin.path().join("projects.json")
        );

        // 提示器检出残留值（启动日志据此点名）。
        assert_eq!(
            retired_env_vars_present(),
            vec![RETIRED_PROJECTS_PATH_VAR],
            "退休变量在场必须可被检出"
        );
        unsafe { std::env::remove_var(RETIRED_PROJECTS_PATH_VAR) };
        assert!(retired_env_vars_present().is_empty());
    }

    // ---- 任务 1.3：派生覆盖断言（枚举全部逻辑名，钉住的目录全覆盖）----
    //
    // 失败演示：把任一逻辑名的 rel_path 改回硬编码 `~/.sebas/...`（例如让
    // NodeRegistry.resolve() 无视主目录直接返回 `$HOME/.sebas/nodes.json`），
    // 本测试立即红——这正是「目录钉不住」缺陷的机械护栏。
    #[test]
    fn derivation_covers_every_logical_name_inside_pinned_dir() {
        let _g = EnvGuard::clean();
        let pin = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var(HOME_VAR, pin.path()) };
        for p in ALL {
            let resolved = p.resolve();
            assert!(
                resolved.starts_with(pin.path()),
                "{p:?} 必须落在钉住的目录内: {resolved:?}"
            );
            // 派生形态（无视覆盖）同样在目录内，且就是目录 + 固定相对路径。
            assert_eq!(p.derived_in(pin.path()), pin.path().join(p.rel_path()));
        }
    }

    // ---- 任务 1.3：默认收敛断言（不设任何变量时全部在默认 home 下）----

    #[test]
    fn defaults_converge_under_the_default_home() {
        let _g = EnvGuard::clean();
        let home = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("HOME", home.path()) };
        let expected = home.path().join(".sebas");
        // 期望清单：平铺名册原位不动（零迁移）+ 新族子目录（design D2）。
        let want: &[(&str, &str)] = &[
            ("settings.db", "settings.db"),
            ("projects.db", "projects.db"),
            ("auth.db", "auth.db"),
            ("usage.db", "usage.db"),
            ("archive.json", "archive.json"),
            ("projects.json", "projects.json"),
            ("nodes.json", "nodes.json"),
            ("services.json", "services.json"),
            ("config.toml", "config.toml"),
            ("core.secret", "core.secret"),
            ("core.sock", "run/core.sock"),
            ("control.sock", "run/control.sock"),
            ("downloads", "cache/downloads"),
            ("node", "node"),
            ("upgrade", "upgrade"),
        ];
        assert_eq!(want.len(), ALL.len(), "期望清单与枚举必须一一对应");
        for (label, rel) in want {
            let p = ALL
                .iter()
                .find(|p| p.rel_path() == *rel)
                .unwrap_or_else(|| panic!("逻辑名缺失: {label}"));
            let resolved = p.resolve();
            assert_eq!(
                resolved,
                expected.join(rel),
                "{label} 默认落点必须在默认 sebas home 下"
            );
        }
    }

    // ---- 任务 1.1：config 缺省解析器 ----

    #[test]
    fn default_config_path_follows_the_sebas_home() {
        let _g = EnvGuard::clean();
        let pin = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var(HOME_VAR, pin.path()) };
        assert_eq!(
            default_config_path(),
            pin.path().join("config.toml"),
            "-c 缺席时 config 缺省 = <home>/config.toml（D3）"
        );
        // 无逐落点 env 入口：显式 -c 是消费点特例（design D6）。
        assert_eq!(StatePath::ConfigDefault.override_var(), None);
        assert_eq!(StatePath::CoreSecretDefault.override_var(), None);
    }
}
