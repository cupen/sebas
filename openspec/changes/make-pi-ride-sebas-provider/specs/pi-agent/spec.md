## MODIFIED Requirements

### Requirement: pi is fed by sebas via a bridge file in sebas's own directory

pi 的端点与密钥由 sebas 在**自己**的目录（`<SEBAS_HOME>/pi-agent/`）生成 `models.json` 桥喂入：sebas 从 settings.db `providers` 注册表按 protocol 家族覆盖 provider 级 `baseUrl`（`base_url_anthropic` → anthropic、`base_url_openai_chat` / `base_url_openai_responses` → openai 家族）；密钥经 `${SEBAS_PI_KEY_n}` 自命名 env 在 `models.json` 的 `apiKey` 字段里插值，真值只在 spawn 期 env 出现，**不落盘**。spawn pi 时 `PI_CODING_AGENT_DIR=<SEBAS_HOME>/pi-agent` 与 `SEBAS_PI_KEY_n=<key>` 注入；sebas SHALL NOT 触碰操作员 `~/.pi/agent`（"一个文件一个写入者"在桥文件这条路径上由"sebas 写自己目录"承担）。sebas SHALL NOT 将 `ANTHROPIC_*` / `OPENAI_*` 等 claude 家族 env 喷到 pi 子进程——pi 对内置 Anthropic 模型把 baseUrl 写死为 `https://api.anthropic.com`，`ANTHROPIC_BASE_URL` env 只作 `BaseAnthropic` 构造器默认值、被 `model.baseUrl` 显式值压掉，对端点重定向无效；用 claude 的 env 名字空间给 pi 传凭据是错配，sebas 不承担。provider 注册表为空时，sebas 不写桥文件，pi 凭据为空、reachability 的 cause 字段如实报"未配 provider"。

pi 的 reachability 仍以二进制在场探测；`pi auth check` 调用 SHALL 传 `--provider <id> --model <id>`，cause 字符串不泄漏 env 变量名到操作员可见文案（实现细节可入 tooltip）。

#### Scenario: registry entry becomes a pi bridge

- **WHEN** settings.db providers 含 anthropic-协议条目（`base_url_anthropic` 非空）且 `default_selection.provider` 指向它
- **THEN** sebas 在 `<SEBAS_HOME>/pi-agent/models.json` 写出 `{"providers":{"anthropic":{"baseUrl":"<base_url_anthropic>","apiKey":"${SEBAS_PI_KEY_0}"}}}`；spawn 时设 `PI_CODING_AGENT_DIR` 与 `SEBAS_PI_KEY_0`；pi 上游收到 pi 17 个内置 claude 模型、密钥经插值生效；操作员 `~/.pi/agent/` 未被读写

#### Scenario: empty registry → pi honestly reports unconfigured

- **WHEN** settings.db providers 表为空、config 无 `[provider.*]` / `[router]`
- **THEN** sebas 不生成桥文件；pi prompt 直接返回"未配 provider"或"No API key found"等具体原因；webui catalog 的 `native` 行 `reachable=false`、cause 指向"未配 provider，请到 Settings → Models 配置"

#### Scenario: sebas does not spray ANTHROPIC_* env

- **WHEN** sebas 启动 pi 子进程
- **THEN** 子进程 env 不含 sebas 注入的 `ANTHROPIC_API_KEY` / `ANTHROPIC_BASE_URL` / `ANTHROPIC_MODEL` 等（真值仅经 `SEBAS_PI_KEY_n` 通过 `${VAR}` 插值进 `models.json` 的 `apiKey` 字段）

#### Scenario: reachability advisory uses operator-friendly cause

- **WHEN** pi 二进制在场但 provider 未配 / 凭据未注入
- **THEN** `pi auth check --provider anthropic --model claude-opus-4-8` 返回非 ready；cause 字段呈现"未配 provider"等可读原因，不出现 env 变量名
