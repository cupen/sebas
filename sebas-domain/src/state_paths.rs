//! 状态路径映射表（single-state-dir D1/D7/D8）：**逻辑名 → 所属库 → 文件名
//! → 覆盖变量**，全部落点从单一状态目录派生的唯一规则表。
//!
//! # 解析优先级（design D1）
//!
//! ```text
//! 逐文件显式覆盖（env）> 状态目录（env SEBAS_STATE_DIR）> 默认（~/.sebas）
//! ```
//!
//!
//! 既有部署与沙箱菜谱的逐文件变量（`SEBAS_ARCHIVE_PATH` 等）保持原语义；
//! 目录变量是新入口，未设置时行为确定。被否备选：目录变量压过逐文件变量
//! （会让既有设置静默失效）。
//!
//! # 默认状态目录
//!
//! `SEBAS_STATE_DIR`，其次 `SEBAS_HOME`（legacy 目录变量，语义就是「状态
//! 目录」），最后默认 `~/.sebas`（`dirs::home_dir()`，与 `expand_tilde`
//! 同源）。解析只读环境变量，**不依赖任何配置文件**——沙箱在写
//! config.toml 之前就能钉住全部落点。
//!
//! # 分层规则（design D2，两级）
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

/// 状态目录环境变量（single-state-dir：单一变量派生全部落点）。
pub const STATE_DIR_VAR: &str = "SEBAS_STATE_DIR";

/// legacy 目录变量：语义与 `SEBAS_STATE_DIR` 相同（既有沙箱菜谱用它钉住
/// `~/.sebas` 下的全部默认落点）。保留兼容，优先级低于 `SEBAS_STATE_DIR`。
pub const LEGACY_HOME_VAR: &str = "SEBAS_HOME";

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
    /// 库文件名（固定落在状态目录下）。
    pub fn file_name(self) -> &'static str {
        match self {
            Database::Settings => "settings.db",
            Database::Projects => "projects.db",
            Database::Auth => "auth.db",
            Database::Usage => "usage.db",
        }
    }

    /// 逐库显式覆盖变量（design D8：`SEBAS_STATE_DB` 由四个逐库变量取代）。
    pub fn override_var(self) -> &'static str {
        match self {
            Database::Settings => "SEBAS_SETTINGS_DB",
            Database::Projects => "SEBAS_PROJECTS_DB",
            // 既有变量名原样保留（add-webui-multiuser-rbac 起就是它）。
            Database::Auth => "SEBAS_WEBUI_AUTH_DB",
            Database::Usage => "SEBAS_ROUTER_USAGE_DB",
        }
    }

    /// 库路径：逐库覆盖（tilde 展开）> 状态目录派生。
    pub fn resolve(self) -> PathBuf {
        override_path(self.override_var()).unwrap_or_else(|| state_dir().join(self.file_name()))
    }
}

/// 状态落点逻辑名。每个变体 = 一条映射表行：所属库（文件类为 `None`）、
/// 固定文件名、逐文件覆盖变量。
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
    /// 文件——一个文件一个写入者，watchdog 自己写，design D6）。
    ServicesOverride,
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
            | StatePath::ServicesOverride => None,
        }
    }

    /// 固定文件名（派生形态 = 状态目录 + 本名字）。
    pub fn file_name(self) -> &'static str {
        match self {
            StatePath::SettingsDb => Database::Settings.file_name(),
            StatePath::ProjectsDb => Database::Projects.file_name(),
            StatePath::AuthDb => Database::Auth.file_name(),
            StatePath::UsageDb => Database::Usage.file_name(),
            StatePath::Archive => "archive.json",
            StatePath::ProjectRegistry => "projects.json",
            StatePath::NodeRegistry => "nodes.json",
            StatePath::ServicesOverride => "services.json",
        }
    }

    /// 逐文件覆盖变量；`None` = 无环境变量入口——显式覆盖走配置键
    /// （[`StatePath::NodeRegistry`] 的 `[node_link] registry_file`），或该
    /// 覆盖变量已退休（[`StatePath::ProjectRegistry`]）。
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
        }
    }

    /// 在给定状态目录下的派生路径（不经任何覆盖——机械断言用）。
    pub fn derived_in(self, dir: &std::path::Path) -> PathBuf {
        dir.join(self.file_name())
    }

    /// 解析落点：逐文件覆盖（tilde 展开）> 状态目录派生（design D1）。
    pub fn resolve(self) -> PathBuf {
        match self.override_var() {
            Some(var) => override_path(var).unwrap_or_else(|| state_dir().join(self.file_name())),
            None => state_dir().join(self.file_name()),
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

/// 状态目录：`SEBAS_STATE_DIR` > `SEBAS_HOME` > `~/.sebas`。纯环境变量解析，
/// 不依赖配置文件（single-state-dir 任务 1.3）。
pub fn state_dir() -> PathBuf {
    for var in [STATE_DIR_VAR, LEGACY_HOME_VAR] {
        if let Ok(v) = std::env::var(var)
            && !v.trim().is_empty()
        {
            return PathBuf::from(crate::prim::expand_tilde(&v));
        }
    }
    crate::prim::expand_tilde("~/.sebas").into()
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
                STATE_DIR_VAR,
                LEGACY_HOME_VAR,
                RETIRED_STATE_DB_VAR,
                "SEBAS_SETTINGS_DB",
                "SEBAS_PROJECTS_DB",
                "SEBAS_WEBUI_AUTH_DB",
                "SEBAS_ROUTER_USAGE_DB",
                "SEBAS_ARCHIVE_PATH",
                RETIRED_PROJECTS_PATH_VAR,
                "SEBAS_SERVICES_FILE",
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

    // ---- 任务 1.1：全部逻辑名的默认值、所属库与覆盖变量名 ----

    #[test]
    fn every_logical_name_maps_to_database_file_and_override() {
        // (逻辑名, 所属库, 文件名, 覆盖变量)
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
            (
                StatePath::Archive,
                None,
                "archive.json",
                Some("SEBAS_ARCHIVE_PATH"),
            ),
            (
                StatePath::ProjectRegistry,
                None,
                "projects.json",
                None,
            ),
            // 节点注册表的显式覆盖是配置键 [node_link] registry_file，
            // 不设环境变量（优先级不变，任务 4.2）。
            (StatePath::NodeRegistry, None, "nodes.json", None),
            (
                StatePath::ServicesOverride,
                None,
                "services.json",
                Some("SEBAS_SERVICES_FILE"),
            ),
        ];
        for (path, db, file, var) in want {
            assert_eq!(path.database(), *db, "{path:?} 所属库");
            assert_eq!(path.file_name(), *file, "{path:?} 文件名");
            assert_eq!(path.override_var(), *var, "{path:?} 覆盖变量");
        }
        // 逐库覆盖变量两两不同（design D8：四库各一个，互不串库）。
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
        // D2 第一级（写入进程）+ 第二级（core 内增长特征）：
        // providers/model_aliases/settings → settings.db（有界）；
        // projects/session_map → projects.db（增长）；auth.db 写入者是 webui、
        // usage.db 写入者是 router——都不属于 core。
        assert_eq!(Database::Settings.file_name(), "settings.db");
        assert_eq!(Database::Projects.file_name(), "projects.db");
        assert_ne!(Database::Settings, Database::Projects);
        assert_ne!(Database::Auth, Database::Settings);
        assert_ne!(Database::Usage, Database::Settings);
    }

    // ---- 任务 1.2：优先级（覆盖 > 目录 > 默认）三组合各一条 ----

    #[test]
    fn neither_dir_nor_override_set_falls_back_to_default_state_dir() {
        let _g = EnvGuard::clean();
        // 默认状态目录 = ~/.sebas（tilde 同源展开）。
        let dir = state_dir();
        assert!(
            dir.ends_with(".sebas"),
            "默认状态目录应是 ~/.sebas: {dir:?}"
        );
        for p in [
            StatePath::SettingsDb,
            StatePath::ProjectsDb,
            StatePath::AuthDb,
            StatePath::UsageDb,
            StatePath::Archive,
            StatePath::ProjectRegistry,
            StatePath::NodeRegistry,
            StatePath::ServicesOverride,
        ] {
            assert_eq!(
                p.resolve(),
                dir.join(p.file_name()),
                "{p:?} 默认落点 = 状态目录 + 固定文件名"
            );
        }
    }

    #[test]
    fn dir_variable_alone_relocates_every_logical_name() {
        let _g = EnvGuard::clean();
        let pin = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var(STATE_DIR_VAR, pin.path()) };
        for p in [
            StatePath::SettingsDb,
            StatePath::ProjectsDb,
            StatePath::AuthDb,
            StatePath::UsageDb,
            StatePath::Archive,
            StatePath::ProjectRegistry,
            StatePath::NodeRegistry,
            StatePath::ServicesOverride,
        ] {
            let resolved = p.resolve();
            assert!(
                resolved.starts_with(pin.path()),
                "{p:?} 必须落在钉住的目录内: {resolved:?}"
            );
        }
    }

    #[test]
    fn per_file_override_wins_over_dir_derivation() {
        let _g = EnvGuard::clean();
        let pin = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var(STATE_DIR_VAR, pin.path()) };
        unsafe {
            std::env::set_var(
                "SEBAS_ARCHIVE_PATH",
                elsewhere.path().join("my-archive.json"),
            )
        };
        // 被覆盖的那一个文件去别处；其余照常在目录内。
        assert_eq!(
            StatePath::Archive.resolve(),
            elsewhere.path().join("my-archive.json")
        );
        let other = StatePath::ProjectRegistry.resolve();
        assert!(other.starts_with(pin.path()), "{other:?}");
    }

    #[test]
    fn per_database_override_wins_and_others_stay_in_dir() {
        let _g = EnvGuard::clean();
        let pin = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var(STATE_DIR_VAR, pin.path()) };
        unsafe {
            std::env::set_var("SEBAS_PROJECTS_DB", elsewhere.path().join("proj.db"))
        };
        assert_eq!(
            Database::Projects.resolve(),
            elsewhere.path().join("proj.db"),
            "逐库覆盖优先于目录派生（spec「Environment override relocates the database」）"
        );
        let settings = Database::Settings.resolve();
        assert!(settings.starts_with(pin.path()), "{settings:?}");
        assert_eq!(settings, pin.path().join("settings.db"));
    }

    #[test]
    fn empty_override_value_is_ignored_like_unset() {
        let _g = EnvGuard::clean();
        let pin = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var(STATE_DIR_VAR, pin.path()) };
        unsafe { std::env::set_var("SEBAS_ARCHIVE_PATH", "   ") };
        assert_eq!(
            StatePath::Archive.resolve(),
            pin.path().join("archive.json"),
            "空串/空白覆盖视同未设置"
        );
    }

    // ---- 任务 1.3：状态目录是纯环境变量，不依赖配置文件 ----

    #[test]
    fn state_dir_resolves_from_env_alone_without_any_config_file() {
        let _g = EnvGuard::clean();
        let pin = tempfile::tempdir().unwrap();
        // 不存在任何 config.toml；只设变量即可解析（解析函数不触盘）。
        unsafe { std::env::set_var(STATE_DIR_VAR, pin.path().join("state")) };
        assert_eq!(state_dir(), pin.path().join("state"));
        assert_eq!(
            StatePath::AuthDb.resolve(),
            pin.path().join("state").join("auth.db")
        );
    }

    #[test]
    fn legacy_home_var_still_works_as_dir_alias_but_dir_var_wins() {
        let _g = EnvGuard::clean();
        let legacy = tempfile::tempdir().unwrap();
        let newer = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var(LEGACY_HOME_VAR, legacy.path()) };
        assert_eq!(state_dir(), legacy.path(), "SEBAS_HOME 保持兼容语义");
        unsafe { std::env::set_var(STATE_DIR_VAR, newer.path()) };
        assert_eq!(state_dir(), newer.path(), "SEBAS_STATE_DIR 优先于 SEBAS_HOME");
        let projects = StatePath::ProjectRegistry.resolve();
        assert_eq!(projects, newer.path().join("projects.json"));
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
        unsafe { std::env::set_var(STATE_DIR_VAR, pin.path()) };
        // 基线（退休变量未导出）。
        let without: Vec<PathBuf> = [
            StatePath::SettingsDb,
            StatePath::ProjectsDb,
            StatePath::AuthDb,
            StatePath::UsageDb,
            StatePath::Archive,
            StatePath::ProjectRegistry,
            StatePath::NodeRegistry,
            StatePath::ServicesOverride,
        ]
        .iter()
        .map(|p| p.resolve())
        .collect();

        // 导出退休变量：行为与不导出时逐路径一致（spec「the retired
        // database variable has no effect」），绝无任何库开到它的路径上。
        unsafe {
            std::env::set_var(RETIRED_STATE_DB_VAR, pin.path().join("sebas.db"))
        };
        let with: Vec<PathBuf> = [
            StatePath::SettingsDb,
            StatePath::ProjectsDb,
            StatePath::AuthDb,
            StatePath::UsageDb,
            StatePath::Archive,
            StatePath::ProjectRegistry,
            StatePath::NodeRegistry,
            StatePath::ServicesOverride,
        ]
        .iter()
        .map(|p| p.resolve())
        .collect();
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
        unsafe { std::env::set_var(STATE_DIR_VAR, pin.path()) };
        let all = [
            StatePath::SettingsDb,
            StatePath::ProjectsDb,
            StatePath::AuthDb,
            StatePath::UsageDb,
            StatePath::Archive,
            StatePath::ProjectRegistry,
            StatePath::NodeRegistry,
            StatePath::ServicesOverride,
        ];
        let without: Vec<PathBuf> = all.iter().map(|p| p.resolve()).collect();

        // 导出退休变量（指向一个别处的 projects.json）：逐路径与不导出一致。
        unsafe {
            std::env::set_var(
                RETIRED_PROJECTS_PATH_VAR,
                elsewhere.path().join("projects.json"),
            )
        };
        let with: Vec<PathBuf> = all.iter().map(|p| p.resolve()).collect();
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

    // ---- 任务 6.2：派生覆盖断言（枚举全部逻辑名，钉住的目录全覆盖）----
    //
    // 失败演示（任务 6.2 验收附注）：把任一逻辑名的 file_name 改回硬编码
    // `~/.sebas/...`（例如让 NodeRegistry.resolve() 无视状态目录直接返回
    // `$HOME/.sebas/nodes.json`），本测试立即红：
    //   panicked at ... NodeRegistry 必须落在钉住的目录内:
    //   "/home/<user>/.sebas/nodes.json"
    // ——这正是本 change 修掉的那类「目录钉不住」缺陷的机械护栏。
    #[test]
    fn derivation_covers_every_logical_name_inside_pinned_dir() {
        let _g = EnvGuard::clean();
        let pin = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var(STATE_DIR_VAR, pin.path()) };
        for p in [
            StatePath::SettingsDb,
            StatePath::ProjectsDb,
            StatePath::AuthDb,
            StatePath::UsageDb,
            StatePath::Archive,
            StatePath::ProjectRegistry,
            StatePath::NodeRegistry,
            StatePath::ServicesOverride,
        ] {
            let resolved = p.resolve();
            assert!(
                resolved.starts_with(pin.path()),
                "{p:?} 必须落在钉住的目录内: {resolved:?}"
            );
            // 派生形态（无视覆盖）同样在目录内，且就是目录 + 固定文件名。
            assert_eq!(p.derived_in(pin.path()), pin.path().join(p.file_name()));
        }
    }

    // ---- 任务 4.3：默认收敛断言（不设任何变量时全部在默认状态目录下）----

    #[test]
    fn defaults_converge_under_the_default_state_dir() {
        let _g = EnvGuard::clean();
        let home = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("HOME", home.path()) };
        let expected_dir = home.path().join(".sebas");
        // 期望清单（任务 4.3：来自改造前实测 + nodes.json 新位置）：
        // - settings.db / projects.db / auth.db / usage.db —— 拆库新落点；
        // - archive.json / projects.json / services.json —— 改造前默认已在
        //   ~/.sebas，落点逐字不变；
        // - nodes.json —— 唯一发生迁移的落点（配置目录 → 状态目录）。
        let want: &[(&str, &str)] = &[
            ("settings.db", "settings.db"),
            ("projects.db", "projects.db"),
            ("auth.db", "auth.db"),
            ("usage.db", "usage.db"),
            ("archive.json", "archive.json"),
            ("projects.json", "projects.json"),
            ("nodes.json", "nodes.json"),
            ("services.json", "services.json"),
        ];
        let all = [
            StatePath::SettingsDb,
            StatePath::ProjectsDb,
            StatePath::AuthDb,
            StatePath::UsageDb,
            StatePath::Archive,
            StatePath::ProjectRegistry,
            StatePath::NodeRegistry,
            StatePath::ServicesOverride,
        ];
        for (label, file) in want {
            let p = all
                .iter()
                .find(|p| p.file_name() == *file)
                .unwrap_or_else(|| panic!("逻辑名缺失: {label}"));
            let resolved = p.resolve();
            assert_eq!(
                resolved,
                expected_dir.join(file),
                "{label} 默认落点必须在默认状态目录下"
            );
        }
    }
}
