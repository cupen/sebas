- 检索代码优先用 codebase, 除非没有安装
- git commit log 遵守 Conventional Commits 提交规范，能一行说明就不要长篇大论。
  参考: https://www.conventionalcommits.org/en/v1.0.0/
- git commit 提交不要太琐碎，相关性较大的代码可一起提交。

## `/provider` 卡片布局（bead sebas-63f epic）

`/provider` 命令只发一张「Provider 管理」主卡（router/src/router/provider_card.rs），自上而下分四段：

1. **模式三按钮** — `Off` / `Direct` / `Gateway`，当前模式 `primary`，其余 `default`。点击写 `~/.sebas/state.json` 并刷整张卡。
2. **DIRECT 模式默认 provider 下拉** — `select_static`，选项 = 全 provider 名（字母序）+ 「（未设置）」。改动写 `state.json.default_selection`。placeholder 在 `default_selection.model` 设了的时候追加「· 当前默认 model: <model>」。
3. **Provider 列表** — 每个 provider 一行：默认折叠的 `CollapsiblePanel`，header 只显示 `name ✓DIRECT默认 · 默认 model` 一行摘要；展开/收起由飞书客户端本地完成（无服务端回调）。面板内是 markdown 字段行（预设 / Base URL / API Key 已配置/未配置 / 默认 model）+ 四按钮：🔍 探测 model 列表 / 编辑 / 删除 / 设为默认（DIRECT）。
4. **新建子区**（常驻底部）— 「＋ 新增（预设）」 / 「＋ 新增（自定义）」两个按钮，复用既有 `CrudForm::handle()` 走 `provider-preset` / `provider-custom` 旧 form 名。

三模式在 spawn 时做的事（src/spawn_env.rs）：

- `Off` + `default_selection` 未设 → driver 不发任何 env/args；claude 用它自己找到的配置（旧行为）。
- `Off` + `default_selection` 已设 → 隐式 Direct：用 `default_selection.provider` 解析（覆盖 overlay），加 `--model`（若 `default_selection.model` 或 overlay `default_model` 任一有值）。
- `Direct { provider }` → 读 `~/.sebas/providers.json`（overlay）+ 兜底 gateway config，翻译成 `ANTHROPIC_BASE_URL` + `ANTHROPIC_AUTH_TOKEN`（Anthropic 协议）或 `OPENAI_BASE_URL` + `OPENAI_API_KEY`（OpenAI 协议）。`--model <id>` 优先级：`state.default_selection.model`（provider 名匹配时）> overlay `default_model` > 不加。
- `Gateway` → 读 `gateway_cfg.listen` + `auth_token[0]`，翻译成 `ANTHROPIC_BASE_URL` + `ANTHROPIC_AUTH_TOKEN`（gateway 对 agent 永远暴露 Anthropic 协议面）。

🔍 探测 model 列表按钮是 model 列表的权威来源：优先尝试 `base_url_openai + /models`，回退 `base_url_anthropic + /v1/models`（后者 anthropic 协议通常会失败卡）。探测成功会把官方返回的完整列表写回 provider 的 `models` 目录字段，结果独立成一张卡让用户点回写 `default_model`；探测不可用则在表单里手填（`default_model` 在 preset / custom 表单都是文本框）。
