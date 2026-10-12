## MODIFIED Requirements

### Requirement: credentials are pi-managed

sebas SHALL inject the provider credentials configured in its own provider registry into the pi child process as standard provider environment variables at spawn time, derived by the same provider resolution used for claude spawns (for example `ANTHROPIC_API_KEY` + `ANTHROPIC_BASE_URL` for an Anthropic-protocol provider, `OPENAI_API_KEY` + `OPENAI_BASE_URL` for an OpenAI-chat-protocol provider), so that an operator's sebas-side provider configuration reaches pi without requiring a pi-side login. When several configured providers map to the same standard environment names, the operator's default provider SHALL take precedence, and providers whose protocol cannot be expressed as a standard env pair SHALL be skipped with the skip recorded in spawn diagnostics. `models.json` and auth stores SHALL remain pi-managed: sebas SHALL NOT write them, and operator-defined custom providers are still configured on the pi side. Provider environment SHALL be resolved at spawn: credential changes after a session has spawned SHALL NOT alter the running session (mid-session model switching still works via `set_model`). pi's reachability SHALL be probed by binary presence, with `pi auth check` (ready/not_ready/invalid) stated as an advisory authentication cause when the binary is present — authentication state SHALL NOT gate selection.

#### Scenario: sebas-configured provider reaches pi without login

- **WHEN** 操作员仅在 sebas 的 Settings → Models 配置了一个 Anthropic 协议 provider（未在 pi 侧 `/login`）并创建 pi 会话
- **THEN** pi 子进程携带 sebas 注入的标准 env（`ANTHROPIC_API_KEY`/`ANTHROPIC_BASE_URL`），回合凭该 provider 完成上游认证，sebas 全程不读写任何 pi 凭据文件

#### Scenario: default provider wins env collisions

- **WHEN** sebas 配置了两个 Anthropic 协议 provider 且默认 provider 为 A
- **THEN** pi 子进程的 `ANTHROPIC_*` env 解析自 A，B 不注入；跳过事实记录在 spawn 诊断中

#### Scenario: models.json and auth stores stay untouched

- **WHEN** pi 会话 spawn 与运行
- **THEN** sebas 不创建、不修改 pi 的 `models.json` 与 auth 存储

#### Scenario: credential change after spawn does not affect running sessions

- **WHEN** 一个 pi 会话运行中，操作员在 Settings → Models 修改了 provider key
- **THEN** 在跑会话不受影响（env 为 spawn 期语义）；新建会话使用新解析

#### Scenario: reachability distinguishes installed-but-unauthenticated

- **WHEN** pi 二进制在场但 `pi auth check` 返回 not_ready
- **THEN** agent 目录将 pi 报为可达（二进制在场）但带认证未就绪的说明，而不是笼统的不可达
