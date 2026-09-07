# Proposal: add-webui-allowed-roots

## Why

项目管理（Add Project）目前有两个入口可以绕出预期范围：目录树的
`GET /api/fs/browse-dirs` 接受任意显式 `root` 参数（带 `root=/etc` 即可浏览
`/etc`）；路径输入框走 `POST /api/projects`，后端只校验「存在且是目录」，
机器上任意目录都可注册。目录范围实际上不受控——需要一个可配置的白名单，
把「浏览」和「注册」两个入口同时圈住。

## What Changes

- 新增 `[watchdog.webui] allowed_roots` 配置（目录列表，支持 `~` 展开）。
- browse-dirs 的 root 解析改为：显式 `root` 必须命中白名单；未带 `root` 时
  仍回退「默认 agent kind 的 work_dir → 进程 cwd」（该回退根自动进入
  白名单，保证默认行为不变）。
- `POST /api/projects`（注册项目）校验路径必须位于白名单内，越界返回 400。
- 白名单为空时保持现状（默认根回退、注册只查存在性），不强制配置。
- 前端无需改动：folder-picker 不传 root 时的行为不变；越界路径的报错
  经由现有错误提示链路展示。
- 顺带补齐错误处理基线（见 design D6）：`/ws` 断线显示全局横幅并在恢复
  后自动刷新；网络级失败与后端业务错误在前端可区分；列表类视图加载失败
  显示内联重试态；composer 的 summary 轮询失败等同 core 不可达。

## Capabilities

### New Capabilities

（无——本变更收敛在既有 webui 能力的行为约束内，不引入新表面。）

### Modified Capabilities

- `webui`: browse-dirs 的 root 解析增加 allowed_roots 白名单约束；
  项目注册端点增加路径范围校验；HTTP route surface 要求中补充
  `POST /api/projects` 的范围拒绝语义。

## Impact

- `src/config.rs`：`[watchdog.webui]` 增加 `allowed_roots` 字段。
- `src/run.rs` / `src/webui_cmd.rs`：两处接线把白名单注入 WebUiState。
- `sebas-webui/src/fs.rs`（safe_path 白名单校验）、`src/server.rs`（State）、
  `sebas-webui/src/projects.rs` 或 `api.rs`（注册校验）。
- 前端：`api/ws.ts` / `api/shared-ws.ts`（连接状态暴露）、`app-shell.ts`
  （全局横幅）、`api/client.ts`（网络级错误包装）、`views/dashboard.ts`、
  `views/project-rail.ts`、`views/sessions.ts`（内联重试态）、
  `views/workbench-composer.ts`（summary 失败等同不可达）。
- 现有测试：`fs.rs` 单测中「explicit_root_overrides_server_default」等
  语义保持；新增白名单边界单测。
- 兼容性：未配置 `allowed_roots` 时行为与现状完全一致，非破坏性。

## Non-goals

- 不改 folder-picker 组件的交互（不新增多根切换 UI）。
- 不做运行时白名单热更新（仍随进程启动读取配置）。
- 不约束已注册项目的会话 work dir 运行时行为（agent 进程沙箱另属
  acp-driver 范畴）。
- 不处理 feishu/im 等其它通道的项目路径来源。
