//! pi provider 桥：把 sebas 的 provider 注册表映射成 pi 的 `models.json`。
//!
//! **为什么需要桥**（make-pi-ride-sebas-provider，design D1–D4）：pi 的内置
//! Anthropic 模型把 `baseUrl` 写死为 `https://api.anthropic.com`，
//! `createClient` 显式传 `baseURL: model.baseUrl` 压掉 SDK 的 env 默认值——
//! 所以 `ANTHROPIC_BASE_URL` 对 pi 的端点重定向**无效**（实测：设到假上游
//! 零命中，pi 仍拨 api.anthropic.com）。pi 只认自己的配置面：
//!
//! 1. `models.json` 的 provider 级 `baseUrl` 覆盖（保留 pi 内置模型目录，
//!    只改端点）——数据面，本模块采用（D1）；
//! 2. extension `pi.registerProvider({baseUrl, apiKey})`——代码面，本 change
//!    不采用（过度设计，见 design D1 的否决理由）。
//!
//! **落点与所有权**（D2）：桥文件写在 sebas **自己**的目录
//! `<SEBAS_HOME>/pi-agent/models.json`（[`StatePath::PiAgentDir`]），spawn pi
//! 时把 `PI_CODING_AGENT_DIR` 指向它。操作员的 `~/.pi/agent` 不被读写——
//! 「一个文件一个写入者」在桥文件这条路径上由「sebas 写自己目录」承担。
//!
//! **密钥传输**（D4）：`models.json` 的 `apiKey` 写 `${SEBAS_PI_KEY_n}` 占位，
//! 真值只在 spawn 期经 env 注入（`SEBAS_PI_KEY_0=<key>`），pi 的解析阶段读
//! env、真值**不落盘**。绝不把密钥塞进 argv（`/proc/*/cmdline` 可见）或
//! `ANTHROPIC_*`（claude 家族 env 名，错配且对 pi 端点重定向无效）。
//!
//! **格式**（D3，实测确认）：只写 provider 级 `baseUrl` 与 `apiKey`，**不**列
//! `models`——pi 文档明说「只注册 baseUrl/headers 保留内置模型」，实测
//! 17 个内置 claude 模型 / 44 个内置 openai 模型全部保留，端点改为新值。
//! 反例：没有 `models.json` 时 pi 的目录是**空**的（实测 0 个模型），所以
//! 桥文件不只是重定向、更是「激活内置目录」的开关。
//!
//! [`StatePath::PiAgentDir`]: sebas_domain::state_paths::StatePath::PiAgentDir

use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// 桥文件在 pi agent 目录里的固定名（pi 的约定）。
pub const MODELS_JSON: &str = "models.json";

/// sebas 私有 pi agent 目录的落点（`<SEBAS_HOME>/pi-agent/`，D2）。
///
/// 走 sebas home 映射表（[`StatePath::PiAgentDir`]）——钉住 `SEBAS_HOME` 即钉住
/// 它，sandbox 里随 home 派生。
///
/// [`StatePath::PiAgentDir`]: sebas_domain::state_paths::StatePath::PiAgentDir
pub fn pi_agent_dir() -> PathBuf {
    sebas_domain::state_paths::StatePath::PiAgentDir.resolve()
}

/// sebas 注入 pi 子进程的密钥 env 名前缀（`SEBAS_PI_KEY_0` / `_1` / …）。
pub const PI_KEY_ENV_PREFIX: &str = "SEBAS_PI_KEY_";

/// pi 的 provider 目录名（与 `models.json` 的 `providers.<name>` 键一致）。
const PI_PROVIDER_ANTHROPIC: &str = "anthropic";
const PI_PROVIDER_OPENAI: &str = "openai";

/// 一条已解析的 provider 桥条目：pi 侧 provider 名 + 端点 + 密钥占位。
///
/// 密钥字段存的是 `apiKey` 的**字面量**（`${SEBAS_PI_KEY_n}` 占位），真值
/// 不在这里——见模块文档 D4。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeEntry {
    /// pi 侧 provider 名（`"anthropic"` / `"openai"`）。
    pub provider: String,
    /// provider 级 baseUrl（来自 sebas provider 的 `base_url_*`）。
    pub base_url: String,
    /// `apiKey` 占位（`${SEBAS_PI_KEY_n}`）。
    pub api_key_placeholder: String,
    /// 该占位对应的 env 名（`SEBAS_PI_KEY_n`）——spawn 期注入真值用。
    pub key_env: String,
    /// 密钥真值（仅 spawn 期在内存里存在，**不**进 `models.json`）。
    pub key_value: String,
    /// 该条目来自的 sebas provider 名（诊断/测试用）。
    pub source_provider: String,
}

/// 一个已解析的 provider（协议 + 端点 + 密钥），供桥映射消费。
///
/// 这是 [`crate::spawn_env::compute_pi_provider_env`] 的替代形态：不再翻成
/// `ANTHROPIC_*` env 对，而是交给 [`render_models_json`] 映射成 pi 的
/// `models.json` 块。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedProvider {
    /// sebas provider 名（诊断用）。
    pub name: String,
    /// 协议家族：`"anthropic"` 或 `"openai"`。
    pub family: ProviderFamily,
    /// 端点（`base_url_anthropic` 或 `base_url_openai_chat`）。
    pub base_url: String,
    /// 密钥真值（spawn 期注入 env；不落盘）。
    pub api_key: String,
}

/// provider 协议家族（决定映射到 pi 的哪个 provider 块）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderFamily {
    Anthropic,
    OpenAi,
}

impl ProviderFamily {
    /// pi 侧 provider 名（`models.json` 的 `providers.<name>` 键）。
    pub fn pi_provider_name(self) -> &'static str {
        match self {
            ProviderFamily::Anthropic => PI_PROVIDER_ANTHROPIC,
            ProviderFamily::OpenAi => PI_PROVIDER_OPENAI,
        }
    }
}

/// 把 sebas provider 列表映射成 pi 桥条目（**同族默认赢**，D3）。
///
/// 规则：
/// - 每个协议家族（anthropic / openai）**最多一条**——pi 的 `models.json` 同
///   名 provider 块会互相覆盖，多写无意义且不确定；
/// - `default_provider` 若命中某族，该族归它；其余同族 provider 跳过并记入
///   `skipped`（诊断面）；
/// - `default_provider` 未命中某族时，该族取列表里**第一个**（调用方应已按
///   确定性顺序排好：默认 provider 优先，其余字典序）。
///
/// 返回 `(entries, skipped)`：`entries` 按 provider 名排序（确定性）；`skipped`
/// 是逐条跳过原因。
pub fn bridge_entries(
    providers: &[ResolvedProvider],
    default_provider: Option<&str>,
) -> (Vec<BridgeEntry>, Vec<String>) {
    let mut skipped: Vec<String> = Vec::new();
    let mut chosen: Vec<ResolvedProvider> = Vec::new();

    for family in [ProviderFamily::Anthropic, ProviderFamily::OpenAi] {
        let same_family: Vec<&ResolvedProvider> =
            providers.iter().filter(|p| p.family == family).collect();
        if same_family.is_empty() {
            continue;
        }
        // 默认 provider 若命中本族 → 赢；否则取第一个。
        let winner = default_provider
            .and_then(|d| same_family.iter().find(|p| p.name == d).copied())
            .unwrap_or(same_family[0]);
        for p in &same_family {
            if p.name != winner.name {
                skipped.push(format!(
                    "provider '{}' 与已占用的 {} 族同族，默认 provider '{}' 优先，已跳过",
                    p.name,
                    family.pi_provider_name(),
                    winner.name
                ));
            }
        }
        chosen.push(winner.clone());
    }

    // 确定性：按 pi provider 名排序（anthropic < openai）。
    chosen.sort_by(|a, b| a.family.pi_provider_name().cmp(b.family.pi_provider_name()));

    let entries = chosen
        .into_iter()
        .enumerate()
        .map(|(n, p)| {
            let key_env = format!("{PI_KEY_ENV_PREFIX}{n}");
            BridgeEntry {
                provider: p.family.pi_provider_name().to_string(),
                base_url: p.base_url,
                api_key_placeholder: format!("${{{key_env}}}"),
                key_env,
                key_value: p.api_key,
                source_provider: p.name,
            }
        })
        .collect();
    (entries, skipped)
}

/// 渲染 `models.json` 的 JSON 值（D3 形状）。
///
/// 空条目 → 空对象 `{}`（provider 注册表为空时 sebas 不写桥文件；本函数对空
/// 输入返回空对象，由 [`write_bridge`] 的调用方决定是否落盘）。
pub fn render_models_json(entries: &[BridgeEntry]) -> Value {
    if entries.is_empty() {
        return json!({});
    }
    let mut providers = serde_json::Map::new();
    for e in entries {
        providers.insert(
            e.provider.clone(),
            json!({
                "baseUrl": e.base_url,
                "apiKey": e.api_key_placeholder,
            }),
        );
    }
    json!({ "providers": Value::Object(providers) })
}

/// 把桥写进 `<pi_agent_dir>/models.json`（原子写：先写临时文件再 rename）。
///
/// `pi_agent_dir` 由调用方给出（[`StatePath::PiAgentDir`] 的 `resolve()` 或
/// 测试的临时目录）。目录不存在则创建。返回写出的文件路径。
///
/// 条目为空时**不写文件**、返回 `Ok(None)`——provider 注册表为空 = pi 无
/// 凭据，如实不激活（spec「empty registry → pi honestly reports
/// unconfigured」）。若目录里已有旧桥文件则删除它（避免旧凭据残留）。
///
/// [`StatePath::PiAgentDir`]: sebas_domain::state_paths::StatePath::PiAgentDir
pub fn write_bridge(
    pi_agent_dir: &Path,
    entries: &[BridgeEntry],
) -> std::io::Result<Option<PathBuf>> {
    let path = pi_agent_dir.join(MODELS_JSON);
    if entries.is_empty() {
        // 空注册表：不写桥；清掉可能残留的旧桥（否则 pi 会读到陈旧端点）。
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        return Ok(None);
    }
    std::fs::create_dir_all(pi_agent_dir)?;
    let value = render_models_json(entries);
    let body = serde_json::to_string_pretty(&value)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    // 原子写：同目录临时文件 + rename（读者永远看到完整文件）。
    let tmp = pi_agent_dir.join(format!(".{MODELS_JSON}.tmp"));
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, &path)?;
    Ok(Some(path))
}

/// spawn 期要注入 pi 子进程的 env 对：`PI_CODING_AGENT_DIR` +
/// `SEBAS_PI_KEY_n=<key>`（D2/D4）。
///
/// `pi_agent_dir` 是桥文件所在目录；`entries` 提供密钥真值与 env 名。
/// 密钥真值只在此处进入进程 env，**不落盘**。
pub fn spawn_env(pi_agent_dir: &Path, entries: &[BridgeEntry]) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = vec![(
        "PI_CODING_AGENT_DIR".to_string(),
        pi_agent_dir.display().to_string(),
    )];
    for e in entries {
        env.push((e.key_env.clone(), e.key_value.clone()));
    }
    env
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anthro(name: &str, url: &str, key: &str) -> ResolvedProvider {
        ResolvedProvider {
            name: name.into(),
            family: ProviderFamily::Anthropic,
            base_url: url.into(),
            api_key: key.into(),
        }
    }

    fn openai(name: &str, url: &str, key: &str) -> ResolvedProvider {
        ResolvedProvider {
            name: name.into(),
            family: ProviderFamily::OpenAi,
            base_url: url.into(),
            api_key: key.into(),
        }
    }

    /// 单 anthropic provider：一条桥条目，占位与 env 名自 0 起。
    #[test]
    fn render_anthropic_minimal() {
        let providers = vec![anthro("pistub", "https://gw.example/v1", "sk-secret")];
        let (entries, skipped) = bridge_entries(&providers, Some("pistub"));
        assert!(skipped.is_empty(), "无同族冲突: {skipped:?}");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].provider, "anthropic");
        assert_eq!(entries[0].base_url, "https://gw.example/v1");
        assert_eq!(entries[0].api_key_placeholder, "${SEBAS_PI_KEY_0}");
        assert_eq!(entries[0].key_env, "SEBAS_PI_KEY_0");
        assert_eq!(entries[0].key_value, "sk-secret");

        let value = render_models_json(&entries);
        assert_eq!(
            value,
            json!({
                "providers": {
                    "anthropic": {
                        "baseUrl": "https://gw.example/v1",
                        "apiKey": "${SEBAS_PI_KEY_0}"
                    }
                }
            })
        );
        // 真值不落盘：序列化字节里不含密钥明文。
        let bytes = serde_json::to_string(&value).unwrap();
        assert!(
            !bytes.contains("sk-secret"),
            "密钥真值不得进 models.json: {bytes}"
        );
    }

    /// 空 provider 快照 → 空 JSON 对象、零条目。
    #[test]
    fn empty_registry_yields_empty_json() {
        let (entries, skipped) = bridge_entries(&[], None);
        assert!(entries.is_empty());
        assert!(skipped.is_empty());
        assert_eq!(render_models_json(&entries), json!({}));
    }

    /// 同族多 provider：默认赢，其余跳过并记诊断；序号按 provider 名确定。
    #[test]
    fn same_family_default_wins_and_others_are_skipped() {
        let providers = vec![
            anthro("other", "https://other.example", "sk-other"),
            anthro("winner", "https://winner.example", "sk-winner"),
        ];
        let (entries, skipped) = bridge_entries(&providers, Some("winner"));
        assert_eq!(entries.len(), 1, "同族只留一条: {entries:?}");
        assert_eq!(entries[0].source_provider, "winner");
        assert_eq!(entries[0].base_url, "https://winner.example");
        assert_eq!(skipped.len(), 1);
        assert!(
            skipped[0].contains("other") && skipped[0].contains("已跳过"),
            "诊断点名被跳过的 provider: {skipped:?}"
        );
    }

    /// 默认 provider 缺席时同族取第一个（调用方已排好确定性顺序）。
    #[test]
    fn same_family_without_default_takes_first() {
        let providers = vec![
            anthro("first", "https://first.example", "sk-1"),
            anthro("second", "https://second.example", "sk-2"),
        ];
        let (entries, _) = bridge_entries(&providers, None);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].source_provider, "first");
    }

    /// 跨族（anthropic + openai）各留一条，序号按 provider 名确定。
    #[test]
    fn cross_family_keeps_one_per_family() {
        let providers = vec![
            openai("oai", "https://oai.example", "sk-oai"),
            anthro("anth", "https://anth.example", "sk-anth"),
        ];
        let (entries, skipped) = bridge_entries(&providers, Some("anth"));
        assert!(skipped.is_empty(), "跨族无冲突: {skipped:?}");
        assert_eq!(entries.len(), 2);
        // 排序后 anthropic 在前。
        assert_eq!(entries[0].provider, "anthropic");
        assert_eq!(entries[0].key_env, "SEBAS_PI_KEY_0");
        assert_eq!(entries[1].provider, "openai");
        assert_eq!(entries[1].key_env, "SEBAS_PI_KEY_1");

        let value = render_models_json(&entries);
        assert_eq!(
            value["providers"]["anthropic"]["baseUrl"],
            "https://anth.example"
        );
        assert_eq!(value["providers"]["openai"]["baseUrl"], "https://oai.example");
    }

    /// spawn env：含 PI_CODING_AGENT_DIR 与逐条密钥 env。
    #[test]
    fn spawn_env_carries_dir_and_keys() {
        let providers = vec![
            anthro("anth", "https://anth.example", "sk-anth"),
            openai("oai", "https://oai.example", "sk-oai"),
        ];
        let (entries, _) = bridge_entries(&providers, None);
        let env = spawn_env(Path::new("/tmp/pi-agent"), &entries);
        assert!(env
            .iter()
            .any(|(k, v)| k == "PI_CODING_AGENT_DIR" && v == "/tmp/pi-agent"));
        assert!(env
            .iter()
            .any(|(k, v)| k == "SEBAS_PI_KEY_0" && v == "sk-anth"));
        assert!(env
            .iter()
            .any(|(k, v)| k == "SEBAS_PI_KEY_1" && v == "sk-oai"));
        // 不喷 claude 家族 env。
        assert!(
            !env.iter().any(|(k, _)| k.starts_with("ANTHROPIC_")
                || k.starts_with("OPENAI_")),
            "桥路径绝不喷 claude 家族 env: {env:?}"
        );
    }

    /// write_bridge：非空写文件、内容正确、原子（无 tmp 残留）。
    #[test]
    fn write_bridge_persists_and_leaves_no_temp() {
        let dir = tempfile::tempdir().unwrap();
        let providers = vec![anthro("anth", "https://anth.example", "sk-anth")];
        let (entries, _) = bridge_entries(&providers, None);
        let path = write_bridge(dir.path(), &entries)
            .expect("write bridge")
            .expect("non-empty → Some(path)");
        assert_eq!(path, dir.path().join("models.json"));
        let body: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            body["providers"]["anthropic"]["baseUrl"],
            "https://anth.example"
        );
        assert!(
            !dir.path().join(".models.json.tmp").exists(),
            "临时文件必须已 rename 掉"
        );
    }

    /// write_bridge：空条目 → 不写文件、并清掉旧桥。
    #[test]
    fn write_bridge_empty_removes_stale_bridge() {
        let dir = tempfile::tempdir().unwrap();
        let providers = vec![anthro("anth", "https://anth.example", "sk-anth")];
        let (entries, _) = bridge_entries(&providers, None);
        write_bridge(dir.path(), &entries).unwrap().unwrap();
        assert!(dir.path().join("models.json").exists());

        // 注册表清空 → 旧桥必须删除（否则 pi 读到陈旧端点）。
        let out = write_bridge(dir.path(), &[]).expect("write empty");
        assert!(out.is_none(), "空条目不落盘");
        assert!(
            !dir.path().join("models.json").exists(),
            "陈旧桥文件必须被清理"
        );
    }
}
