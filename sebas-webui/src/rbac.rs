//! RBAC 角色与权限模型（add-webui-multiuser-rbac 1.4）。
//!
//! 角色集固定四档（root / admin / member / viewer），角色→权限映射固定在
//! 代码里，不可经 API 修改（design D3）。[`Role::permissions`] 是 spec
//! 「RBAC 角色与权限执法」权限表的代码化事实源；路由层的中央表
//! （后续批次的 `required_permission(path, method)`）按 [`Permission`]
//! 执法，「只读」不设权限词——认证即得（spec 权限表最后一行）。

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// 固定角色集。词表与用户库 `users.role` 列的 CHECK 约束、wire 上的
/// JSON 字符串一致（小写）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// 超管：用户管理 + 全部能力。
    Root,
    /// 管理员：系统设置 + 服务控制 + 会话写，无用户管理。
    Admin,
    /// 成员：会话与项目写。
    Member,
    /// 只读访客：仅工作台读面 + `/ws` 事件流。
    Viewer,
}

/// 可执法的权限词（spec deltas 命名四个；「只读」是认证基线，不设词）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Permission {
    /// 用户管理（`/api/users*`）。
    UsersManage,
    /// 系统设置写（卡片/显示偏好，`POST /api/settings`）。
    SettingsManage,
    /// 服务控制（watchdog 服务启停/升级/回滚，`/api/admin/*` 类）。
    ServicesControl,
    /// 会话与项目写（创建/发消息/归档/恢复/项目增删）。
    SessionsWrite,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Root => "root",
            Role::Admin => "admin",
            Role::Member => "member",
            Role::Viewer => "viewer",
        }
    }

    /// 角色→权限映射（spec 权限表的代码化，唯一事实源）：
    ///
    /// | 权限 | root | admin | member | viewer |
    /// |---|---|---|---|---|
    /// | users.manage | ✓ | | | |
    /// | settings.manage | ✓ | ✓ | | |
    /// | services.control | ✓ | ✓ | | |
    /// | sessions.write | ✓ | ✓ | ✓ | |
    pub fn permissions(&self) -> &'static [Permission] {
        match self {
            Role::Root => &[
                Permission::UsersManage,
                Permission::SettingsManage,
                Permission::ServicesControl,
                Permission::SessionsWrite,
            ],
            Role::Admin => &[
                Permission::SettingsManage,
                Permission::ServicesControl,
                Permission::SessionsWrite,
            ],
            Role::Member => &[Permission::SessionsWrite],
            Role::Viewer => &[],
        }
    }

    /// 该角色是否持有某权限。
    pub fn has(&self, permission: Permission) -> bool {
        self.permissions().contains(&permission)
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 未知角色字符串（词表外或大小写不符）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownRole(pub String);

impl fmt::Display for UnknownRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "未知角色 {:?}（合法词表 root/admin/member/viewer）", self.0)
    }
}

impl std::error::Error for UnknownRole {}

impl FromStr for Role {
    type Err = UnknownRole;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "root" => Ok(Role::Root),
            "admin" => Ok(Role::Admin),
            "member" => Ok(Role::Member),
            "viewer" => Ok(Role::Viewer),
            other => Err(UnknownRole(other.to_string())),
        }
    }
}

// ─── 测试 ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// spec 权限表逐格钉死：改动映射必须显式过这里（并同步 spec delta）。
    #[test]
    fn permission_matrix_matches_spec() {
        let p = |r: Role, perm: Permission| r.has(perm);
        // users.manage：仅 root。
        assert!(p(Role::Root, Permission::UsersManage));
        for r in [Role::Admin, Role::Member, Role::Viewer] {
            assert!(!p(r, Permission::UsersManage), "{r} 不应持有 users.manage");
        }
        // settings.manage / services.control：root + admin。
        for perm in [Permission::SettingsManage, Permission::ServicesControl] {
            assert!(p(Role::Root, perm));
            assert!(p(Role::Admin, perm));
            assert!(!p(Role::Member, perm), "member 不应持有 {perm:?}");
            assert!(!p(Role::Viewer, perm), "viewer 不应持有 {perm:?}");
        }
        // sessions.write：root + admin + member。
        for r in [Role::Root, Role::Admin, Role::Member] {
            assert!(p(r, Permission::SessionsWrite), "{r} 应持有 sessions.write");
        }
        assert!(!p(Role::Viewer, Permission::SessionsWrite));
        // 权限集单调不减（root ⊇ admin ⊇ member ⊇ viewer），防映射漂移。
        assert_eq!(Role::Root.permissions().len(), 4);
        assert_eq!(Role::Admin.permissions().len(), 3);
        assert_eq!(Role::Member.permissions().len(), 1);
        assert_eq!(Role::Viewer.permissions().len(), 0);
    }

    #[test]
    fn role_parse_accepts_canonical_words() {
        for (word, role) in [
            ("root", Role::Root),
            ("admin", Role::Admin),
            ("member", Role::Member),
            ("viewer", Role::Viewer),
        ] {
            assert_eq!(word.parse::<Role>().unwrap(), role);
            assert_eq!(role.to_string(), word);
        }
    }

    /// 未知角色字符串解析拒绝：词表外、大小写不符、空串都不得静默成默认角色。
    #[test]
    fn role_parse_rejects_unknown_words() {
        for bad in ["superadmin", "ROOT", "Admin", "", "root ", "viewer,"] {
            let err = bad.parse::<Role>().unwrap_err();
            assert_eq!(err.0, bad, "错误信息必须回带原始串");
        }
    }

    /// wire 形状钉死：Role 的 serde 词表与 DB CHECK / CLI 一致（小写），
    /// 未知词反序列化报错（与 FromStr 同姿态）。
    #[test]
    fn role_serde_round_trip_is_lowercase() {
        let json = serde_json::to_value(Role::Root).unwrap();
        assert_eq!(json, serde_json::json!("root"));
        for role in [Role::Root, Role::Admin, Role::Member, Role::Viewer] {
            let word = serde_json::to_value(role).unwrap();
            assert_eq!(serde_json::from_value::<Role>(word).unwrap(), role);
        }
        assert!(serde_json::from_value::<Role>(serde_json::json!("boss")).is_err());
    }
}
