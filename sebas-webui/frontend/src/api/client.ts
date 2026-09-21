/**
 * Typed client for the backend JSON API (`/api/*`), mirroring the
 * `webui-api` capability contract. One place owns the shapes; views never
 * hand-roll fetch calls.
 *
 * A 401 from any admin endpoint signals "login required"; callers branch
 * on `ApiError.status`.
 */

import { notify } from '../notify.js'

export type StatusSlug =
  | 'starting'
  | 'queued'
  | 'working'
  | 'waiting'
  | 'done'
  | 'failed'
  | 'dormant'

/**
 * 待生效提交的处置（workbench-turn-queue D1）：`staging` = 并入首条消息
 * （spawn 窗口暂存）；`turn` = 按序执行的待执行回合。
 */
export type PendingDisposition = 'staging' | 'turn'

/**
 * One pending submission (workbench-turn-queue D1/D6): accepted by the core
 * but not yet started. `position` is the delivery-order index (staging
 * entries precede the turn queue); ids are per-session monotonic.
 */
export interface PendingSubmission {
  id: number
  text: string
  position: number
  disposition: PendingDisposition
  priority: boolean
}

/**
 * （add-remote-execution-node 8.x）远端会话的呈现信息，逐字对应 core 的
 * `RemoteSessionView`（冻结的 wire 契约）。`null`/缺省 = 主控本机会话——本机
 * 没有「节点在线吗」这个维度，**不伪造**一个 `online`。
 */
export interface RemoteSessionView {
  /** 会话所在执行节点的稳定标识。 */
  node_id: string
  /** `online` | `offline` | `terminated` | `gone`。 */
  node_status: string
  /** 离线/终止的成因（如实陈述；`terminated` 不等于「暂时联系不上」）。 */
  node_cause?: string | null
  /** 会话**期望**的 mode（`ask` / `edit` / `allow` / `auto`）。 */
  desired_mode?: string | null
  /** 执行体**实际强制**的 mode；与 desired 不同即「强制不了」，两个都要显示。 */
  effective_mode?: string | null
  /** 仍在等主控决定的悬空审批数；`> 0` = 在等人，不是在跑。 */
  parked_approvals: number
}

/**
 * （add-remote-execution-node 8.2）一个执行节点的可用性视图。真源是 core 的
 * 节点注册表（`GET /api/nodes` 透传），`local: true` 表示主控本机——它永远
 * 在线（否则你读不到这个响应）。
 */
export interface NodeInfo {
  id: string
  status: string
  last_seen_unix?: number | null
  created_unix?: number
  local?: boolean
}

/** `GET /api/nodes` 的响应：`remote_available=false` = 注册表不可得（不是「没有节点」）。 */
export interface NodesResponse {
  nodes: NodeInfo[]
  remote_available: boolean
  cause?: string | null
}

export interface SessionRow {
  encoded_key: string
  /**
   * ⚠ /api/sessions 行**不带** `chat_id`（它在 summary 的 active_session
   * 与 detail 的词表里）；0-turn 占位行连 prompt_preview /
   * session_id_short 都为 null——展示名走 fullSessionLabel 的键尾段兜底。
   */
  chat_id?: string
  thread_id: string | null
  session_id: string | null
  session_id_short: string | null
  status: string
  status_label: string
  status_slug: StatusSlug
  status_glyph: string
  last_active: string
  last_active_unix: number
  is_active: boolean
  /** 绑定项目的稳定 id。`null` = inbox（无项目）。 */
  project_id: string | null
  /** Short preview of the first user message, used as display label. */
  prompt_preview: string | null
  /**
   * （fix-webui-approval-restore-and-session-identity 5.1，design D6）操作者
   * 设置的会话 label；null/缺省 = 未设置。行命名优先级：label → 首条 prompt
   * 预览 → 短 id。
   */
  label?: string | null
  /**
   * rail-declutter-unread：服务端累计的可见回复段数。rail 未读徽标 =
   * `msg_count − 共享读锚（unread-cursor 模块）`，聚焦会话即清零。
   */
  msg_count: number
  /** 当前生效的模型 id（ACP agent 的 configOptions）；null = 无模型选择面。 */
  current_model: string | null
  /** 该会话可选的模型 id 列表；fallback 给创建会话表单当下拉数据源。 */
  available_models: string[] | null
  /** 创建时绑定的执行后端 kind（add-composer-agent-binding）；null = 默认 kind。 */
  agent_kind: string | null
  /** （wire-webui-sebas-agent-e2e）会话所属执行体（"acp"/"native"）；null = 未打标。 */
  backend?: string | null
  /** （workbench-turn-queue 7.4）待生效提交条数（Rail 关闭确认文案用）。 */
  pending_count: number
  /** （add-remote-execution-node 8.x）远端节点/mode/悬空审批呈现；null = 本机。 */
  remote?: RemoteSessionView | null
  /**
   * （session-slash-commands 2.2）agent 自广告的会话命令表。可选：空表不上
   * wire（旧 core 组合下键缺省）；缺省/空 = 无命令表面（composer 不渲染
   * 面板、不拦截 `/` 输入）。
   */
  available_commands?: AvailableCommandInfo[]
  /**
   * （fix-pending-queue-liveness 2.3）「回合占用」的引擎事实（WORKING ∨
   * 泊车 ∨ spawn 窗口）。（session-parallel-liveness-and-unread-polish 2.1，
   * D2）每帧/每行必带（无「只在 true 时上 wire」的兼容保留），消费方不再
   * 做 `status_slug === 'working'` 字符串回退。
   */
  turn_engaged: boolean
  /**
   * （session-parallel-liveness-and-unread-polish 1.3）spawn 失败原因原文。
   * 缺省/null = 非 spawn-failed 会话；失败会话行就地呈现原因。
   */
  spawn_failure_reason?: string | null
  /**
   * （3.2，D5b）操作者期望的 mode（控制面词汇）。非空 `string`，缺省
   * `'ask'`——wire/内存/UI 四层同一份字符串，无 null 路径。
   */
  desired_mode: string
}

export interface SessionSummary {
  chat_id: string
  thread_id: string | null
  session_id: string | null
  status: string
  status_label: string
  status_slug: StatusSlug
  status_glyph: string
  encoded_key: string
  /** 当前生效的模型 id；null = 无模型选择面。 */
  current_model: string | null
  /** 该会话可选的模型 id 列表；null/空 = 无模型选择面。 */
  available_models: string[] | null
  /** 创建时绑定的执行后端 kind（add-composer-agent-binding）；null = 默认 kind。 */
  agent_kind: string | null
  /** （wire-webui-sebas-agent-e2e）会话所属执行体（"acp"/"native"）；null = 未打标。 */
  backend?: string | null
  /**
   * （fix-webui-approval-restore-and-session-identity 4.1）绑定项目的稳定 id
   * ——聚焦反投影项目上下文的查找键。null = 无项目会话。
   */
  project_id?: string | null
  /** （workbench-turn-queue 6.1）待生效提交全量视图（投递序）。 */
  pending: PendingSubmission[]
  /** （add-remote-execution-node 8.x）远端节点/mode/悬空审批呈现；null = 本机。 */
  remote?: RemoteSessionView | null
  /**
   * （session-slash-commands 2.2）聚焦会话的命令表（composer 面板数据源）。
   * 可选：空表不上 wire；缺省/空 = 无命令表面。
   */
  available_commands?: AvailableCommandInfo[]
  /**
   * （fix-pending-queue-liveness 2.3）「回合占用」的引擎事实（WORKING ∨
   * 泊车 ∨ spawn 窗口）。（2.1，D2）每行必带，无字符串回退。
   */
  turn_engaged?: boolean
  // （fix-webui-streaming-liveness 3.3，D3 BREAKING）`entries` 已随 summary
  // 拆分退役：聚焦会话正文一律走 detail 游标路径
  // （`/api/sessions/{key}?entries_after=<n>`）。
  /**
   * （3.2，D5b）操作者期望的 mode（控制面词汇）。非空 `string`，缺省
   * `'ask'`；detail/行同形状真源。
   *
   * summary 投影按旧 core 兼容只在有值时透出（同 `turn_engaged`），消费端
   * 一律带 `?? 'ask'` 回退。
   */
  desired_mode?: string
}

export interface CardConfig {
  theme_color: string
  fold_long_output: boolean
  thinking_display: string
  max_user_text_chars: number
  max_tool_output_chars: number
}

export interface ProviderInfo {
  name: string
  /** 派生 preset 名；缺省 = 自定义 provider。 */
  preset?: string | null
  base_url_anthropic: string | null
  base_url_openai_chat: string | null
  base_url_openai_responses: string | null
}

/**
 * 模型条目的能力标记词表（redesign-provider-models-settings D2）。`text`
 * 隐含于每个条目、**不在 wire 上出现**；其余三个显式标注多模态输入能力。
 * 标记是纯展示/编辑用的元数据：不影响路由与请求准入。
 */
export type ModelCapability = 'vision' | 'audio' | 'video'

/** provider 模型列表的一个条目（redesign-provider-models-settings 1.1）：
 * 模型 id + 显式能力标记。遗留的裸字符串在读取侧归一化为 `tags: []`。 */
export interface ProviderModelEntry {
  id: string
  tags: ModelCapability[]
}

/** /api/providers 的 admin 列表条目（BFF 透传 core 状态库投影）。 */
export interface ProviderAdmin {
  name: string
  preset?: string | null
  base_url_anthropic: string | null
  base_url_openai_chat: string | null
  base_url_openai_responses: string | null
  api_key_env: string | null
  api_key_configured: boolean
  models: ProviderModelEntry[]
  /** 条目上存的默认 model（编辑回填用；null = 未设）。 */
  default_model?: string | null
  /** 协议偏好（auto/anthropic/openai；编辑回填用）。 */
  protocol?: string | null
  /** 上游 model id 改名映射（Advanced 编辑用；null/缺省 = 未设）。 */
  model_map?: Record<string, string> | null
}

/** /api/provider-presets 的条目（内置 preset 表只读视图，跟随代码）。 */
export interface ProviderPreset {
  name: string
  base_url_anthropic: string | null
  base_url_openai_chat: string | null
  base_url_openai_responses: string | null
  api_key_env: string
  models: ProviderModelEntry[]
}

/** provider 创建/编辑 payload（admin API 的键值子集）。`models` 是模型条目
 *  目录（条目对象数组；add-fetch-models 起抓取结果的挑选经普通编辑写入）。
 *  `api_key_env` 不再是表单输入（preset 的 env 名只是无明文 key 时的隐式
 *  回退）；仅编辑路径静默回填存量值以防整体替换丢字段。 */
export interface ProviderPayload {
  name?: string
  preset?: string
  base_url_anthropic?: string
  base_url_openai_chat?: string
  base_url_openai_responses?: string
  api_key?: string
  api_key_env?: string
  default_model?: string
  protocol?: string
  model_map?: Record<string, string>
  models?: ProviderModelEntry[]
}

export interface RouterInfo {
  listen: string | null
  provider_count: number
  debug: boolean
  has_auth: boolean
  providers: ProviderInfo[]
}

/**
 * 一条 agent 自广告的会话斜杠命令（session-slash-commands D1/D2 的 TS 镜像，
 * 对应 Rust `sebas_acp::AvailableCommand`）。`name` 是提交时的命令词
 * （`/name`），`description` 是面板说明，`hint` 是参数提示（claude 的
 * `argumentHint` / ACP 的 `UnstructuredCommandInput.hint`）。命令表真源是
 * agent 自己的广告（claude 握手 / 通用 ACP `available_commands_update`），
 * 前端绝不硬编码；空表 = 无命令表面（native 等），不是错误。
 */
export interface AvailableCommandInfo {
  name: string
  description: string
  hint?: string | null
}

/**
 * One conversation entry on the session payload（workbench-conversation-view
 * 1.1，design D1/D2）: `kind` is who produced it (`prompt` = operator
 * submission, `content` = agent side), `element_type` is the render type
 * (`markdown` | `thinking` | `tool` | `error`). The two sides of the
 * conversation share one ordered sequence — a client never rebuilds the
 * operator's turns from a separate field or from timestamps.
 */
/**
 * 转写条目的渲染类型词表（`element_type`，与 core 侧 `TurnEntry` 同源）：
 * markdown 正文 / thinking 过程 / tool 工具 / error 错误气泡 /
 * permission_mode_result 权限模式切换契约条目 / notice 中性提示条目。
 * `notice` 是零输出回合的合成提示（close-acceptance-blind-spots 4.2，
 * design D3）：回合正常结束但零可见输出时 core 追加，前端渲染为中性
 * 信息条——非错误语义，绝不复用 error 的红泡形态。
 */
export type ConversationElementType =
  | 'markdown'
  | 'thinking'
  | 'tool'
  | 'error'
  | 'permission_mode_result'
  | 'notice'

export interface ConversationEntryView {
  /** 0-based monotonic transcript position. */
  position: number
  kind: string
  element_type: ConversationElementType
  content: string
  /**
   * Unix seconds when this entry was appended (stamped at push time by
   * the core). `0` for entries without a known time — the client treats
   * those as anchor-less for the seen-boundary seam. The value is the
   * stable-identity anchor used by the conversation view's seam
   * visualisation: anchoring by position alone would drift onto a
   * different element when an older card refreshes in place.
   */
  created_at_unix: number
  /**
   * （workbench-agent-identity-and-process-folds D2）后端为工具条目构造的
   * 结构化标题（工具名 + 关键参数，如 `read · src/main.rs`）。可选：
   * 旧持久化条目没有该字段（undefined/null），前端回退通用标签。
   */
  title?: string | null
  /**
   * （fix-webui-qa-defects 5.2，design D5）错误条目的失败分类
   * （`spawn` | `stall` | `generic`）。可选：旧条目没有该字段，前端回退
   * 中性标签——错误气泡标签不再一律写死「spawn failed」。
   */
  failure_class?: string | null
}

export interface SessionDetail {
  chat_id: string
  thread_id: string | null
  session_id: string | null
  status: string
  status_label: string
  status_slug: StatusSlug
  status_glyph: string
  /**
   * The conversation as one ordered entry sequence（design D1）. The former
   * single `user_prompt` and agent-output-only `body` fields are retired —
   * a client SHALL NOT reconstruct operator turns from a separate field.
   */
  entries: ConversationEntryView[]
  msg_id: string | null
  last_active: string
  encoded_key: string
  /**
   * rail-declutter-unread：服务端累计的可见回复段数。transcript 标记已读时
   * 以它推进共享读锚的 `anchor_count`，rail 徽标与 seam 保持一致。
   */
  msg_count: number
  /** 当前生效的模型 id（add-acp-model-selection）；null = agent 无模型选项。 */
  current_model: string | null
  /** 可选模型列表（agent 的 configOptions），会话详情模型选择器的数据源。 */
  available_models: string[] | null
  /** 创建时绑定的执行后端 kind（add-composer-agent-binding）；null = 默认 kind。 */
  agent_kind: string | null
  /** （wire-webui-sebas-agent-e2e）会话所属执行体（"acp"/"native"）；null = 未打标。 */
  backend?: string | null
  /**
   * （fix-webui-qa-defects-round3 4.2）绑定项目的稳定 id——与 summary 行的
   * `project_id` 同词表（detail 投影本就下发，接口此前漏声明）。深链窗口
   * 里 summary 的焦点指针未落位时，归属反投影从 detail 取它。
   */
  project_id?: string | null
  /** （workbench-turn-queue 6.1）待生效提交全量视图（投递序）。 */
  pending: PendingSubmission[]
  /** （add-remote-execution-node 8.x）远端节点/mode/悬空审批呈现；null = 本机。 */
  remote?: RemoteSessionView | null
  /**
   * 操作者期望的 mode（add-agent-mode-selection，控制面词汇）。（3.2，D5b）
   * 非空 `string`，缺省 `'ask'`；composer 真源渲染，无空态选择器。
   */
  desired_mode: string
  /** 执行体回报的实际生效 mode；null = 未声称生效（与 desired 差异如实可见）。 */
  effective_mode?: string | null
  /**
   * （session-slash-commands 2.2）agent 自广告的会话命令表。可选：空表不上
   * wire（旧 core 组合下键缺省）；缺省/空 = 无命令表面。
   */
  available_commands?: AvailableCommandInfo[]
  /**
   * （fix-pending-queue-liveness 2.3）「回合占用」的引擎事实（WORKING ∨
   * 泊车 ∨ spawn 窗口）。（2.1，D2）每响应必带，无字符串回退。
   */
  turn_engaged: boolean
  /**
   * （session-parallel-liveness-and-unread-polish 1.3）spawn 失败原因原文
   * （与列表行同形状）。缺省/null = 非 spawn-failed；composer 就地呈现
   * 原因与重试入口。
   */
  spawn_failure_reason?: string | null
}

/**
 * Whether the agent core is reachable from the backend. When `ok` is false
 * the composer is gated — submitting would only produce a confusing error
 * from the spawned child, so we surface `cause` up front.
 */
export interface ReachabilityInfo {
  ok: boolean
  /**
   * （add-webui-tiered-notices D5）机器可读不可达分档，与 `core.reachability`
   * 推送 / `core.reachability.get` 响应同词表；fatal 横幅按它分文案，kind
   * 缺失退化通用「核心不可达」。可达时缺省。
   */
  kind?: 'startup_failed' | 'auth_rejected' | 'disconnected'
  cause?: string
}

/**
 * wire-webui-sebas-agent-e2e: 双执行体的逐体可用性。`native` 不可用时 composer
 * 渲染该选项为 disabled + cause（不让操作员提交后才看到失败）。后端不区分
 * 执行体时省略该段（`execution_bodies?: …`）。
 */
export interface ExecutionBodyStatus {
  name: string
  ok: boolean
  cause?: string | null
}

export interface Summary {
  active_count: number
  dormant_count: number
  spawning_count: number
  total_sessions: number
  uptime: string
  recent_sessions: SessionRow[]
  active_session: SessionSummary | null
  active_session_key: string | null
  reachability: ReachabilityInfo
  execution_bodies?: ExecutionBodyStatus[]
}

export interface SessionList {
  recent_sessions: SessionRow[]
  active_count: number
  dormant_count: number
  spawning_count: number
  total_sessions: number
  active_session_key: string | null
}

export interface About {
  uptime: string
  version: string
  rustc_version: string
  router_listen: string | null
  provider_count: number
  /**
   * 新会话缺省 agent kind（preselect-last-used-model 3.2）：装配点从
   * `cfg.acp.default_kind()` 注入的运行时真值（About INSTANCE 段只读行）。
   */
  default_agent_kind: string
}

/**
 * One agent in the catalog (`GET /api/agents`，workbench-agent-wire-fix
 * 3.1/3.2) — the single availability source. `id` is the wire vocabulary
 * (`[acp.agents.*]` config key or the reserved `"native"`); `display` is
 * presentation only. Driver concepts never appear here.
 */
export interface AgentKindInfo {
  id: string
  display: string
  reachable: boolean
  cause?: string
  version?: string
}

export interface AdminStatus {
  adapter_ok: boolean
  status: {
    version: string
    uptime_secs: number
    operations: Array<{ operation_id: string; request_type: string; status: string; message: string }>
    active_operation: unknown
  }
  uptime_secs: number
  uptime_display: string
}

export interface AdminEvent {
  seq: number
  operation_id: string
  kind: string
  message: string
}

export interface AdminService {
  name: string
  status: string
  desired: string
  uptime_secs: number | null
}

/** Admin mutation result（/api/admin/services/{name}/enable|disable|restart）。 */
export interface AdminMutationResult {
  operation_id: string
  status: string
  message: string
}

/**
 * `GET /api/env` 单条目的分类（split-env-vars-settings-section D2）：
 * `plain` = 非敏感（可显实际值）；`set_unset` = 敏感（只显已设置/未设置，
 * 值经服务端遮蔽、永不出现在响应里）。
 */
export type EnvVarKind = 'plain' | 'set_unset'

/**
 * `GET /api/env` 策划清单里的一条环境变量。`plain` 已设置时 `value` 是
 * 实际值，未设置时 `value = null`（默认值说明随 `what` 下发，前端标注
 * 「未设置（用默认）」）。`set_unset` 项 `value` 恒为 null；`set` 布尔表达
 * 已设置与否——**该字段缺失时前端无法断言状态**，必须如实呈现「无法确定」
 * 而不是按未设置处理（防御性解析）。
 */
export interface EnvVarEntry {
  name: string
  what: string
  kind: EnvVarKind
  value: string | null
  set?: boolean
}

/**
 * Execution-backend hint sent with `POST /api/sessions`. `"native"` spawns the
 * built-in kernel; `"acp"` (the default) spawns the configured default
 * third-party agent; `"acp:<slug>"` selects a specific configured agent kind.
 * Single-backend seams ignore the field.
 */
export type BackendHint = 'acp' | `acp:${string}` | 'native'

/** Parsed form of a [`BackendHint`]: the driver plus the optional kind slug. */
export interface ParsedBackendHint {
  driver: 'acp' | 'native'
  /** Agent kind slug for `acp:<slug>`; absent for the default `acp`. */
  slug?: string
}

/**
 * Normalize a backend-hint string into its driver + optional slug. The bare
 * `acp` hint (and any unrecognized value) resolves to the configured default
 * third-party agent, mirroring the backend's "empty kind = default" rule.
 */
export function parseBackendHint(hint: string): ParsedBackendHint {
  if (hint === 'native') return { driver: 'native' }
  if (hint.startsWith('acp:')) {
    const slug = hint.slice('acp:'.length)
    return slug ? { driver: 'acp', slug } : { driver: 'acp' }
  }
  return { driver: 'acp' }
}

/**
 * The operator's answer to a gated tool call, mirroring the backend's
 * internally-tagged `PermissionDecision` (session_backend.rs): on the wire
 * each variant is `{"decision": "allow_once" | "allow_session" | "deny"}`
 * or `{"decision": "escalate", "reason": "…"}`, and the answer endpoint
 * nests it as `{decision: <PermissionDecision>}`.
 */
export type PermissionDecision =
  | { decision: 'allow_once' }
  | { decision: 'allow_session' }
  | { decision: 'deny' }
  | { decision: 'escalate'; reason: string }

/**
 * RBAC 角色词表（add-webui-multiuser-rbac）：固定四档，映射由服务端代码
 * 定义（D3）。前端只做呈现层裁剪（隐藏无权限入口），防线在服务端路由层。
 */
export type Role = 'root' | 'admin' | 'member' | 'viewer'

/** 角色词表的规范顺序（新建/改角色下拉的数据源；跟随代码，不落配置）。 */
export const ROLES: readonly Role[] = ['root', 'admin', 'member', 'viewer']

/** GET /api/auth/me 的响应：服务端是否启用登录鉴权 + 当前会话状态。 */
export interface AuthInfo {
  enabled: boolean
  authenticated: boolean
  username: string | null
  /**
   * 零用户首启标记（add-webui-multiuser-rbac D5）：enabled 且未认证且
   * `needs_setup` 为真 → 前端渲染首启设置页而非登录页。字段缺失（旧服务端
   * 过渡期）按 false 处理。
   */
  needs_setup?: boolean
  /** 认证后随行的角色（会话绑定用户的当次实时解析读数）；未认证/未启用时缺省。 */
  role?: Role
}

/**
 * /api/users 列表条目（add-webui-multiuser-rbac D6）：无任何哈希字段——
 * 盐/哈希绝不 travels the wire，这里也不为它们留位。
 */
export interface UserRecord {
  id: number
  username: string
  role: Role
  enabled: boolean
  created_at_unix: number
  /** 服务端可能随行的更新时间；缺失不参与任何呈现判定。 */
  updated_at_unix?: number
}

// ---- Skills 管理面（add-agent-skills 5.2）--------------------------------

/**
 * `GET /api/skills` 的一行：仓内一个条目（agentskills 格式：目录 +
 * SKILL.md + 随附文件）。`reason` 只在 `valid=false` 时出现在 wire 上。
 */
export interface SkillEntry {
  name: string
  description: string | null
  /** 随附文件（`/` 分隔的相对路径，已排序）；不含 SKILL.md 本身。 */
  attachments: string[]
  valid: boolean
  /** invalid 成因（frontmatter 缺字段 / SKILL.md 缺失等）。 */
  reason?: string
}

/** `GET /api/skills/{name}`：SKILL.md 原文（前端渲染 markdown）+ attachments。 */
export interface SkillDetail {
  name: string
  /** 条目在仓但缺 SKILL.md（invalid）时为 null——诚实呈现，不是 404。 */
  text: string | null
  attachments: string[]
}

/** 一次 sync 对一个 backend 落点的报告（「仓 wins」语义的呈现层）。 */
export interface BackendSkillSyncReport {
  backend: string
  written: string[]
  overwritten: string[]
  deleted: string[]
  /** 名外条目计数（用户私产：服务端不读不导不动，只报数）。 */
  private_ignored: number
}

/** `POST /api/skills/sync` 的响应：逐 backend 报告 + 无落点 backend 名单。 */
export interface SkillsSyncResponse {
  reports: BackendSkillSyncReport[]
  no_placement: string[]
}

/** Error carrying the HTTP status so callers can branch (e.g. 401 login). */
export class ApiError extends Error {
  readonly status: number
  /**
   * 机器可读拒绝码（unify-router-process-shape 2.2/2.3：如
   * `active_routed_sessions`）；错误体未携带时为 null。调用方据此区分
   * 「业务拒绝（可交互兜底，如强制出口弹窗）」与「普通失败（内联呈现）」。
   */
  readonly code: string | null
  /** 与 `code` 同行的数值载荷（如活跃 routed 会话计数）；缺失/非数值为 null。 */
  readonly count: number | null
  constructor(
    status: number,
    message: string,
    code: string | null = null,
    count: number | null = null,
  ) {
    super(message)
    this.status = status
    this.code = code
    this.count = count
  }
}

/**
 * 网络级失败（add-webui-allowed-roots D6）：请求根本没拿到 HTTP 响应——
 * 服务进程死了、连接被拒、DNS 失败——fetch 抛 TypeError。统一包装成
 * `NetworkError`，让视图能区分「后端拒绝（ApiError）」与「进程没了」。
 */
export class NetworkError extends Error {
  constructor(message = '无法连接服务器（服务可能未运行）') {
    super(message)
    this.name = 'NetworkError'
  }
}

/**
 * Global handler fired whenever an API call gets a 401 while webui 登录鉴权
 * is enabled — the shell registers it to flip to the login view (e.g. when a
 * session expires mid-use). Login/logout calls bypass it.
 */
let onUnauthorized: (() => void) | null = null
export function setUnauthorizedHandler(handler: (() => void) | null): void {
  onUnauthorized = handler
}

/** 登录/探活端点自身的 401 不应触发全局登录页跳转（否则登录失败即循环跳转）。 */
function isAuthExempt(path: string): boolean {
  return (
    path === '/api/auth/login' ||
    path === '/api/auth/me' ||
    path === '/api/auth/setup' ||
    path === '/api/auth/logout'
  )
}

/**
 * 统一通知拦截的豁免名单（add-webui-tiered-notices D4/4.1，注释互链
 * `openspec/changes/add-webui-tiered-notices`）——**唯一清单**。名单内的
 * 调用点都已有内联错误呈现（表单就地报错 / 列表自带重试态）或走 401 登录
 * 跳转，拦截器跳过以免同一失败双弹；新增 API 调用点若失败已就地呈现，
 * 必须把它的 path 模式加进来（并注明调用点），否则会自动弹 warn。
 */
const NOTIFY_EXEMPT_PATHS: RegExp[] = [
  // 401 跳登录路径（isAuthExempt 同名单）：登录/首启/登出/探活自身失败就地
  // 呈现，且 401 本就整体不入通知层（见 reportForNotice）。
  /^\/api\/auth\//,
  // settings 表单与各分区（settings-modal.ts：失败一律内联呈现）。
  /^\/api\/settings$/,
  /^\/api\/env$/,
  /^\/api\/agents$/, // 新建会话对话框 / composer 目录（「目录不可用」内联态）
  /^\/api\/users(\/|$)/,
  /^\/api\/skills(\/|$)/,
  /^\/api\/providers(\/|$)/, // 含 /probe 抓模型（settings 表单内联）
  /^\/api\/provider-presets$/,
  /^\/api\/provider-defaults$/, // settings + 新建会话对话框（目录不可用态）
  // composer 提交与回合内控制（composer / turn-stream / review-card 内联）。
  /^\/api\/sessions\/[^/]+\/(message|model|mode|cancel|pending|activate)/,
  /^\/api\/permissions\//, // review-card 应答（卡片内联重试）
  // dashboard / 列表加载与项目面（视图自带内联重试态 / 就地报错）。
  /^\/api\/summary$/,
  /^\/api\/sessions(\/|$)/, // 列表/详情加载（重试态）+ createSession 表单 +
  // switch/close/archive/restore（确认框内联）；activate 由调用方
  // fire-and-forget（失败不致命，占位保留）——全段豁免。
  /^\/api\/projects(\/|$)/, // rail 加载/增删/排序（内联 addError 等）
  /^\/api\/archive(\/|$)/,
  /^\/api\/nodes$/, // rail/composer 节点面（remote_available=false 内联退化）
  /^\/api\/admin(\/|$)/, // services 分区（*Safe 包装 + 内联退化呈现）
  /^\/api\/fs\//, // folder-picker（内联错误态）
]

/**
 * 影响面判级（spec「判级由前端按影响面裁定，HTTP 状态只是信号」）：拦截器
 * 只对**未豁免**的失败弹 warn——操作调用（非 GET）给「操作失败」文案，读
 * 给「加载失败」文案；401 不入通知层（既有登录跳转接管）。当前视图面全部
 * 已豁免（各自内联），本规则先立、有源再接（proposal Non-goals 同款姿态）。
 */
function reportForNotice(method: string, path: string, message: string, status: number | null): void {
  if (status === 401) return // 401 → 登录跳转，SHALL NOT 进入通知层
  if (NOTIFY_EXEMPT_PATHS.some((re) => re.test(path))) return
  const action = method === 'GET' ? '加载失败' : '操作失败'
  notify({
    level: 'warn',
    message: `${action}：${message}`,
    dedupeKey: `${method} ${path.split('?')[0]}`,
  })
}

/**
 * fetch 的唯一包装点：网络级失败（TypeError，无 HTTP 响应）转成
 * `NetworkError`；HTTP 响应（含 4xx/5xx）原样返回，由 `unwrap` 归一。
 * 网络级失败在转换前先过统一通知拦截（add-webui-tiered-notices 4.1）。
 */
async function doFetch(path: string, init?: RequestInit): Promise<Response> {
  try {
    return await fetch(path, init)
  } catch (e) {
    if (e instanceof TypeError) {
      const err = new NetworkError()
      reportForNotice(init?.method ?? 'GET', path, err.message, null)
      throw err
    }
    throw e
  }
}

async function unwrap<T>(resp: Response, path?: string, method = 'GET'): Promise<T> {
  if (resp.ok) return (await resp.json()) as T
  if (resp.status === 401 && onUnauthorized && path && !isAuthExempt(path)) onUnauthorized()
  let message = `HTTP ${resp.status}`
  let code: string | null = null
  let count: number | null = null
  try {
    // 错误体信封不钉死（unify-router-process-shape D4）：拒绝载荷可能是
    // 顶层 `{code, count}`，也可能是嵌套信封 `{error: {code, count}}`；
    // `error` 为字符串时仍是既有 message 语义。两处都找，缺失即 null。
    const body = (await resp.json()) as unknown
    if (typeof body === 'object' && body !== null) {
      const b = body as { error?: unknown; code?: unknown; count?: unknown }
      const inner =
        b.error !== null && typeof b.error === 'object'
          ? (b.error as { code?: unknown; count?: unknown })
          : b
      if (typeof inner.code === 'string') code = inner.code
      if (typeof inner.count === 'number') count = inner.count
      if (typeof b.error === 'string') message = b.error
    }
  } catch {
    // non-JSON error body; keep the generic message
  }
  if (path) reportForNotice(method, path, message, resp.status)
  throw new ApiError(resp.status, message, code, count)
}

async function get<T>(path: string): Promise<T> {
  return unwrap<T>(await doFetch(path, { headers: { accept: 'application/json' } }), path, 'GET')
}

/**
 * Build a request URL with a query string. One place owns query encoding
 * (URLSearchParams) so values containing `\`, `+`, `&`, CJK, … survive the
 * round trip (add-webui-picker-workdir-start). `null`/`undefined`/empty
 * values are omitted entirely — an absent param lets the server apply its
 * own default (e.g. browse-dirs' work root), unlike an empty string.
 */
export function withQuery(path: string, params: Record<string, string | null | undefined>): string {
  const qs = new URLSearchParams()
  for (const [key, value] of Object.entries(params)) {
    if (value) qs.set(key, value)
  }
  const query = qs.toString()
  return query ? `${path}?${query}` : path
}

async function post<T>(path: string, body?: unknown): Promise<T> {
  return unwrap<T>(
    await doFetch(path, {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
        accept: 'application/json',
      },
      body: body === undefined ? '{}' : JSON.stringify(body),
    }),
    path,
    'POST',
  )
}

async function put<T>(path: string, body?: unknown): Promise<T> {
  return unwrap<T>(
    await doFetch(path, {
      method: 'PUT',
      headers: {
        'content-type': 'application/json',
        accept: 'application/json',
      },
      body: body === undefined ? '{}' : JSON.stringify(body),
    }),
    path,
    'PUT',
  )
}

async function del<T>(path: string): Promise<T> {
  return unwrap<T>(
    await doFetch(path, {
      method: 'DELETE',
      headers: { accept: 'application/json' },
    }),
    path,
    'DELETE',
  )
}

// Project registry namespace — defined first so `api.projects` can re-export it below.
const projects = {
  list: () => get<{ projects: Project[] }>('/api/projects'),
  // harden-core-channel-deployment 4.2/D7：本地降级路径的响应携带
  // `degraded: {cause}`（状态库路径无此字段）——前端据此就地提示
  // 「核心不可达，已写入本地注册表」。
  // add-remote-execution-node 8.1：`nodeId` 非空 = 注册到指定执行节点；
  // 缺省 = 本机节点（隐式，行为与既有注册一致）。远端路径由**该节点**判定。
  add: (path: string, nodeId?: string | null) =>
    post<Project & { degraded?: { cause: string } }>('/api/projects', {
      path,
      ...(nodeId ? { node_id: nodeId } : {}),
    }),
  /** 按稳定 id 移除项目（workbench-agent-wire-fix 2.5）。 */
  remove: async (id: string) =>
    unwrapText(
      await doFetch(`/api/projects/${encodeURIComponent(id)}/remove`, {
        method: 'POST',
      }),
      `/api/projects/${encodeURIComponent(id)}/remove`,
      'POST',
    ),
  reorder: (ids: string[]) =>
    post<{ projects: Project[] }>('/api/projects/reorder', { ids }),
  branch: (id: string) =>
    get<ProjectBranchInfo>(`/api/projects/${encodeURIComponent(id)}/branch`),
}

export const api = {
  // Reads
  summary: () => get<Summary>('/api/summary'),
  sessions: () => get<SessionList>('/api/sessions'),
  /**
   * Session detail（conversation-incremental-sync 2.1/D2）。`entriesAfter`
   * 给定 → 拼 `?entries_after=<n>`：响应结构不变，`entries` 只含
   * `position > n` 的条目（后端透传 `turns(key, n)`），status/pending 等
   * 其余字段照常随行；缺省 = 全量（现行为，老消费者零破坏）。
   */
  session: (encodedKey: string, entriesAfter?: number) =>
    get<SessionDetail>(
      withQuery(`/api/sessions/${encodedKey}`, {
        entries_after: entriesAfter === undefined ? undefined : String(entriesAfter),
      }),
    ),
  settings: () => get<{ card_config: CardConfig; router: RouterInfo }>('/api/settings'),
  about: () => get<About>('/api/about'),
  /**
   * 环境变量只读清单（split-env-vars-settings-section 1.1/2.1）：webui
   * 进程自身 env 的服务端策划清单，遮蔽在服务端完成——敏感项只回
   * `set` 布尔。纯 webui 面，core 不可达时照常工作。
   */
  env: () => get<{ items: EnvVarEntry[] }>('/api/env'),
  /** Agent catalog（唯一可用性真源；workbench-agent-wire-fix 3.2）。 */
  agents: () => get<{ agents: AgentKindInfo[] }>('/api/agents'),
  /**
   * （add-remote-execution-node 8.2）执行节点可用性。本机节点恒在列且在线；
   * 远端节点来自 core 注册表，`remote_available=false` 表示注册表不可得
   * （与「没有远端节点」是两回事——前端不许混为一谈）。
   */
  nodes: () => get<NodesResponse>('/api/nodes'),

  // Auth（webui 多用户登录鉴权，add-webui-multiuser-rbac D5/D6）：me 探明
  // enabled/authenticated/needs_setup/role；login 只收 {username,password}
  // 双字段（服务端缺字段一律 400）；setup 为零用户
  // 首启专属（成功即建立会话，语义同登录）。
  authMe: () => get<AuthInfo>('/api/auth/me'),
  authLogin: (username: string, password: string) =>
    post<{ status: string; username: string }>('/api/auth/login', { username, password }),
  authSetup: (username: string, password: string) =>
    post<{ status: string; username: string }>('/api/auth/setup', { username, password }),
  authLogout: () => post<{ status: string }>('/api/auth/logout'),

  // Users 管理（add-webui-multiuser-rbac D6，仅 root；越权 403、用户名占用
  // 与最后启用 root 保护 409、弱密码 400——文案取响应 error 字段就地展示）。
  // password/role/enabled 变更与删除按需踢目标用户会话（服务端职责）。
  usersList: () => get<{ users: UserRecord[] }>('/api/users'),
  usersCreate: (username: string, password: string, role: Role) =>
    post<{ status: string; username: string }>('/api/users', { username, password, role }),
  usersSetPassword: (id: number, password: string) =>
    post<{ status: string }>(`/api/users/${id}/password`, { password }),
  usersSetRole: (id: number, role: Role) =>
    post<{ status: string }>(`/api/users/${id}/role`, { role }),
  usersSetEnabled: (id: number, enabled: boolean) =>
    post<{ status: string }>(`/api/users/${id}/enabled`, { enabled }),
  usersDelete: (id: number) => del<{ status: string }>(`/api/users/${id}`),

  // Skills 管理面（add-agent-skills 5.2）：仓 = webui 所在机器上的
  // agentskills 目录。webui 是「查看与移除的窗口，不是编辑器」——没有
  // create/edit 入口（spec 明令），创建与修改走 CLI / git / npx 后点刷新。
  skillsList: () => get<{ skills: SkillEntry[] }>('/api/skills'),
  skillDetail: (name: string) =>
    get<SkillDetail>(`/api/skills/${encodeURIComponent(name)}`),
  /** 只删仓；backend 里的副本在下次 sync 时清理（spec 语义，确认文案讲明）。 */
  skillsDelete: (name: string) =>
    del<{ status: string; name: string }>(`/api/skills/${encodeURIComponent(name)}`),
  skillsSync: () => post<SkillsSyncResponse>('/api/skills/sync'),

  // Session mutations
  /**
   * Create a session（workbench-agent-wire-fix D2）：`agent` 必填（agent id
   * 或 "native"）；项目以 `project_id` 引用。会话创建后 agent 不可变。
   */
  createSession: (opts: {
    agent: string
    /** **必填**：会话必须从属于项目（无项目 = 无归属、无 rail 行、无状态面，
     * 服务端一律 400）。 */
    projectId: string
    prompt?: string | null
    model?: string | null
    /** 权限模式（add-agent-mode-selection）：`ask`/`edit`/`allow`/`auto`；
     * `null` = agent 默认行为（不发送 mode 字段）。 */
    mode?: string | null
  }) =>
    post<{ key: string }>('/api/sessions', {
      prompt: opts.prompt ?? null,
      project_id: opts.projectId,
      agent: opts.agent,
      model: opts.model ?? null,
      // undefined = 未选 mode：JSON 序列化时整个键省略（服务端 serde default
      // 语义 = agent 默认行为），与"缺省不发送字段"的 wire 姿态一致。
      mode: opts.mode,
    }),
  /** 中程切换会话模型（add-acp-model-selection）：`session/set_config_option`。 */
  setSessionModel: (encodedKey: string, modelId: string) =>
    post<{ status: string }>(`/api/sessions/${encodedKey}/model`, {
      model_id: modelId,
    }),
  /** 中程切换会话权限模式（add-agent-mode-selection）：接受与否经事件流反馈。 */
  setSessionMode: (encodedKey: string, mode: string) =>
    post<{ status: string }>(`/api/sessions/${encodedKey}/mode`, {
      mode,
    }),
  /**
   * （fix-webui-approval-restore-and-session-identity 1.2）待批审批读模型：
   * 打开/刷新会话时拉取一次，与 WS `permission.requested` 按 request_id
   * 幂等合并。未知会话 404（ApiError）；无泊车 = 空表。
   */
  sessionApprovals: (encodedKey: string) =>
    get<{ approvals: PendingApprovalInfo[] }>(
      `/api/sessions/${encodedKey}/approvals`,
    ),
  /**
   * （fix-webui-approval-restore-and-session-identity 5.1）设置/清空会话
   * label。`null` = 清空（行命名回退首条 prompt 预览 / 短 id）。
   */
  setSessionLabel: (encodedKey: string, label: string | null) =>
    post<{ status: string }>(`/api/sessions/${encodedKey}/label`, {
      label,
    }),
  /**
   * Answer a gated tool call (review card). Resolves `{status: "delivered"}`
   * when the pending request got the decision; rejects with `ApiError`
   * status 404 when no pending request carries that id (already answered,
   * timed out, or unknown).
   */
  answerPermission: (requestId: string, decision: PermissionDecision) =>
    post<{ status: string }>(`/api/permissions/${encodeURIComponent(requestId)}/answer`, {
      decision,
    }),
  sendMessage: (encodedKey: string, message: string) =>
    post<{ status: string }>(`/api/sessions/${encodedKey}/message`, { message }),
  /**
   * （workbench-interaction-polish 1.3/D5）中断会话在飞 turn：停止按钮走
   * 这里。类型化拒绝——未知 key 404、空闲会话 409（会话在但没有在飞
   * turn）、core 不可达 503；调用方经 callout 如实呈现。
   */
  cancelSession: (encodedKey: string) =>
    post<{ status: string }>(`/api/sessions/${encodedKey}/cancel`),
  /**
   * （workbench-turn-queue 6.2/D8）移除一个未开始的待生效提交；成功返回
   * 操作后的全量 pending（调用方据此对账，design D8）。拒绝类型化：未知
   * id 404 / 已开始 409 / 越优先 409 / 越界 400。
   */
  removePending: (encodedKey: string, pendingId: number) =>
    post<{ status: string; pending: PendingSubmission[] }>(
      `/api/sessions/${encodedKey}/pending/${pendingId}/remove`,
    ),
  /** （workbench-turn-queue 6.2/D8）在处置组内把提交重排到 to_index。 */
  movePending: (encodedKey: string, pendingId: number, toIndex: number) =>
    post<{ status: string; pending: PendingSubmission[] }>(
      `/api/sessions/${encodedKey}/pending/${pendingId}/move`,
      { to_index: toIndex },
    ),
  /** close 返回 discarded_pending（workbench-turn-queue 5.2）：随之丢弃的
   * 未执行待生效提交条数。 */
  closeSession: (encodedKey: string) =>
    post<{ status: string; active_session_key: string | null; discarded_pending: number }>(
      `/api/sessions/${encodedKey}/close`,
    ),
  switchSession: (encodedKey: string) =>
    post<{ status: string; redirect: string; active_session_key: string }>(
      `/api/sessions/${encodedKey}/switch`,
    ),
  /**
   * （workbench-live-conversation-flow 3.1）聚焦即拉起：无 prompt 拉起会话
   * 子进程（0-turn 占位 fresh、Dormant resume）。幂等——已活/在途返回
   * `already-running`；失败不致命（占位保留），调用方 fire-and-forget。
   */
  activateSession: (encodedKey: string) =>
    post<{ status: 'started' | 'already-running' }>(
      `/api/sessions/${encodedKey}/activate`,
    ),

  // Provider 管理（BFF → core 状态库；preset 表跟随代码）。
  providers: () => get<{ providers: ProviderAdmin[] }>('/api/providers'),
  /**
   * （workbench-conversation-view 4.1）默认 provider/model 预选数据。数据
   * 真源是 core 状态库（BFF 经状态 seam 读）；未设置 → 双 null；core 不可达
   * → 503（ApiError），调用方据此落到「目录不可用」显式态。
   */
  providerDefaults: () =>
    get<{ default_provider: string | null; default_model: string | null }>(
      '/api/provider-defaults',
    ),
  providerPresets: () => get<{ presets: ProviderPreset[] }>('/api/provider-presets'),
  providerCreate: (payload: ProviderPayload) =>
    post<{ created: string }>('/api/providers', payload),
  providerUpdate: (name: string, payload: ProviderPayload) =>
    put<{ updated: string }>(`/api/providers/${encodeURIComponent(name)}`, payload),
  providerDelete: (name: string) =>
    del<{ deleted: string }>(`/api/providers/${encodeURIComponent(name)}`),
  /**
   * （add-fetch-models）从 provider 官方 base url 抓取 model id 列表。core
   * 只读 GET，**不落库**：返回的 ids 仅用于呈现，挑选某个 id 才是普通编辑
   * （providerUpdate）。失败抛 ApiError，message 是净化后的原因
   * （状态码/类别，绝无密钥材料）。
   */
  fetchProviderModels: (name: string) =>
    post<{ provider: string; models: string[] }>(
      `/api/providers/${encodeURIComponent(name)}/probe`,
    ),

  // Admin reads
  adminStatus: () => get<AdminStatus>('/api/admin/status'),
  adminEvents: () => get<{ adapter_ok: boolean; events: AdminEvent[] }>('/api/admin/events'),
  adminServices: () =>
    get<{ adapter_ok: boolean; services: AdminService[] }>('/api/admin/services'),
  adminEventsSafe: async (): Promise<{ adapter_ok: boolean; events: AdminEvent[] }> => {
    try {
      return await api.adminEvents()
    } catch (e) {
      // 401/403 照常上抛（登录页接管 / 明示拒绝）；503、网络级失败等一切
      // 其余形态按「无 watchdog 控制面」诚实退化（fix-settings-menu-and-
      // services-semantics 1.1）。
      if (e instanceof ApiError && (e.status === 401 || e.status === 403)) throw e
      return { adapter_ok: false, events: [] }
    }
  },
  adminServicesSafe: async (): Promise<{ adapter_ok: boolean; services: AdminService[] }> => {
    try {
      return await api.adminServices()
    } catch (e) {
      // 同上：非鉴权失败一律呈现为 adapter_ok: false + 空表（spec「无
      // watchdog adapter 退化」），不让 Services 分区死在错误上。
      if (e instanceof ApiError && (e.status === 401 || e.status === 403)) throw e
      return { adapter_ok: false, services: [] }
    }
  },

  // Admin mutations + auth
  adminUpdate: () => post<{ operation_id: string; message: string }>('/api/admin/update'),
  adminUpdateDryRun: () =>
    post<{ operation_id: string; message: string }>('/api/admin/update/dry-run'),
  adminUpdateDev: () =>
    post<{ operation_id: string; message: string }>('/api/admin/update/dev'),
  adminRollback: () =>
    post<{ operation_id: string; message: string }>('/api/admin/rollback'),
  /** Per-service enable/disable（ServiceSet RPC，选择持久化）。
   * 503 = 无 watchdog 控制面；401/403 = 鉴权/CSRF 拒绝，调用方区分呈现。
   * disable 的 force 透传（unify-router-process-shape D3/D4）：停止被拒
   * （400/409 + `active_routed_sessions` + 计数，经 ApiError.code/.count
   * 携带）后，操作员确认强制出口即以 `{force: true}` 重发同一请求；
   * 首次尝试不带 force 字段（body 与既有字节形态一致）。 */
  enableService: (name: string) =>
    post<AdminMutationResult>(`/api/admin/services/${encodeURIComponent(name)}/enable`),
  disableService: (name: string, force = false) =>
    post<AdminMutationResult>(
      `/api/admin/services/${encodeURIComponent(name)}/disable`,
      force ? { force: true } : undefined,
    ),
  /**
   * Per-service restart（watchdog 监督循环的 ServiceRestart；wire 形状为
   * mutation_json 的 {status:"accepted", operation_id, message}）。
   * status-driven-service-rows D3：core 行纯只读、不渲染 ⟳，特判 core 的
   * 直连重启调用已删（core 重启只经 CLI / 升级流程；后端 admin 路由保留
   * 但前端无调用方）。
   */
  restartService: (name: string) =>
    post<AdminMutationResult>(`/api/admin/services/${encodeURIComponent(name)}/restart`),

  // Project registry (Workbench left rail).
  projects,

  // Archive
  archiveList: () => get<ArchiveList>('/api/archive'),
  /** 归档会话的只读详情（对话快照；polish-workbench-walkthrough-ux 2.1）。 */
  archiveDetail: (encodedKey: string) =>
    get<ArchiveDetail>(`/api/archive/${encodedKey}`),
  archiveSession: (encodedKey: string) =>
    post<{ status: string; entry: ArchiveEntry }>(`/api/sessions/${encodedKey}/archive`),
  restoreSession: (encodedKey: string) =>
    post<{ status: string; entry: ArchiveEntry }>(`/api/sessions/${encodedKey}/restore`),

  // Filesystem
  fsBrowse: (path: string) =>
    get<FsBrowseResponse>(withQuery('/api/fs/browse', { path })),
  // root 省略（null/undefined/空串）→ 服务端默认 work root（拾取器即用此）。
  fsBrowseDirs: (path: string, root?: string | null) =>
    get<FsBrowseResponse>(withQuery('/api/fs/browse-dirs', { path, root: root ?? undefined })),
}

// ---- Project API ----

export interface Project {
  /** Stable wire id (`proj-<12hex>`); the raw path never travels the wire. */
  id: string
  path: string
  name: string
  /**
   * （add-remote-execution-node 8.1）项目所在的执行节点。项目身份是
   * `(节点, 路径)`：同一路径在两台机器上是两个项目。缺省/`local` = 主控本机。
   */
  node_id?: string
  /** 该项目最近一次创建会话所用 agent（composer 预选用）。 */
  default_agent?: string | null
  added_at: number
  branch?: string | null
  branch_at?: number
  /**
   * harden-core-channel-deployment D7: present only when the registry write
   * fell back to the local file because the core state store was
   * unreachable. Absent on the healthy state-store path and on 503s.
   */
  degraded?: { cause: string }
}

export interface ProjectBranchInfo {
  project_id: string
  branch: string | null
  accessible: boolean
}

/** One archived session entry. */
export interface ArchiveEntry {
  session_key: string
  project_path: string
  label: string
  archived_at: number
  retention_deadline: number
  /**
   * （fix-webui-approval-restore-and-session-identity 3.1，design D3）归档
   * 时刻的会话身份四项；旧条目（无字段）恢复时如实回退默认。
   */
  agent_kind?: string | null
  desired_mode?: string | null
  current_model?: string | null
  available_models?: string[] | null
}

/**
 * （fix-webui-approval-restore-and-session-identity 1.2）一条待批权限请求的
 * 读模型行——`GET /api/sessions/{key}/approvals` 的元素。与 WS 推送按
 * `request_id` 幂等合并，重建审批面不依赖是否收到过原始事件。
 */
export interface PendingApprovalInfo {
  request_id: string
  tool_name: string
  args: unknown
}

/** Response from GET /api/archive. */
export interface ArchiveList {
  archived_sessions: ArchiveEntry[]
  /** 启动迁移提示（polish-workbench-walkthrough-ux 1.2）：仅迁移发生时出现，前端转 toast。 */
  migration?: string
}

/** GET /api/archive/{key} 的响应：归档条目 + 归档时刻的对话快照（只读回看）。 */
export interface ArchiveDetail {
  entry: ArchiveEntry
  entries: ConversationEntryView[]
}

/** Response from GET /api/fs/browse-dirs. */
export interface FsBrowseResponse {
  path: string
  entries: { name: string; is_dir: boolean; has_subdirs?: boolean }[]
}

async function unwrapText(resp: Response, path?: string, method = 'GET'): Promise<string> {
  if (resp.ok) return resp.text()
  if (resp.status === 401 && onUnauthorized && path && !isAuthExempt(path)) onUnauthorized()
  let message = `HTTP ${resp.status}`
  try {
    const body = (await resp.json()) as { error?: string }
    if (typeof body.error === 'string') message = body.error
  } catch {
    /* non-JSON error body */
  }
  if (path) reportForNotice(method, path, message, resp.status)
  throw new ApiError(resp.status, message)
}
