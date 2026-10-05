// @vitest-environment jsdom
// /sessions 总览页（fix-webui-qa-round14 2.3/2.4，D-5-1/D-5-3）：
//   - viewer 无写入口：新建表单与卡片「聚焦/关闭」按钮不呈现，只读列表
//     照常渲染（role→visibility 映射与工作台 rail 同源）
//   - 被拒动作明确呈现：403 = 「无权限」口径（点名角色限制）、其余 =
//     后端类型化文案——经 notice 层，绝不把列表替换成「加载失败」横幅
//   - 横幅语义拆分：只有读失败才渲染「加载失败」
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { installWaDomPolyfills } from '../test-support/wa-polyfills.js'

installWaDomPolyfills()

const apiMocks = vi.hoisted(() => ({
  sessions: vi.fn(),
  projectsList: vi.fn(),
  agents: vi.fn(),
  createSession: vi.fn(),
  switchSession: vi.fn(),
  closeSession: vi.fn(),
}))

vi.mock('../api/client.js', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/client.js')>()
  return {
    ...actual,
    ApiError: actual.ApiError,
    api: {
      ...actual.api,
      sessions: apiMocks.sessions,
      projects: { ...actual.api.projects, list: apiMocks.projectsList },
      agents: apiMocks.agents,
      createSession: apiMocks.createSession,
      switchSession: apiMocks.switchSession,
      closeSession: apiMocks.closeSession,
    },
  }
})

const wsMocks = vi.hoisted(() => ({
  subscribe: vi.fn(() => () => undefined),
}))
vi.mock('../api/shared-ws.js', () => ({ sharedWs: wsMocks }))

// 通知层的模块级 store：断言经 subscribeNotices 观察入栈条目。
import { resetNotices, subscribeNotices } from '../notify.js'
import { navigate } from '../router.js'

vi.mock('../router.js', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../router.js')>()
  return { ...actual, navigate: vi.fn() }
})

import type { SessionRow } from '../api/client.js'
import { ApiError } from '../api/client.js'
import './sessions.js'
import { reportActionRejection, type SebasSessions } from './sessions.js'

let seq = 0
function row(overrides: Partial<SessionRow> = {}): SessionRow {
  seq += 1
  return {
    encoded_key: `oc_${seq}%00`,
    thread_id: null,
    session_id: `aaaaaaaa-000${seq}`,
    session_id_short: `aaaa000${seq}`,
    status: 'done',
    status_label: 'Done',
    status_slug: 'done',
    status_glyph: '✓',
    last_active: '2m ago',
    last_active_unix: 1000 + seq,
    is_active: false,
    project_id: null,
    prompt_preview: null,
    current_model: null,
    available_models: null,
    agent_kind: null,
    pending_count: 0,
    msg_count: 0,
    turn_engaged: false,
    desired_mode: 'ask',
    ...overrides,
  }
}

const listPayload = (rows: SessionRow[]) => ({
  recent_sessions: rows,
  active_count: 0,
  dormant_count: 0,
  spawning_count: 0,
  total_sessions: rows.length,
  active_session_key: null,
})

async function mount(role: 'root' | 'member' | 'viewer' | null = null): Promise<SebasSessions> {
  const el = document.createElement('sebas-sessions') as SebasSessions
  if (role !== null) el.role = role
  document.body.appendChild(el)
  await el.updateComplete
  await new Promise((r) => setTimeout(r, 0))
  await el.updateComplete
  await new Promise((r) => setTimeout(r, 0))
  await el.updateComplete
  return el
}

beforeEach(() => {
  resetNotices()
  vi.clearAllMocks()
  apiMocks.sessions.mockResolvedValue(listPayload([row()]))
  apiMocks.projectsList.mockResolvedValue({
    projects: [{ id: 'proj-1', path: '/p', name: 'p', added_at: 0 }],
  })
  apiMocks.agents.mockResolvedValue({
    agents: [{ id: 'claude', display: 'Claude', reachable: true }],
  })
  apiMocks.createSession.mockResolvedValue({ key: 'oc_new%00' })
})

afterEach(() => {
  document.body.innerHTML = ''
  resetNotices()
  window.history.replaceState({}, '', '/')
})

describe('/sessions honors role visibility (round14 2.3, D-5-1)', () => {
  it('viewer sees no creation form and no per-card write buttons; the listing renders', async () => {
    const el = await mount('viewer')
    // 新建表单整体不呈现。
    expect(el.shadowRoot!.querySelector('form.composer')).toBeNull()
    // 只读列表照常渲染（卡片 + 标题链接）。
    const cards = [...el.shadowRoot!.querySelectorAll('article.scard')]
    expect(cards).toHaveLength(1)
    expect(cards[0]!.querySelector('a.chat')).toBeTruthy()
    // 写操作按钮（聚焦 = switch 写、关闭）不呈现。
    const buttons = [...cards[0]!.querySelectorAll('wa-button')].map((b) => b.textContent?.trim())
    expect(buttons).not.toContain('聚焦')
    expect(buttons).not.toContain('关闭')
    // 页面副标题如实说明只读口径。
    expect(el.shadowRoot!.textContent).toContain('只读总览')
    el.remove()
  })

  it('roles with sessions.write keep the full surface', async () => {
    const el = await mount('member')
    expect(el.shadowRoot!.querySelector('form.composer')).toBeTruthy()
    const buttons = [...el.shadowRoot!.querySelectorAll('article.scard wa-button')].map((b) =>
      b.textContent?.trim(),
    )
    expect(buttons).toContain('聚焦')
    expect(buttons).toContain('关闭')
    el.remove()
  })

  it('auth-disabled hosts (role null) keep the full surface', async () => {
    const el = await mount(null)
    expect(el.shadowRoot!.querySelector('form.composer')).toBeTruthy()
    el.remove()
  })
})

describe('rejected actions present explicitly; banner semantics split (round14 2.4/2.1, D-5-3)', () => {
  it('reportActionRejection turns a 403 into a permission-denied notice naming the restriction', () => {
    const notices: Array<{ level: string; message: string }> = []
    subscribeNotices((s) => {
      notices.length = 0
      notices.push(...s.items.map((i) => ({ level: i.level, message: i.message })))
    })
    reportActionRejection(
      new ApiError(403, '权限不足：viewer 角色无权执行该操作'),
    )
    expect(notices).toHaveLength(1)
    expect(notices[0]!.level).toBe('error')
    expect(notices[0]!.message).toContain('无权限')
    expect(notices[0]!.message).toContain('viewer 角色无权执行该操作')
  })

  it('reportActionRejection surfaces typed rejection copy (capacity) verbatim', () => {
    const notices: Array<{ level: string; message: string }> = []
    subscribeNotices((s) => {
      notices.length = 0
      notices.push(...s.items.map((i) => ({ level: i.level, message: i.message })))
    })
    reportActionRejection(new ApiError(400, '会话数已达上限 32'))
    expect(notices).toHaveLength(1)
    expect(notices[0]!.message).toContain('会话数已达上限 32')
  })

  it('a failed creation never replaces the listing with the load-failure banner', async () => {
    apiMocks.createSession.mockRejectedValue(new ApiError(400, '会话数已达上限 32'))
    const notices: string[] = []
    subscribeNotices((s) => {
      notices.length = 0
      notices.push(...s.items.map((i) => i.message))
    })
    const el = await mount('member')
    // 直接驱动创建路径（表单结构属 WA 组件细节，逐字段驱动归 GUI 面）。
    const privateEl = el as unknown as {
      prompt: string
      projectId: string
      create: (e: Event) => Promise<void>
    }
    privateEl.prompt = 'hello'
    privateEl.projectId = 'proj-1'
    await privateEl.create(new Event('submit'))
    await el.updateComplete

    // 列表保留（没有被「加载失败」横幅替换）。
    expect(el.shadowRoot!.querySelectorAll('article.scard')).toHaveLength(1)
    expect(el.shadowRoot!.querySelector('.callout-error')).toBeNull()
    // 类型化拒绝文案经 notice 层就地呈现（不产生幻影会话 URL）。
    expect(notices.some((m) => m.includes('会话数已达上限 32'))).toBe(true)
    expect(navigate).not.toHaveBeenCalled()
    el.remove()
  })

  it('a failed read renders the load-failure banner instead of the listing', async () => {
    apiMocks.sessions.mockRejectedValue(new ApiError(503, '核心不可达'))
    const el = await mount('viewer')
    expect(el.shadowRoot!.querySelector('.callout-error')).toBeTruthy()
    expect(el.shadowRoot!.textContent).toContain('加载失败')
    el.remove()
  })

  it('a successful creation navigates to the new session URL', async () => {
    const el = await mount('member')
    const privateEl = el as unknown as { prompt: string; projectId: string; create: (e: Event) => Promise<void> }
    privateEl.prompt = 'hello'
    privateEl.projectId = 'proj-1'
    await privateEl.create(new Event('submit'))
    expect(navigate).toHaveBeenCalledWith('/sessions/oc_new%00')
    el.remove()
  })
})
