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
  /** 创建时绑定的执行后端 kind（add-composer-agent-binding）；null = 默认 kind。 */
  agent_kind: string | null
  /** （wire-webui-sebas-agent-e2e）会话所属执行体（"acp"/"native"）；null = 未打标。 */
  backend?: string | null
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

/** /router/api/providers 的 admin 列表条目（BFF 透传 router admin API）。 */
export interface RouterProviderAdmin {
  name: string
  preset?: string | null
  base_url_anthropic: string | null
  base_url_openai_chat: string | null
  base_url_openai_responses: string | null
  api_key_env: string | null
  api_key_configured: boolean
  models: string[]
}

/** /api/agent-defaults（新会话默认 provider/model；BFF 透传 router admin）。 */
export interface AgentDefaults {
  provider: string | null
  model: string | null
}

/** /router/api/presets 的条目（内置 preset 表只读视图，跟随代码）。 */
export interface ProviderPreset {
  name: string
  base_url_anthropic: string | null
  base_url_openai_chat: string | null
  base_url_openai_responses: string | null
  api_key_env: string
  models: string[]
}

/** provider 创建/编辑 payload（admin API 的键值子集）。 */
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
}

export interface RouterInfo {
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
  /** 创建时绑定的执行后端 kind（add-composer-agent-binding）；null = 默认 kind。 */
  agent_kind: string | null
  /** （wire-webui-sebas-agent-e2e）会话所属执行体（"acp"/"native"）；null = 未打标。 */
  backend?: string | null
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
  router_listen: string | null
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

/** Admin mutation result（/api/admin/services/{name}/enable|disable|restart）。 */
export interface AdminMutationResult {
  operation_id: string
  status: string
  message: string
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
/** GET /api/auth/me 的响应：服务端是否启用登录鉴权 + 当前会话状态。 */
export interface AuthInfo {
  enabled: boolean
  authenticated: boolean
  username: string | null
}

/** Error carrying the HTTP status so callers can branch (e.g. 401 login). */
export class ApiError extends Error {
  readonly status: number
  constructor(status: number, message: string) {
    super(message)
    this.status = status
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

/**
 * Admin CSRF token（`POST /api/admin/login` 或 `GET /api/admin/csrf` 下发，
 * JS 从 HttpOnly cookie 拿不到，只能走 body）。内存 + sessionStorage 双持：
 * 内存是真源，sessionStorage 让页面 reload/第二 tab 免重新登录即可恢复。
 */
let adminCsrfToken: string | null = null
try {
  adminCsrfToken = sessionStorage.getItem('sebas_admin_csrf')
} catch {
  adminCsrfToken = null
}
export function setAdminCsrfToken(token: string | null): void {
  adminCsrfToken = token
  try {
    if (token) sessionStorage.setItem('sebas_admin_csrf', token)
    else sessionStorage.removeItem('sebas_admin_csrf')
  } catch {
    // sessionStorage 不可用（隐私模式等）则仅内存持有
  }
}
export function getAdminCsrfToken(): string | null {
  return adminCsrfToken
}

/** 登录/探活端点自身的 401 不应触发全局登录页跳转（否则登录失败即循环跳转）。 */
function isAuthExempt(path: string): boolean {
  return (
    path === '/api/auth/login' ||
    path === '/api/auth/me' ||
    path === '/api/auth/logout' ||
    path === '/api/admin/login' ||
    path === '/api/admin/csrf'
  )
}

/**
 * fetch 的唯一包装点：网络级失败（TypeError，无 HTTP 响应）转成
 * `NetworkError`；HTTP 响应（含 4xx/5xx）原样返回，由 `unwrap` 归一。
 */
async function doFetch(path: string, init?: RequestInit): Promise<Response> {
  try {
    return await fetch(path, init)
  } catch (e) {
    if (e instanceof TypeError) throw new NetworkError()
    throw e
  }
}

async function unwrap<T>(resp: Response, path?: string): Promise<T> {
  if (resp.ok) return (await resp.json()) as T
  if (resp.status === 401 && onUnauthorized && path && !isAuthExempt(path)) onUnauthorized()
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
  return unwrap<T>(await doFetch(path, { headers: { accept: 'application/json' } }), path)
}

function csrfHeaders(): Record<string, string> {
  return adminCsrfToken ? { 'x-csrf-token': adminCsrfToken } : {}
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
        ...csrfHeaders(),
      },
      body: body === undefined ? '{}' : JSON.stringify(body),
    }),
    path,
  )
}

async function put<T>(path: string, body?: unknown): Promise<T> {
  return unwrap<T>(
    await doFetch(path, {
      method: 'PUT',
      headers: {
        'content-type': 'application/json',
        accept: 'application/json',
        ...csrfHeaders(),
      },
      body: body === undefined ? '{}' : JSON.stringify(body),
    }),
    path,
  )
}

async function del<T>(path: string): Promise<T> {
  return unwrap<T>(
    await doFetch(path, {
      method: 'DELETE',
      headers: { accept: 'application/json', ...csrfHeaders() },
    }),
    path,
  )
}

// Project registry namespace — defined first so `api.projects` can re-export it below.
const projects = {
  list: () => get<{ projects: Project[] }>('/api/projects'),
  // harden-core-channel-deployment 4.2/D7：本地降级路径的响应携带
  // `degraded: {cause}`（状态库路径无此字段）——前端据此就地提示
  // 「核心不可达，已写入本地注册表」。
  add: (path: string) =>
    post<Project & { degraded?: { cause: string } }>('/api/projects', { path }),
  remove: async (path: string) =>
    unwrapText(
      await doFetch(`/api/projects/${encodeURIComponent(path)}/remove`, {
        method: 'POST',
        headers: { ...csrfHeaders() },
      }),
      `/api/projects/${encodeURIComponent(path)}/remove`,
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
  settings: () => get<{ card_config: CardConfig; router: RouterInfo }>('/api/settings'),
  router: () => get<{ router: RouterInfo }>('/api/router'),
  about: () => get<About>('/api/about'),
  agentKinds: () => get<{ kinds: AgentKindInfo[] }>('/api/agent-kinds'),

  // Auth（webui 登录鉴权；me 探明 enabled/authenticated，login 换会话 cookie。
  // 登录为单字段形态：secret 可以是登录 token 或账户密码，服务端自动识别。）
  authMe: () => get<AuthInfo>('/api/auth/me'),
  authLogin: (secret: string) =>
    post<{ status: string; username: string }>('/api/auth/login', { secret }),
  authLogout: () => post<{ status: string }>('/api/auth/logout'),

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

  // Router provider 管理（BFF → router admin API；preset 表跟随代码）。
  routerProviders: () =>
    get<{ providers: RouterProviderAdmin[] }>('/router/api/providers'),
  routerPresets: () => get<{ presets: ProviderPreset[] }>('/router/api/presets'),
  routerProviderCreate: (payload: ProviderPayload) =>
    post<{ created: string }>('/router/api/providers', payload),
  routerProviderUpdate: (name: string, payload: ProviderPayload) =>
    put<{ updated: string }>(`/router/api/providers/${encodeURIComponent(name)}`, payload),
  routerProviderDelete: (name: string) =>
    del<{ deleted: string }>(`/router/api/providers/${encodeURIComponent(name)}`),
  routerProviderProbe: (name: string) =>
    post<{ models: string[]; applied: boolean }>(
      `/router/api/providers/${encodeURIComponent(name)}/probe?apply=true`,
    ),
  agentDefaults: () => get<AgentDefaults>('/api/agent-defaults'),
  setAgentDefaults: (payload: { provider: string | null; model?: string | null }) =>
    put<AgentDefaults>('/api/agent-defaults', payload),

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
  adminRestart: () =>
    post<{ operation_id: string; message: string }>('/api/admin/restart'),
  /** Per-service enable/disable（ServiceSet RPC，选择持久化）。
   * 503 = 无 watchdog 控制面；401/403 = 鉴权/CSRF 拒绝，调用方区分呈现。 */
  enableService: (name: string) =>
    post<AdminMutationResult>(`/api/admin/services/${encodeURIComponent(name)}/enable`),
  disableService: (name: string) =>
    post<AdminMutationResult>(`/api/admin/services/${encodeURIComponent(name)}/disable`),
  /**
   * Per-service restart（fix-settings-menu-and-services-semantics D3）。
   * core 走既有 restart-core 路径（spec「restart 操作」）；其余受管服务走
   * watchdog 监督循环的 ServiceRestart。两个端点的 wire 形状同为
   * mutation_json 的 {status:"accepted", operation_id, message}。
   */
  restartService: async (name: string): Promise<AdminMutationResult> => {
    if (name === 'core') {
      const r = await api.adminRestart()
      return { operation_id: r.operation_id, status: 'accepted', message: r.message }
    }
    return post<AdminMutationResult>(`/api/admin/services/${encodeURIComponent(name)}/restart`)
  },
  adminLogin: async (password: string) => {
    const res = await post<{ status: string; csrf_token?: string }>('/api/admin/login', {
      password,
    })
    if (res.csrf_token) setAdminCsrfToken(res.csrf_token)
    return res
  },
  /** 已有会话免重新登录恢复 CSRF（页面 reload/第二 tab 用）。 */
  adminCsrf: async () => {
    const res = await get<{ csrf_token: string }>('/api/admin/csrf')
    setAdminCsrfToken(res.csrf_token)
    return res
  },
  adminLogout: async () => {
    try {
      return await post<{ status: string }>('/api/admin/logout')
    } finally {
      setAdminCsrfToken(null)
    }
  },

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
    get<FsBrowseResponse>(withQuery('/api/fs/browse', { path })),
  // root 省略（null/undefined/空串）→ 服务端默认 work root（拾取器即用此）。
  fsBrowseDirs: (path: string, root?: string | null) =>
    get<FsBrowseResponse>(withQuery('/api/fs/browse-dirs', { path, root: root ?? undefined })),
}

// ---- Project API ----

export interface Project {
  path: string
  name: string
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

async function unwrapText(resp: Response, path?: string): Promise<string> {
  if (resp.ok) return resp.text()
  if (resp.status === 401 && onUnauthorized && path && !isAuthExempt(path)) onUnauthorized()
  let message = `HTTP ${resp.status}`
  try {
    const body = (await resp.json()) as { error?: string }
    if (typeof body.error === 'string') message = body.error
  } catch {
    /* non-JSON error body */
  }
  throw new ApiError(resp.status, message)
}
