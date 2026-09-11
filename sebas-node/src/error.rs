//! 节点侧错误。每一类都带**可读成因**——沿用仓库既有的诚实口径：可达性/可用性
//! 问题一律如实回错并指名原因，不伪装受理、不静默降级。
//!
//! 启动阶段返回任何 `NodeError` 都意味着「未能达到 ready」，由二进制入口走
//! `sebas_startup::exit_startup_failure`（stderr 末行摘要 + EX_TEMPFAIL 75）。

/// 节点 fatal 错误。
#[derive(Debug, thiserror::Error)]
pub enum NodeError {
    /// 配置不可用：文件解析失败、字段非法、必填缺失。
    #[error("配置错误：{cause}")]
    Config {
        /// 人类可读的成因（含具体字段名与取值）。
        cause: String,
    },

    /// 节点未配对：状态目录里没有可用凭据。
    #[error("未配对：{cause}")]
    Unpaired {
        /// 成因与下一步（如何配对）。
        cause: String,
    },

    /// 状态目录不可用（创建、读写、权限失败）。
    #[error("状态目录不可用：{cause}")]
    StateDir {
        /// 成因（含路径）。
        cause: String,
    },

    /// 控制面链路不可用。
    #[error("控制面链路不可用：{cause}")]
    LinkUnavailable {
        /// 成因（含端点或协议版本信息）。
        cause: String,
    },
}

impl NodeError {
    /// `Config` 变体的便捷构造。
    pub fn config(cause: impl Into<String>) -> Self {
        Self::Config {
            cause: cause.into(),
        }
    }

    /// `Unpaired` 变体的便捷构造。
    pub fn unpaired(cause: impl Into<String>) -> Self {
        Self::Unpaired {
            cause: cause.into(),
        }
    }

    /// `StateDir` 变体的便捷构造。
    pub fn state_dir(cause: impl Into<String>) -> Self {
        Self::StateDir {
            cause: cause.into(),
        }
    }

    /// `LinkUnavailable` 变体的便捷构造。
    pub fn link_unavailable(cause: impl Into<String>) -> Self {
        Self::LinkUnavailable {
            cause: cause.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_name_their_cause() {
        assert_eq!(
            NodeError::config("未知字段 `nod`").to_string(),
            "配置错误：未知字段 `nod`"
        );
        assert_eq!(
            NodeError::unpaired("节点上没有凭据").to_string(),
            "未配对：节点上没有凭据"
        );
        assert_eq!(
            NodeError::state_dir("/tmp/x 不可写").to_string(),
            "状态目录不可用：/tmp/x 不可写"
        );
        assert_eq!(
            NodeError::link_unavailable("wss://c.example 不可达").to_string(),
            "控制面链路不可用：wss://c.example 不可达"
        );
    }
}
