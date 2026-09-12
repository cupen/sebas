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

export type StatusSlug = 'starting' | 'queued' | 'working' | 'done' | 'failed' | 'dormant'

export interface SessionRow {
  encoded_key: string
  chat_id: string
  status_slug: StatusSlug
  project_dir: string | null
  available_models: string[] | null
  current_model: string | null
  /** rail-declutter-unread：可见回复段数（未读徽标的服务端计数）。 */
  msg_count?: number
  /** rail-declutter-unread：首条用户消息预览（rail 行名数据源）。 */
  prompt_preview?: string | null
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
    prompt: string
    projectId?: string | null
    agent?: string
    /** 权限模式（add-agent-mode-selection）：`ask`/`edit`/`allow`/`auto`。 */
    mode?: string
  } = { prompt: 'hello' },
): Promise<string> {
  const resp = await request.post('/api/sessions', {
    data: {
      prompt: opts.prompt ?? null,
      project_id: opts.projectId ?? null,
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
  const d = (await (await request.get('/router/api/providers')).json()) as {
    providers?: { name: string }[]
  }
  return (d.providers ?? []).map((p) => p.name)
}

export async function authMe(request: APIRequestContext): Promise<AuthInfo> {
  return (await request.get('/api/auth/me')).json() as Promise<AuthInfo>
}

export async function authLogin(request: APIRequestContext, secret: string): Promise<number> {
  const resp = await request.post('/api/auth/login', { data: { secret } })
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
