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

  // Session mutations
  createSession: (prompt: string, projectDir?: string | null) =>
    post<{ key: string }>('/api/sessions', { prompt, project_dir: projectDir ?? null }),
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
