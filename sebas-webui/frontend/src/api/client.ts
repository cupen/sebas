/**
 * Typed client for the backend JSON API (`/api/*`), mirroring the
 * `webui-api` capability contract. One place owns the shapes; views never
 * hand-roll fetch calls.
 *
 * A 401 from any admin endpoint signals "login required"; callers branch
 * on `ApiError.status`.
 */

export type StatusSlug = 'starting' | 'queued' | 'working' | 'done' | 'failed' | 'dormant'

export interface SessionRow {
  encoded_key: string
  chat_id: string
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
  /** Bound project directory. `null` = inbox (no project). */
  project_dir: string | null
  /** Short preview of the first user message, used as display label. */
  prompt_preview: string | null
  /** 当前生效的模型 id（ACP agent 的 configOptions）；null = 无模型选择面。 */
  current_model: string | null
  /** 该会话可选的模型 id 列表；fallback 给创建会话表单当下拉数据源。 */
  available_models: string[] | null
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
  base_url_anthropic: string | null
  base_url_openai: string | null
}

export interface GatewayInfo {
  listen: string | null
  provider_count: number
  debug: boolean
  has_auth: boolean
  providers: ProviderInfo[]
}

export interface CardElementView {
  element_type: string
  content: string
  /**
   * Unix seconds when this entry was appended (stamped at push time by
   * the router). `0` for legacy entries that pre-date the field — the
   * client treats those as "no timestamp known" and skips them from the
   * seen-boundary calculation. The value is the stable-identity anchor
   * used by the transcript view's seam visualisation: anchoring by
   * position alone would drift onto a different element when an older
   * card refreshes in place, because `transcript_push` does not bump
   * `created_at_unix` on refresh.
   */
  created_at_unix: number
}

export interface SessionDetail {
  chat_id: string
  thread_id: string | null
  session_id: string | null
  status: string
  status_label: string
  status_slug: StatusSlug
  status_glyph: string
  user_prompt: string | null
  body: CardElementView[]
  msg_id: string | null
  last_active: string
  encoded_key: string
  /** 当前生效的模型 id（add-acp-model-selection）；null = agent 无模型选项。 */
  current_model: string | null
  /** 可选模型列表（agent 的 configOptions），会话详情模型选择器的数据源。 */
  available_models: string[] | null
}

/**
 * Whether the agent core is reachable from the backend. When `ok` is false
 * the composer is gated — submitting would only produce a confusing error
 * from the spawned child, so we surface `cause` up front.
 */
export interface ReachabilityInfo {
  ok: boolean
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
  gateway_listen: string | null
  provider_count: number
}

/** One configured third-party agent kind, as reported by /api/agent-kinds. */
export interface AgentKindInfo {
  name: string
  slug: string
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

/** Error carrying the HTTP status so callers can branch (e.g. 401 login). */
export class ApiError extends Error {
  readonly status: number
  constructor(status: number, message: string) {
    super(message)
    this.status = status
  }
}

async function unwrap<T>(resp: Response): Promise<T> {
  if (resp.ok) return (await resp.json()) as T
  let message = `HTTP ${resp.status}`
  try {
    const body = (await resp.json()) as { error?: string }
    if (typeof body.error === 'string') message = body.error
  } catch {
    // non-JSON error body; keep the generic message
  }
  throw new ApiError(resp.status, message)
}

async function get<T>(path: string): Promise<T> {
  return unwrap<T>(await fetch(path, { headers: { accept: 'application/json' } }))
}

async function post<T>(path: string, body?: unknown): Promise<T> {
  return unwrap<T>(
    await fetch(path, {
      method: 'POST',
      headers: { 'content-type': 'application/json', accept: 'application/json' },
      body: body === undefined ? '{}' : JSON.stringify(body),
    }),
  )
}

// Project registry namespace — defined first so `api.projects` can re-export it below.
const projects = {
  list: () => get<{ projects: Project[] }>('/api/projects'),
  add: (path: string) =>
    post<Project>('/api/projects', { path }),
  remove: async (path: string) =>
    unwrapText(
      await fetch(`/api/projects/${encodeURIComponent(path)}/remove`, { method: 'POST' }),
    ),
  reorder: (paths: string[]) =>
    post<{ projects: Project[] }>('/api/projects/reorder', { paths }),
  branch: (path: string) =>
    get<ProjectBranchInfo>(`/api/projects/${encodeURIComponent(path)}/branch`),
}

export const api = {
  // Reads
  summary: () => get<Summary>('/api/summary'),
  sessions: () => get<SessionList>('/api/sessions'),
  session: (encodedKey: string) => get<SessionDetail>(`/api/sessions/${encodedKey}`),
  settings: () => get<{ card_config: CardConfig; gateway: GatewayInfo }>('/api/settings'),
  gateway: () => get<{ gateway: GatewayInfo }>('/api/gateway'),
  about: () => get<About>('/api/about'),
  agentKinds: () => get<{ kinds: AgentKindInfo[] }>('/api/agent-kinds'),

  // Session mutations
  createSession: (
    prompt?: string | null,
    projectDir?: string | null,
    backend?: string | null,
    model?: string | null,
  ) =>
    post<{ key: string }>('/api/sessions', {
      prompt: prompt ?? null,
      project_dir: projectDir ?? null,
      backend: backend ?? null,
      model: model ?? null,
    }),
  /** 中程切换会话模型（add-acp-model-selection）：`session/set_config_option`。 */
  setSessionModel: (encodedKey: string, modelId: string) =>
    post<{ status: string }>(`/api/sessions/${encodedKey}/model`, {
      model_id: modelId,
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
  closeSession: (encodedKey: string) =>
    post<{ status: string; active_session_key: string | null }>(
      `/api/sessions/${encodedKey}/close`,
    ),
  switchSession: (encodedKey: string) =>
    post<{ status: string; redirect: string; active_session_key: string }>(
      `/api/sessions/${encodedKey}/switch`,
    ),

  // Admin reads
  adminStatus: () => get<AdminStatus>('/api/admin/status'),
  adminEvents: () => get<{ adapter_ok: boolean; events: AdminEvent[] }>('/api/admin/events'),
  adminServices: () =>
    get<{ adapter_ok: boolean; services: AdminService[] }>('/api/admin/services'),

  // Admin mutations + auth
  adminUpdate: () => post<{ operation_id: string; message: string }>('/api/admin/update'),
  adminUpdateDryRun: () =>
    post<{ operation_id: string; message: string }>('/api/admin/update/dry-run'),
  adminUpdateDev: () =>
    post<{ operation_id: string; message: string }>('/api/admin/update/dev'),
  adminRollback: () =>
    post<{ operation_id: string; message: string }>('/api/admin/rollback'),
  adminRestart: () =>
    post<{ operation_id: string; message: string }>('/api/admin/restart'),
  adminLogin: (password: string) => post<{ status: string }>('/api/admin/login', { password }),
  adminLogout: () => post<{ status: string }>('/api/admin/logout'),

  // Project registry (Workbench left rail).
  projects,

  // Archive
  archiveList: () => get<ArchiveList>('/api/archive'),
  archiveSession: (encodedKey: string) =>
    post<{ status: string; entry: ArchiveEntry }>(`/api/sessions/${encodedKey}/archive`),
  restoreSession: (encodedKey: string) =>
    post<{ status: string; entry: ArchiveEntry }>(`/api/sessions/${encodedKey}/restore`),

  // Filesystem
  fsBrowse: (path: string) =>
    get<FsBrowseResponse>(`/api/fs/browse?path=${encodeURIComponent(path)}`),
  fsBrowseDirs: (path: string, root?: string | null) =>
    root
      ? get<FsBrowseResponse>(`/api/fs/browse-dirs?path=${encodeURIComponent(path)}&root=${encodeURIComponent(root)}`)
      : get<FsBrowseResponse>(`/api/fs/browse-dirs?path=${encodeURIComponent(path)}`),
}

// ---- Project API ----

export interface Project {
  path: string
  name: string
  added_at: number
  branch?: string | null
  branch_at?: number
}

export interface ProjectBranchInfo {
  path: string
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
}

/** Response from GET /api/archive. */
export interface ArchiveList {
  archived_sessions: ArchiveEntry[]
}

/** Response from GET /api/fs/browse-dirs. */
export interface FsBrowseResponse {
  path: string
  entries: { name: string; is_dir: boolean; has_subdirs?: boolean }[]
}

async function unwrapText(resp: Response): Promise<string> {
  if (resp.ok) return resp.text()
  let message = `HTTP ${resp.status}`
  try {
    const body = (await resp.json()) as { error?: string }
    if (typeof body.error === 'string') message = body.error
  } catch {
    /* non-JSON error body */
  }
  throw new ApiError(resp.status, message)
}
