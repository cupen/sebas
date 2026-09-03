# sebas-agent-next

## Why

Phase 1a（headless 内核，`openspec/changes/sebas-agent`，已归档）证明循环与六件套工具可用，但 sebas-agent 尚无**权限门控**（任何写操作畅通无阻，破坏性动作不经过审批——checklist C3 缺口）、**网络工具**（不联网无法调研外部资料）、**webui 交互面**（无 UI，无法成为产品级工作台）、**上下文管理**（全量重放，长会话撞 token 顶）、以及**可复现的评估面**（缺乏自己的 benchmark 证明能力）。参考对象已经变了：DeepSeek Harness 于 2026-08-13 **开源**（`deepseek-ai/deepseek-harness`，MIT）——蓝图里靠第三方资料观测的 DSH 机制（文件沙箱、显式审批升级、todo/goal/job 编排、web_search/web_fetch）现在有第一手源码可对照。本 change 立项 **Phase 2：权限与沙箱 + 网络工具 + webui 首个交互面 + 上下文管理第一步 + benchmark 评估面**，把 sebas-agent 从"headless 内核"推进到"可日常使用的编码 agent"。

## What Changes

- **新增 `agent-core` 的权限与沙箱**：统一 `PolicyEngine`——allow/deny/ask 三层规则（静态规则 + 会话精确签名 allowlist + 交互审批）+ DSH 式一次性升级（"升级 = 带理由的同一操作重试"）；`PermissionRequest` 事件启用（webui 为首个回答者）；bash 子进程沙箱**缺省 Landlock**（进程内 `pre_exec`：工作区外拒写 + TCP 全拒，已实测验证；无外部二进制、Docker 内可用），不支持内核自动回退防火墙档（env 清洗 + 危险二进制探测），生效档位如实标注；`write`/`edit`/`bash` 的破坏面在工具执行前统一过策略。
- **新增 `bash` 网络升级** + 网络工具面（web_search / web_fetch，硬性结果上限与截断标记，默认拒绝、门控放行）。
- **新增 `agent-core` 的上下文管理**：工具结果改写（首个 ~8k 字符 + "\n[truncated]"，替换旧实现全量入库）、Assembly 预算的 max_messages 与 token 估算、并发工具执行（只读组并行 + 写工具串行）、`read_image` 与按需 lsp（不做能力不宣称）。
- **webui 接线（首个交互面）**：`run --webui` 进程内 `SessionBackend` 适配器（经既有 SessionBackend 缝）+ 审查卡片（webui 呈现 `PermissionRequest`，允许一次 / 本会话 / 拒绝返回策略）——webui 会话行下拉从 `acp-claude` / sebas-agent 选择执行后端；`acp-claude` 与其他面零改动。
- **sebas-agent 首个 benchmark（agent-bench）**：冒烟 CLI（web_search / apply_patch / subagent 分桶打分）+ 轨迹 JSONL + DAL 式 dashboard + 失败自愈用例。
- **对照更新 DSH 与 Codex 拆解 + 路线图**：`docs/superpowers/specs/2026-08-29-agent-core-architecture-design.md` 的 §3/§4/§11 修订（两者均已开源——`deepseek-ai/deepseek-harness`（2026-08-13）与 `openai/codex`（2026 演进）；Codex 的 CX-1/CX-3 裁决修正，DSH 机制行源码重写，Phase 3 拆 3a/3b/3c，持久化升为路线图显式条目，registry 中式设计共享）。

## Capabilities

### New Capabilities

- `agent-core`: sebas-agent 从 headless 内核升级为带权限沙箱、上下文管理、网络工具、并发与图像/语言能力（read_image/lsp）的可用编码 agent；事件词汇新增 `PermissionRequest` 启用语义与 `ToolFinish`/`SessionSummary` 辅助事件。
- `agent-bench`: sebas-agent 的能力评估面——冒烟 CLI、轨迹 JSONL、树状结果 dashboard、重放断言；不算分，不接 webui 报表。

### Modified Capabilities

- `webui`: `run --webui` 增加原生 agent 后端（会话行下拉选择 `acp-claude` / sebas-agent）；新增审查卡片与 `PermissionRequest` 呈现；webui 成为权限流程的首个回答者。

## Impact

- `sebas-agent/`：新 `policy/` 模块 + 事件与工具契约升级 + 新工具（web_search / web_fetch / read_image / lsp）+ LLM 多模态内容块 + 上下文压缩/并发/预算扩展（新依赖：`landlock`（Linux，主沙箱后端）、`url`、`mime_guess`、`serde_path_to_error`）。
- `sebas-webui/`：`SessionBackend` 新实现（`NativeAgentBackend`）+ 审查卡片路由 + 会话行后端选择。
- `sebas/`（binary crate）：`run --webui` 与 `agent-bench` 子命令。
- 对 `acp-claude` / `feishu` / `gateway` / `sebas-router`：零改动（benchmark 与通道共用 `sebas-agent` 直连 provider 路径，不依赖 channel 落地）。