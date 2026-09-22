/**
 * API helper (tasks 2.1): typed wrappers around the webui JSON API for
 *Arrange/act steps that are faster or more deterministic over HTTP than
 * through the UI (session creation, polling for turn convergence), while
 * the assertions about what the OPERATOR SEES stay in the browser.
 *
 * Session keys embed a NUL byte; the API returns the key already in its
 * URL-encoded form (`web%00web-…` — the same shape the frontend stores in
 * `encoded_key`), so helpers pass it through verbatim and NEVER re-encode
 * (double-encoding only works by accident of the server's lenient decode).
 */
import type { APIRequestContext } from '@playwright/test'
import { sceneDir } from './scene.js'

// `waiting`（泊车审批在等操作者）是 detail/rail 的七词相位之一（webui
// models.rs Waiting → "waiting"）；审批对账旅程用它做 API 侧等待。
export type StatusSlug =
  | 'starting'
  | 'queued'
  | 'working'
  | 'waiting'
  | 'done'
  | 'failed'
  | 'dormant'

export interface SessionRow {
  encoded_key: string
  chat_id: string
  status_slug: StatusSlug
  project_dir: string | null
  available_models: string[] | null
  current_model: string | null
  /** 绑定项目的稳定 id（workbench-agent-wire-fix 2.5；null = inbox）。 */
  project_id?: string | null
  /** rail-declutter-unread：可见回复段数（未读徽标的服务端计数）。 */
  msg_count?: number
  /** rail-declutter-unread：首条用户消息预览（rail 行名数据源）。 */
  prompt_preview?: string | null
  /** fix-webui-approval-restore-and-session-identity 5.1：操作者 label（行名第一顺位）。 */
  label?: string | null
  /** rail 行名回退链的短 id 形态（后端 middle_truncate 18）。 */
  session_id_short?: string | null
  /** 创建时绑定的 agent kind（归档恢复身份断言用）；null = 默认。 */
  agent_kind?: string | null
}

export interface ConversationEntry {
  position: number
  kind: string
  element_type: string
  content: string
  created_at_unix: number
}

export interface SessionDetail {
  status_slug: StatusSlug
  status_label: string
  session_id: string | null
  /** workbench-conversation-view 1.1: one ordered entry sequence, both sides. */
  entries: ConversationEntry[]
  encoded_key: string
  current_model: string | null
  available_models: string[] | null
  /** add-agent-mode-selection：本机会话的 argv 应用值 / ModeChanged 同步值。 */
  desired_mode?: string | null
  effective_mode?: string | null
  /** 创建时绑定的 agent kind；null/缺省 = 默认 agent（归档恢复身份断言用）。 */
  agent_kind?: string | null
  /** fix-pending-queue-liveness：回合占用事实——只在 true 时上 wire。 */
  turn_engaged?: true
  /** workbench-turn-queue D1：投递序的待执行提交（队列管理面断言用）。 */
  pending?: PendingSubmissionRow[]
}

/** 泊车审批读模型行（fix-webui-approval-restore-and-session-identity 1.2）。 */
export interface PendingApproval {
  request_id: string
  tool_name: string
  args: unknown
}

/** 会话详情携带的待执行提交行（workbench-turn-queue D1 wire 形状）。 */
export interface PendingSubmissionRow {
  id: number
  text: string
  position: number
  disposition: 'staging' | 'turn'
  priority: boolean
}

/**
 * GET /api/sessions/{key}/approvals — the session's parked permission
 * requests (the read model the review surface rebuilds from on reload).
 */
export async function getSessionApprovals(
  request: APIRequestContext,
  encodedKey: string,
): Promise<{ status: number; approvals: PendingApproval[] }> {
  const resp = await request.get(`${sessionPath(encodedKey)}/approvals`)
  if (!resp.ok()) return { status: resp.status(), approvals: [] }
  const body = (await resp.json()) as { approvals?: PendingApproval[] }
  return { status: resp.status(), approvals: body.approvals ?? [] }
}

export interface Summary {
  active_session_key: string | null
  reachability: { ok: boolean; cause?: string }
  execution_bodies?: { name: string; ok: boolean; cause?: string | null }[]
}

export interface AuthInfo {
  enabled: boolean
  authenticated: boolean
  username: string | null
  /** add-webui-multiuser-rbac：仅认证后给出。 */
  role?: string | null
  /** 鉴权开启且零用户——首启设置页形态（前端渲染 setup 而非 login）。 */
  needs_setup?: boolean
}

/** Session detail path — the encoded key is already URL-safe, use verbatim. */
export function sessionPath(encodedKey: string): string {
  return `/api/sessions/${encodedKey}`
}

/**
 * Replicate the backend's display elision (models.rs `middle_truncate`):
 * the project rail labels session rows with this `session_id_short` form.
 * Backend detail: budget 18 → keep 17, head 9, tail 8, '…' in the middle.
 */
export function middleTruncate(s: string, max: number): string {
  const chars = [...s]
  if (chars.length <= max) return s
  const keep = max - 1
  const head = Math.floor(keep / 2) + (keep % 2)
  const tail = Math.floor(keep / 2)
  return [...chars.slice(0, head), '…', ...chars.slice(chars.length - tail)].join('')
}

export async function getSummary(request: APIRequestContext): Promise<Summary> {
  return (await request.get('/api/summary')).json() as Promise<Summary>
}

export async function listSessions(request: APIRequestContext): Promise<SessionRow[]> {
  const d = (await (await request.get('/api/sessions')).json()) as {
    recent_sessions: SessionRow[]
  }
  return d.recent_sessions
}

export async function getSession(
  request: APIRequestContext,
  encodedKey: string,
): Promise<{ status: number; detail: SessionDetail | null }> {
  const resp = await request.get(sessionPath(encodedKey))
  if (!resp.ok()) return { status: resp.status(), detail: null }
  return { status: resp.status(), detail: (await resp.json()) as SessionDetail }
}

export async function createSession(
  request: APIRequestContext,
  opts: {
    /** `null` = 0-turn placeholder（不 spawn 子进程，首条消息才开轮）。 */
    prompt: string | null
    projectId?: string | null
    agent?: string
    /** 权限模式（add-agent-mode-selection）：`ask`/`edit`/`allow`/`auto`。 */
    mode?: string
  } = { prompt: 'hello' },
): Promise<string> {
  // （session-parallel-liveness-and-unread-polish 3.6/D6b）缺省把会话绑到沙箱
  // 场景项目：会话状态只挂 rail 行首圆点，而无项目会话不进 rail——不绑项目
  // 就没有可断言的状态面，也不符操作者的真实用法（在项目下开会话）。
  // 显式传 `projectId`（含 `null`）时不覆盖。
  const projectId =
    opts.projectId === undefined ? (await ensureSceneProject(request)).id : opts.projectId
  const resp = await request.post('/api/sessions', {
    data: {
      prompt: opts.prompt ?? null,
      project_id: projectId ?? null,
      // agent 必填（workbench-agent-wire-fix D2）：沙箱默认 agent 是
      // `claude`（fake-claude）；fail-fast journeys 传未知 id 强制内显失败。
      agent: opts.agent ?? 'claude',
      // 缺省 = undefined → JSON 序列化整个键省略（服务端 serde default 语义）。
      mode: opts.mode,
    },
  })
  if (!resp.ok()) throw new Error(`createSession failed: HTTP ${resp.status()}`)
  const { key } = (await resp.json()) as { key: string }
  return key
}

export async function sendMessage(
  request: APIRequestContext,
  encodedKey: string,
  message: string,
): Promise<void> {
  const resp = await request.post(`${sessionPath(encodedKey)}/message`, { data: { message } })
  if (!resp.ok()) throw new Error(`sendMessage failed: HTTP ${resp.status()}`)
}

/**
 * project-session-actions「label writes through any path update the row
 * live」的 API 写入半边（fix-webui-qa-defects-round5 3.x）：绕过 rail 对话框
 * 直接写 label；`null` = 清空。
 */
export async function setSessionLabel(
  request: APIRequestContext,
  encodedKey: string,
  label: string | null,
): Promise<void> {
  const resp = await request.post(`${sessionPath(encodedKey)}/label`, { data: { label } })
  if (!resp.ok()) throw new Error(`setSessionLabel failed: HTTP ${resp.status()}`)
}

export async function listProjects(request: APIRequestContext): Promise<
  { id: string; path: string; name: string; default_agent?: string | null }[]
> {
  const d = (await (await request.get('/api/projects')).json()) as {
    projects: { id: string; path: string; name: string; default_agent?: string | null }[]
  }
  return d.projects
}

/**
 * rail-declutter-unread：Inbox 分组移除后，需要 rail 可见会话的旅程先注册
 * scene 项目并把会话绑定到它。幂等：已注册则直接返回既有条目。
 */
export async function ensureSceneProject(
  request: APIRequestContext,
): Promise<{ id: string; name: string }> {
  const scene = sceneDir()
  const name = scene.split(/[\\/]/).filter(Boolean).pop()!
  const existing = (await listProjects(request)).find((p) => p.path === scene)
  if (existing) return { id: existing.id, name }
  await addProject(request, scene)
  const created = (await listProjects(request)).find((p) => p.path === scene)
  if (!created) throw new Error('ensureSceneProject: project missing after add')
  return { id: created.id, name }
}

export async function addProject(request: APIRequestContext, path: string): Promise<void> {
  const resp = await request.post('/api/projects', { data: { path } })
  if (!resp.ok()) throw new Error(`addProject failed: HTTP ${resp.status()}`)
}

/** Raw add: returns status + body so rejection semantics (400/409) are assertable. */
export async function addProjectRaw(
  request: APIRequestContext,
  path: string,
): Promise<{ status: number; body: string }> {
  const resp = await request.post('/api/projects', { data: { path } })
  return { status: resp.status(), body: await resp.text() }
}

/** Reorder the project registry to the given stable-id sequence. */
export async function reorderProjects(
  request: APIRequestContext,
  ids: string[],
): Promise<void> {
  const resp = await request.post('/api/projects/reorder', { data: { ids } })
  if (!resp.ok()) throw new Error(`reorderProjects failed: HTTP ${resp.status()}`)
}

/** Branch info for a registered project id (git → name, non-git → null). */
export async function getBranch(
  request: APIRequestContext,
  id: string,
): Promise<{ status: number; branch: string | null }> {
  const resp = await request.get(`/api/projects/${encodeURIComponent(id)}/branch`)
  if (!resp.ok()) return { status: resp.status(), branch: null }
  const d = (await resp.json()) as { branch?: string | null }
  return { status: resp.status(), branch: d.branch ?? null }
}

/** Raw remove: returns status + body so rejection semantics (409) are assertable. */
export async function removeProjectRaw(
  request: APIRequestContext,
  id: string,
): Promise<{ status: number; body: string }> {
  const resp = await request.post(`/api/projects/${encodeURIComponent(id)}/remove`)
  return { status: resp.status(), body: await resp.text() }
}

export async function removeProject(request: APIRequestContext, id: string): Promise<void> {
  const resp = await request.post(
    `/api/projects/${encodeURIComponent(id)}/remove`,
  )
  if (!resp.ok()) throw new Error(`removeProject failed: HTTP ${resp.status()}`)
}

export async function archiveSession(
  request: APIRequestContext,
  encodedKey: string,
): Promise<void> {
  const resp = await request.post(`${sessionPath(encodedKey)}/archive`)
  if (!resp.ok()) throw new Error(`archiveSession failed: HTTP ${resp.status()}`)
}

export interface RouterInfo {
  listen: string | null
  provider_count: number
  debug: boolean
  has_auth: boolean
}

export interface AboutInfo {
  version: string
  uptime: string
  rustc_version: string
  router_listen: string | null
  provider_count: number
  /** 新会话缺省 agent kind（preselect-last-used-model 3.2 的载荷字段）。 */
  default_agent_kind: string
}

/** Router gateway card backing the settings Models section (listen/debug/auth). */
export async function getRouterInfo(request: APIRequestContext): Promise<RouterInfo> {
  const d = (await (await request.get('/api/router')).json()) as { router: RouterInfo }
  return d.router
}

/** Build metadata backing the settings About section. */
export async function getAbout(request: APIRequestContext): Promise<AboutInfo> {
  return (await request.get('/api/about')).json() as Promise<AboutInfo>
}

export interface AdminServiceRow {
  name: string
  status: string
  desired: string
  uptime_secs: number | null
}

export interface AdminServicesTruth {
  adapter_ok: boolean
  services: AdminServiceRow[]
}

/**
 * Watchdog managed-service surface backing the settings Services section
 * (fix-settings-menu-and-services-semantics §1: response is the truth source —
 * the sandbox assembly is a variable, never enumerate concrete services).
 */
export async function getAdminServices(
  request: APIRequestContext,
): Promise<AdminServicesTruth> {
  return (await request.get('/api/admin/services')).json() as Promise<AdminServicesTruth>
}

/** Raw provider-admin list (sandbox: read-only works, mutations 503). */
export async function listRouterProviders(request: APIRequestContext): Promise<string[]> {
  const d = (await (await request.get('/api/providers')).json()) as {
    providers?: { name: string }[]
  }
  return (d.providers ?? []).map((p) => p.name)
}

export async function authMe(request: APIRequestContext): Promise<AuthInfo> {
  return (await request.get('/api/auth/me')).json() as Promise<AuthInfo>
}

/** POST /api/auth/login（用户名+密码双字段）；返回状态码供断言。 */
export async function authLogin(
  request: APIRequestContext,
  username: string,
  password: string,
): Promise<number> {
  const resp = await request.post('/api/auth/login', { data: { username, password } })
  return resp.status()
}

/**
 * Poll the session detail until `predicate` holds (or timeout). Returns the
 * last detail snapshot. Used for API-side waits on turn convergence; UI
 * assertions use expect.poll directly on the page instead.
 */
export async function pollSession(
  request: APIRequestContext,
  encodedKey: string,
  predicate: (detail: SessionDetail) => boolean,
  opts: { timeout?: number; interval?: number } = {},
): Promise<SessionDetail> {
  const timeout = opts.timeout ?? 15_000
  const interval = opts.interval ?? 250
  const deadline = Date.now() + timeout
  let last!: SessionDetail
  for (;;) {
    const { detail } = await getSession(request, encodedKey)
    if (detail) {
      last = detail
      if (predicate(detail)) return detail
    }
    if (Date.now() > deadline) {
      throw new Error(
        `pollSession: condition not met within ${timeout}ms (last status: ${last?.status_slug})`,
      )
    }
    await new Promise((r) => setTimeout(r, interval))
  }
}

/** Convenience: wait until the session's turn converges to one of `slugs`. */
export function waitStatus(
  request: APIRequestContext,
  encodedKey: string,
  slugs: StatusSlug[],
  timeout = 20_000,
): Promise<SessionDetail> {
  return pollSession(request, encodedKey, (d) => slugs.includes(d.status_slug), { timeout })
}

/**
 * Focus-safe wait: the DETAIL read (GET /api/sessions/{key}) sets the server
 * focus pointer as a side effect（api.rs「Reading the detail focuses this
 * session」——深链语义）。聚焦敏感的旅程（未读徽章的「聚焦 + 可见不呈现」
 * 推导、底部跟读锚）在点击聚焦之后轮询详情，会把服务端焦点偷回被轮询的
 * 会话，被测语义随之失真——测试自己的探针流量成了焦点小偷。列表读取
 * （GET /api/sessions）无焦点副作用，行内 `status_slug` 同样可判相位。
 */
export async function waitListStatus(
  request: APIRequestContext,
  encodedKey: string,
  slugs: StatusSlug[],
  timeout = 20_000,
): Promise<SessionRow> {
  const deadline = Date.now() + timeout
  for (;;) {
    const row = (await listSessions(request)).find((r) => r.encoded_key === encodedKey)
    if (row && slugs.includes(row.status_slug)) return row
    if (Date.now() > deadline) {
      throw new Error(
        `waitListStatus: condition not met within ${timeout}ms (last: ${row?.status_slug ?? 'row missing'})`,
      )
    }
    await new Promise((r) => setTimeout(r, 250))
  }
}

/**
 * Reset the workbench to a deterministic base state: close every active
 * session (closing also clears the focused-session pointer) and unregister
 * every project. Journeys that assert first-paint structure (empty stream,
 * creation-mode composer, empty rail) call this first so they stay
 * independent of journey ordering and of retry leftovers.
 */
export async function resetState(request: APIRequestContext): Promise<void> {
  for (const s of await listSessions(request)) {
    await request.post(`${sessionPath(s.encoded_key)}/close`)
  }
  for (const p of await listProjects(request)) {
    await removeProject(request, p.id)
  }
}
