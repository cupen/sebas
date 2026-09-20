/**
 * Tests for the sidebar project tree (project-rail.ts).
 *
 * Covers: project rows with counts, expand/collapse, session deep-links,
 * drag-to-reorder, add-project dialog, row action consolidation (… menu +
 * "+"), the unread badge (shared read cursor), session naming by first
 * prompt, History newest-first, and Inbox removal (rail-declutter-unread).
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { api, type Project, type SessionRow } from '../api/client.js'
import { writeFocusAnchor } from './unread-cursor.js'
import './project-rail.js'
import {
  SebasProjectRail,
  LABEL_REFRESH_DEBOUNCE_MS,
  RAIL_EXPANDED_KEY,
  addPathScopeHintFrom,
  fullSessionLabel,
  normalizeDisplayPath,
  parseRailExpanded,
  railExpandedDefault,
  serializeRailExpanded,
} from './project-rail.js'

// ---- localStorage polyfill --------------------------------------------
// 测试环境的全局没有 localStorage；徽标读锚走共享游标模块（写全局），用
// 内存 Map 替换以获得确定性行为（与 transcript-view.test.ts 同款）。
const seenStore = new Map<string, string>()
const ls = {
  getItem: (k: string) => seenStore.get(k) ?? null,
  setItem: (k: string, v: string) => {
    seenStore.set(k, v)
  },
  removeItem: (k: string) => {
    seenStore.delete(k)
  },
  clear: () => seenStore.clear(),
  key: () => null,
  get length() {
    return seenStore.size
  },
}
Object.defineProperty(globalThis, 'localStorage', { value: ls, configurable: true })

/**
 * 共享 WS 客户端 mock（session-parallel-liveness-and-unread-polish 2.2）：
 * subscribe 捕获 handler 供用例派发相位帧（emit），验证 rail 圆点与未读
 * 徽标从帧字段真读、不依赖 HTTP 列表刷新。
 */
const wsMocks = vi.hoisted(() => {
  const handlers = new Set<(ev: unknown) => void>()
  return {
    subscribe: vi.fn((h: (ev: unknown) => void) => {
      handlers.add(h)
      return () => handlers.delete(h)
    }),
    emit: (ev: unknown): void => {
      for (const h of handlers) h(ev)
    },
    clearHandlers: (): void => handlers.clear(),
  }
})
vi.mock('../api/shared-ws.js', () => ({ sharedWs: wsMocks }))

vi.mock('../api/client.js')

// vi.auto-mock 把方法变成了 vi.fn，但类型仍是真实 client 的形状——
// 统一经 `mockOf` 拿到可编排的 mock 类型，调用点保持与真 client 同形。
const mockOf = <F>(fn: F): ReturnType<typeof vi.fn> & F =>
  fn as unknown as ReturnType<typeof vi.fn> & F

const apiMock = vi.mocked(api)

const projects: Project[] = [
  { id: 'proj-alpha', path: '/home/me/alpha', name: 'alpha', added_at: 0 },
  { id: 'proj-beta', path: '/home/me/beta', name: 'beta', added_at: 1 },
]

let seq = 0
function row(overrides: Partial<SessionRow>): SessionRow {
  seq += 1
  const base: SessionRow = {
    encoded_key: `oc_${seq}%00`,
    chat_id: `chat-${seq}`,
    thread_id: null,
    session_id: `aaaaaaaa-000${seq}`,
    session_id_short: `aaaa000${seq}`,
    status: 'working',
    status_label: 'Working',
    status_slug: 'working',
    status_glyph: '●',
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
  }
  return { ...base, ...overrides }
}

const sessionRows: SessionRow[] = [
  row({ project_id: 'proj-alpha', status: 'working', status_slug: 'working' }),
  row({ project_id: 'proj-alpha', status: 'done', status_slug: 'done' }),
  row({ project_id: null, status: 'working', status_slug: 'working' }),
  row({ project_id: null, status: 'queued', status_slug: 'queued' }),
]

async function mount(): Promise<SebasProjectRail> {
  const el = document.createElement('sebas-project-rail') as SebasProjectRail
  document.body.appendChild(el)
  // The component calls void this.refresh() in connectedCallback — each
  // await inside refresh() queues a microtask. Flush them all.
  await new Promise((r) => setTimeout(r, 0))
  await el.updateComplete
  await new Promise((r) => setTimeout(r, 0))
  await el.updateComplete
  await new Promise((r) => setTimeout(r, 0))
  await el.updateComplete
  return el
}

const sessionList = (rows: SessionRow[]) => ({
  recent_sessions: rows,
  active_count: rows.filter((r) => r.status === 'active').length,
  dormant_count: 0,
  spawning_count: 0,
  total_sessions: rows.length,
  active_session_key: null,
})

beforeEach(() => {
  mockOf(apiMock.projects.list).mockResolvedValue({ projects })
  mockOf(apiMock.projects.branch).mockRejectedValue(new Error('no branch lookup here'))
  // Mock sessions.
  mockOf(apiMock.sessions).mockResolvedValue(sessionList(sessionRows))
  // Mock archive list (empty by default).
  mockOf(apiMock.archiveList).mockResolvedValue({ archived_sessions: [] })
  // add-remote-execution-node 8.2：节点可用性默认只有本机在线。
  mockOf(apiMock.nodes).mockResolvedValue({
    nodes: [{ id: 'local', status: 'online', local: true }],
    remote_available: true,
  })
})

beforeEach(() => {
  wsMocks.clearHandlers()
})

afterEach(() => {
  document.body.innerHTML = ''
  window.history.replaceState({}, '', '/')
  // 徽标读锚在 localStorage 里：用例间清空，防串扰。
  seenStore.clear()
})


// ── fix-webui-qa-defects 7.2：Add project 越界路径的禁用原因 ───────────────

describe('add-project scope hint (fix-webui-qa-defects 7.2 / 本 change 5.2)', () => {
  it('maps the server boundary rejection to a readable out-of-scope reason', () => {
    const hint = addPathScopeHintFrom('路径超出允许范围: 不在 workspace root 内')
    // （5.2）文案与 spec 对齐：越界点名列出边界。
    expect(hint).toContain('workspace root 之外')
    expect(hint).toContain('workspace root')
  })

  it('maps the browse-dirs boundary failure to the same reason', () => {
    expect(addPathScopeHintFrom('path outside the workspace root')).toBeTruthy()
  })

  it('nonexistent / not-a-directory now carry their own reasons (5.2 收口)', () => {
    expect(addPathScopeHintFrom('路径不存在或无法访问: /x')).toContain('不存在')
    expect(addPathScopeHintFrom('不是目录')).toContain('不是目录')
  })

  it('returns null for non-scope failures so the register call names them', () => {
    expect(addPathScopeHintFrom('路径不存在: /x')).toContain('不存在')
    expect(addPathScopeHintFrom('读取目录失败: …')).toBeNull()
  })
})

describe('sebas-project-rail (sidebar tree)', () => {
  it('renders project rows with live session counts from the fetched snapshot', async () => {
    const el = await mount()
    const rows = [...el.shadowRoot!.querySelectorAll('.row')]
    expect(rows).toHaveLength(2)
    expect(el.shadowRoot!.textContent).toContain('alpha')
    expect(el.shadowRoot!.textContent).toContain('beta')
    const counts = rows.map((r) => r.querySelector('.meta .count')?.textContent ?? '')
    // alpha has 2 sessions, beta has 0 (no sessions with project_dir=/home/me/beta)
    expect(counts).toEqual(['2', ''])
    expect(el.shadowRoot!.querySelector('.section-label .add-btn')).toBeTruthy()
    el.remove()
  })

  it('expands a project on click, emitting rail-select, and shows nested session rows', async () => {
    const el = await mount()
    const selected = vi.fn()
    el.addEventListener('rail-select', selected)

    const alphaRow = el.shadowRoot!.querySelectorAll('.row')[0] as HTMLElement
    alphaRow.click()
    await el.updateComplete

    expect(selected).toHaveBeenCalledTimes(1)
    expect((selected.mock.calls[0]![0] as CustomEvent).detail.path).toBe('/home/me/alpha')

    const items = [...el.shadowRoot!.querySelectorAll('li.session-item')]
    expect(items).toHaveLength(2)
    expect(el.shadowRoot!.textContent).toContain('aaaa0001')
    expect(items[0]!.querySelector('.session-dot')?.getAttribute('data-status')).toBe('working')
    expect(items[1]!.querySelector('.session-dot')?.getAttribute('data-status')).toBe('done')
    el.remove()
  })

  it('a session.updated frame alone advances the unread badge without any list refresh (2.2)', async () => {
    // 读锚先落在 0：帧带来的 3 段全部未读（无锚 = fully read，不冒未读）。
    writeFocusAnchor('oc_1%00', 0)
    const el = await mount()
    ;(el.shadowRoot!.querySelectorAll('.row')[0] as HTMLElement).click()
    await el.updateComplete
    const items = () => [...el.shadowRoot!.querySelectorAll('li.session-item')]
    expect(items()[0]!.querySelector('[data-testid="session-unread"]')).toBeNull()
    const listCallsBefore = mockOf(apiMock.sessions).mock.calls.length

    // 引擎 flip：可见回复段 +3。相位帧到达即打补丁——不刷新列表（徽标
    // 从帧字段真读，2.2），圆点同步翻 working；label 与行已知一致（同为
    // 空）→ 不调度行名重取（6.3 收窄）。
    wsMocks.emit({
      type: 'session.updated',
      session_id: 'oc_1%00',
      status_slug: 'working',
      turn_engaged: true,
      msg_count: 3,
      pending: [],
      label: null,
    })
    await el.updateComplete

    const badge = items()[0]!.querySelector('[data-testid="session-unread"]')
    expect(badge).toBeTruthy()
    expect(badge!.textContent).toBe('3')
    expect(mockOf(apiMock.sessions).mock.calls.length).toBe(listCallsBefore)
    // 圆点状态同帧翻转到 working（避免「读数新了、圆点还是旧的」的错位）。
    expect(items()[0]!.querySelector('.session-dot')?.getAttribute('data-status')).toBe('working')
    el.remove()
  })

  it('unread rows carry the emphasis tint and read rows do not (2.4, D4)', async () => {
    // 先写读锚再让计数超过它：第一行有 2 条未读，第二行无未读。
    const unreadRow = row({ project_id: 'proj-alpha', msg_count: 3 })
    const readRow = row({ project_id: 'proj-alpha', status_slug: 'done', msg_count: 3 })
    writeFocusAnchor(unreadRow.encoded_key, 1)
    writeFocusAnchor(readRow.encoded_key, 3)
    mockOf(apiMock.sessions).mockResolvedValue(sessionList([unreadRow, readRow]))
    const el = await mount()
    ;(el.shadowRoot!.querySelectorAll('.row')[0] as HTMLElement).click()
    await el.updateComplete
    const items = [...el.shadowRoot!.querySelectorAll('li.session-item')]
    expect(items[0]!.className).toContain('unread')
    expect(items[0]!.querySelector('[data-testid="session-unread"]')).toBeTruthy()
    expect(items[1]!.className).not.toContain('unread')
    expect(items[1]!.querySelector('[data-testid="session-unread"]')).toBeNull()
    // 样式钉死：未读行用 accent-soft 族 tint（在读数字之前可分辨）。
    // happy-dom 下 Lit 走 adoptedStyleSheets，样式文本从 cssResult 取。
    const cssText = [SebasProjectRail.styles]
      .flat()
      .map((c) => (c as unknown as { cssText?: string }).cssText ?? '')
      .join('\n')
    expect(cssText).toMatch(
      /\.session-item\.unread:not\(\.current\)\s*\{[^}]*background:\s*var\(--sebas-accent-soft\)/,
    )
    el.remove()
  })

  it('clicking a session switches focus IN PLACE and stays on the workbench (3.1)', async () => {
    mockOf(apiMock.switchSession).mockResolvedValue({
      status: 'switched',
      redirect: '/sessions/oc_1%00',
      active_session_key: 'oc_1%00',
    })
    window.history.replaceState({}, '', '/')
    const el = await mount()
    ;(el.shadowRoot!.querySelectorAll('.row')[0] as HTMLElement).click()
    await el.updateComplete

    const first = el.shadowRoot!.querySelector('li.session-item') as HTMLElement
    first.click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    // switch 端点被调用；操作员不被导航走——留在工作台。
    expect(apiMock.switchSession).toHaveBeenCalledWith('oc_1%00')
    expect(window.location.pathname).toBe('/')
    el.remove()
  })

  it('the current-session marker follows the focus pointer, not the location (3.2)', async () => {
    mockOf(apiMock.switchSession).mockResolvedValue({
      status: 'switched',
      redirect: '/sessions/oc_2%00',
      active_session_key: 'oc_2%00',
    })
    // 浏览器位置在旧深链上，焦点指针却指向另一会话：标记必须看指针。
    window.history.replaceState({}, '', '/sessions/oc_1%00')
    mockOf(apiMock.sessions).mockResolvedValue({
      ...sessionList(sessionRows),
      active_session_key: 'oc_2%00',
    })
    const el = await mount()
    await el.updateComplete
    // （4.2）焦点指针指向 oc_2（proj-alpha）→ alpha 组缺省展开并物化，
    // 无需再点项目行；点击反而会 toggle 收起。
    const items = [...el.shadowRoot!.querySelectorAll('li.session-item')]
    expect(items[0]!.classList.contains('current')).toBe(false)
    expect(items[1]!.classList.contains('current')).toBe(true)
    expect(items[1]!.getAttribute('aria-current')).toBe('true')
    el.remove()
  })

  it('marks the focused session current after an in-place switch (3.1+3.2)', async () => {
    mockOf(apiMock.switchSession).mockResolvedValue({
      status: 'switched',
      redirect: '/sessions/oc_1%00',
      active_session_key: 'oc_1%00',
    })
    const el = await mount()
    ;(el.shadowRoot!.querySelectorAll('.row')[0] as HTMLElement).click()
    await el.updateComplete
    const first = el.shadowRoot!.querySelector('li.session-item') as HTMLElement
    first.click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    expect((el as unknown as { focusedKey: string | null }).focusedKey).toBe('oc_1%00')
    const items = [...el.shadowRoot!.querySelectorAll('li.session-item')]
    expect(items[0]!.classList.contains('current')).toBe(true)
    el.remove()
  })

  it('keeps drag-to-reorder persistence via POST /api/projects/reorder', async () => {
    mockOf(apiMock.projects.reorder).mockResolvedValue({
      projects: [projects[1], projects[0]],
    })
    const el = await mount()
    const rows = () => [...el.shadowRoot!.querySelectorAll('.row')]
    expect(rows()[0]!.textContent).toContain('alpha')

    rows()[0]!.dispatchEvent(new Event('dragstart', { bubbles: true }))
    rows()[1]!.dispatchEvent(new Event('drop', { bubbles: true }))
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete

    expect(apiMock.projects.reorder).toHaveBeenCalledWith(['proj-beta', 'proj-alpha'])
    expect(rows()[0]!.textContent).toContain('beta')
    el.remove()
  })

  it('opens add-project dialog when the + button is clicked', async () => {
    mockOf(apiMock.fsBrowse).mockResolvedValue({ path: '/home/me', entries: [{ name: 'alpha', is_dir: true }] })
    const el = await mount()
    ;(el.shadowRoot!.querySelector('.section-label .add-btn') as HTMLElement).click()
    await el.updateComplete
    // Should have a wa-dialog.
    const dialog = el.shadowRoot!.querySelector('wa-dialog')
    expect(dialog).toBeTruthy()
    // wa-dialog uses .open property, not the open attribute
    expect((dialog as any).open).toBe(true)
    el.remove()
  })
})

describe('inbox removal (rail-declutter-unread 4.1)', () => {
  it('renders no Inbox group even when project-less sessions exist', async () => {
    const el = await mount()
    // 无项目会话（project_id null）不再有任何 rail 展示位：Inbox 组不存在，
    // 会话也不落在任何分组里。
    const heads = [...el.shadowRoot!.querySelectorAll('.group-head')].map((h) => h.textContent)
    expect(heads.some((t) => t?.includes('Inbox'))).toBe(false)
    ;(el.shadowRoot!.querySelectorAll('.row')[0] as HTMLElement).click()
    await el.updateComplete
    const items = [...el.shadowRoot!.querySelectorAll('li.session-item')]
    expect(items).toHaveLength(2)
    expect(items.every((i) => i.textContent!.includes('aaaa'))).toBe(true)
    el.remove()
  })

  it('unbound sessions are invisible even when they are the only sessions', async () => {
    mockOf(apiMock.sessions).mockResolvedValue(sessionList([sessionRows[2]]))
    const el = await mount()
    expect(el.shadowRoot!.querySelector('.group-head')).toBeNull()
    expect(el.shadowRoot!.querySelector('li.session-item')).toBeNull()
    el.remove()
  })
})

describe('row action consolidation (rail-declutter-unread 3.1/3.2)', () => {
  it('project row exposes the … menu first, then the + button; … opens the remove dialog', async () => {
    const el = await mount()
    const actions = el.shadowRoot!.querySelector('.row .row-actions')!
    const children = [...actions.children]
    // 顺序固定：… 下拉在前，+ 在后；行内不再有平铺的移除按钮。
    expect(children[0]!.tagName.toLowerCase()).toBe('wa-dropdown')
    expect(children[1]!.tagName.toLowerCase()).toBe('button')
    expect(children[1]!.getAttribute('aria-label')).toBe('New session in alpha')
    expect(el.shadowRoot!.querySelector('.row .row-remove')).toBeNull()

    // … 菜单含「移除项目」；点击菜单项打开移除弹窗。
    const item = el.shadowRoot!.querySelector('wa-dropdown-item[value="remove"]') as HTMLElement
    item.click()
    await el.updateComplete
    const dialog = el.shadowRoot!.querySelector('wa-dialog[label="Remove project"]')
    expect((dialog as any).open).toBe(true)
    el.remove()
  })

  it('session row has a single … menu whose only lifecycle entry is Archive (danger) (4.2)', async () => {
    const el = await mount()
    ;(el.shadowRoot!.querySelectorAll('.row')[0] as HTMLElement).click()
    await el.updateComplete
    const item = el.shadowRoot!.querySelector('li.session-item')!
    // 行内不再有直删/归档按钮。
    expect(item.querySelector('button[title="Archive this session"]')).toBeNull()
    expect(item.querySelector('button[title="Close (delete) this session"]')).toBeNull()
    // 唯一的 … 菜单：归档（danger，唯一出口）——close 菜单项已并入归档语义。
    expect(item.querySelectorAll('wa-dropdown').length).toBe(1)
    const archive = item.querySelector('wa-dropdown-item[value="archive"]')
    const close = item.querySelector('wa-dropdown-item[value="close"]')
    expect(archive).toBeTruthy()
    expect(close).toBeNull()
    expect(archive!.getAttribute('variant')).toBe('danger')
    el.remove()
  })

  it('the … menu archives through the confirm dialog (4.2)', async () => {
    mockOf(apiMock.archiveSession).mockResolvedValue({ status: 'archived' })
    const el = await mount()
    ;(el.shadowRoot!.querySelectorAll('.row')[0] as HTMLElement).click()
    await el.updateComplete
    const item = el.shadowRoot!.querySelector('li.session-item')!
    ;(item.querySelector('wa-dropdown-item[value="archive"]') as HTMLElement).click()
    await el.updateComplete
    // 确认框弹出（归档一律确认——「将丢弃 N 条待执行」的告知不依赖状态）。
    const dialog = el.shadowRoot!.querySelector('wa-dialog[label="归档会话"]')
    expect((dialog as any).open).toBe(true)
    expect(apiMock.archiveSession).not.toHaveBeenCalled()
    ;([...(dialog as HTMLElement).querySelectorAll('wa-button')].find(
      (b) => b.textContent?.trim() === '归档',
    ) as HTMLElement).click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    expect(apiMock.archiveSession).toHaveBeenCalledWith('oc_1%00')
    el.remove()
  })

  it('archive always confirms first, whatever the session state (4.2)', async () => {
    mockOf(apiMock.archiveSession).mockResolvedValue({ status: 'archived' })
    const dormant = row({ project_id: 'proj-alpha', status: 'done', status_slug: 'done' })
    const working = row({ project_id: 'proj-alpha', status: 'working', status_slug: 'working' })
    mockOf(apiMock.sessions).mockResolvedValue(sessionList([dormant, working]))
    const el = await mount()
    ;(el.shadowRoot!.querySelectorAll('.row')[0] as HTMLElement).click()
    await el.updateComplete
    const items = [...el.shadowRoot!.querySelectorAll('li.session-item')]
    // inactive：确认框弹出，不直接归档。
    ;(items[0]!.querySelector('wa-dropdown-item[value="archive"]') as HTMLElement).click()
    await el.updateComplete
    let dialog = el.shadowRoot!.querySelector('wa-dialog[label="归档会话"]')
    expect((dialog as any).open).toBe(true)
    ;([...(dialog as HTMLElement).querySelectorAll('wa-button')].find(
      (b) => b.textContent?.trim() === '归档',
    ) as HTMLElement).click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    // happy-dom 下 wa-button 的合成 click 会双发；组件侧有重入护栏，
    // 这里只断言归档确实经确认框执行（严格计数见组件单测）。
    expect(apiMock.archiveSession).toHaveBeenCalled()
    // active：同样先确认。
    ;(items[1]!.querySelector('wa-dropdown-item[value="archive"]') as HTMLElement).click()
    await el.updateComplete
    dialog = el.shadowRoot!.querySelector('wa-dialog[label="归档会话"]')
    expect((dialog as any).open).toBe(true)
    el.remove()
  })
})

describe('unread badge (rail-declutter-unread 2.3)', () => {
  it('shows no badge without a stored anchor (first visit = fully read)', async () => {
    const unreadRow = row({ project_id: 'proj-alpha', msg_count: 7 })
    mockOf(apiMock.sessions).mockResolvedValue(sessionList([unreadRow]))
    const el = await mount()
    ;(el.shadowRoot!.querySelectorAll('.row')[0] as HTMLElement).click()
    await el.updateComplete
    expect(el.shadowRoot!.querySelector('[data-testid="session-unread"]')).toBeNull()
    el.remove()
  })

  it('shows msg_count − anchor_count and clears when focus advances the anchor', async () => {
    const unreadRow = row({ project_id: 'proj-alpha', msg_count: 120 })
    mockOf(apiMock.sessions).mockResolvedValue(sessionList([unreadRow]))
    // 预置读锚 = 118 → 未读 2。
    writeFocusAnchor(unreadRow.encoded_key, 118)
    const el = await mount()
    ;(el.shadowRoot!.querySelectorAll('.row')[0] as HTMLElement).click()
    await el.updateComplete
    const badge = el.shadowRoot!.querySelector('[data-testid="session-unread"]')
    expect(badge?.textContent?.trim()).toBe('2')

    // 聚焦成功 = 写锚推进到当前 count → 徽标消失。
    mockOf(apiMock.switchSession).mockResolvedValue({ status: 'switched' })
    const item = el.shadowRoot!.querySelector('li.session-item') as HTMLElement
    item.click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    expect(el.shadowRoot!.querySelector('[data-testid="session-unread"]')).toBeNull()
    el.remove()
  })

  it('caps the displayed number at 99+', async () => {
    const unreadRow = row({ project_id: 'proj-alpha', msg_count: 300 })
    mockOf(apiMock.sessions).mockResolvedValue(sessionList([unreadRow]))
    writeFocusAnchor(unreadRow.encoded_key, 1)
    const el = await mount()
    ;(el.shadowRoot!.querySelectorAll('.row')[0] as HTMLElement).click()
    await el.updateComplete
    expect(el.shadowRoot!.querySelector('[data-testid="session-unread"]')?.textContent?.trim()).toBe('99+')
    el.remove()
  })
})

describe('session naming by first prompt (rail-declutter-unread 3.4)', () => {
  it('names rows by prompt_preview and carries the full text in the title', async () => {
    const named = row({
      project_id: 'proj-alpha',
      prompt_preview: '帮我重构 rail 组件的渲染逻辑',
    })
    mockOf(apiMock.sessions).mockResolvedValue(sessionList([named]))
    const el = await mount()
    ;(el.shadowRoot!.querySelectorAll('.row')[0] as HTMLElement).click()
    await el.updateComplete
    const item = el.shadowRoot!.querySelector('li.session-item')!
    expect(item.querySelector('.session-name')?.textContent).toBe('帮我重构 rail 组件的渲染逻辑')
    expect(item.getAttribute('title')).toContain('帮我重构 rail 组件的渲染逻辑')
    el.remove()
  })

  it('truncates long previews at 40 code points with an ellipsis; title keeps the full text', async () => {
    const long = 'x'.repeat(60) + '尾巴'
    const named = row({ project_id: 'proj-alpha', prompt_preview: long })
    mockOf(apiMock.sessions).mockResolvedValue(sessionList([named]))
    const el = await mount()
    ;(el.shadowRoot!.querySelectorAll('.row')[0] as HTMLElement).click()
    await el.updateComplete
    const item = el.shadowRoot!.querySelector('li.session-item')!
    const name = item.querySelector('.session-name')?.textContent ?? ''
    expect([...name].length).toBe(41) // 40 码点 + …
    expect(name.endsWith('…')).toBe(true)
    expect(item.getAttribute('title')).toBe(long)
    el.remove()
  })

  it('zero-turn placeholders fall back to the short identifier', async () => {
    const placeholder = row({
      project_id: 'proj-alpha',
      prompt_preview: null,
      session_id_short: 'aaaa0009',
    })
    mockOf(apiMock.sessions).mockResolvedValue(sessionList([placeholder]))
    const el = await mount()
    ;(el.shadowRoot!.querySelectorAll('.row')[0] as HTMLElement).click()
    await el.updateComplete
    expect(
      el.shadowRoot!.querySelector('li.session-item .session-name')?.textContent,
    ).toBe('aaaa0009')
    el.remove()
  })

  it('the archive confirmation names the session by the same label', async () => {
    const working = row({
      project_id: 'proj-alpha',
      status: 'working',
      status_slug: 'working',
      prompt_preview: '确认弹窗里的名字',
      pending_count: 2,
    })
    mockOf(apiMock.sessions).mockResolvedValue(sessionList([working]))
    const el = await mount()
    ;(el.shadowRoot!.querySelectorAll('.row')[0] as HTMLElement).click()
    await el.updateComplete
    const item = el.shadowRoot!.querySelector('li.session-item')!
    ;(item.querySelector('wa-dropdown-item[value="archive"]') as HTMLElement).click()
    await el.updateComplete
    const dialog = el.shadowRoot!.querySelector('wa-dialog[label="归档会话"]')
    expect(dialog?.textContent).toContain('确认弹窗里的名字')
    const line = el.shadowRoot!.querySelector('[data-testid="close-discards-pending"]')
    expect(line?.textContent).toContain('2')
    el.remove()
  })
})

describe('project removal precheck (rail-declutter-unread 3.3)', () => {
  it('prechecks live sessions and states the count inline; backend rejection surfaces too', async () => {
    const el = await mount()
    // alpha 有 2 个非归档会话：弹窗就地说明。
    ;(el as unknown as { openRemoveDialog: (e: Event, p: Project) => void }).openRemoveDialog(
      new Event('click'),
      projects[0],
    )
    await el.updateComplete
    const blocked = el.shadowRoot!.querySelector('[data-testid="remove-blocked"]')
    expect(blocked?.textContent).toContain('2')
    expect(blocked?.textContent).toContain('归档')
    // 旧「迁移 Inbox」文案废除。
    expect(el.shadowRoot!.textContent).not.toContain('迁移到 Inbox')

    // 后端 typed rejection 内联呈现（预检只是提醒，强制在后端）。
    mockOf(apiMock.projects.remove).mockRejectedValue(
      new Error('项目下仍有 2 个未归档会话，请先归档或关闭它们再移除项目'),
    )
    await (el as unknown as { confirmRemoveProject: () => Promise<void> }).confirmRemoveProject()
    await el.updateComplete
    expect(apiMock.projects.remove).toHaveBeenCalledWith('proj-alpha')
    expect(el.shadowRoot!.textContent).toContain('请先归档或关闭')
    el.remove()
  })

  it('shows the plain copy when no live sessions remain', async () => {
    mockOf(apiMock.sessions).mockResolvedValue(sessionList([]))
    const el = await mount()
    ;(el as unknown as { openRemoveDialog: (e: Event, p: Project) => void }).openRemoveDialog(
      new Event('click'),
      projects[0],
    )
    await el.updateComplete
    expect(el.shadowRoot!.querySelector('[data-testid="remove-blocked"]')).toBeNull()
    expect(el.shadowRoot!.textContent).toContain('此操作只解除注册')
    el.remove()
  })
})

describe('history group (archived sessions)', () => {
  it('lists archived sessions newest-first by archive time (rail-declutter-unread 4.2)', async () => {
    mockOf(apiMock.archiveList).mockResolvedValue({
      archived_sessions: [
        { session_key: 'oc_old%00', project_path: '/home/me/alpha', label: 'Old session', archived_at: 1000, retention_deadline: 2000 },
        { session_key: 'oc_new%00', project_path: '/home/me/alpha', label: 'New session', archived_at: 5000, retention_deadline: 6000 },
        { session_key: 'oc_mid%00', project_path: '/home/me/alpha', label: 'Mid session', archived_at: 3000, retention_deadline: 4000 },
      ],
    })
    const el = await mount()
    const heads = [...el.shadowRoot!.querySelectorAll('.group-head')]
    const historyHead = heads.find((h) => h.textContent?.includes('History'))
    expect(historyHead).toBeTruthy()
    expect(historyHead!.querySelector('.group-count')?.textContent).toBe('3')
    // （5.5）组头是原生 <button>：role=button 语义来自元素本身（保留
    // Enter/Space 键盘行为），不再是 div[role=button]。
    expect(historyHead!.tagName.toLowerCase()).toBe('button')
    expect(historyHead!.getAttribute('role')).toBeNull()
    ;(historyHead as HTMLElement).click()
    await el.updateComplete
    const labels = [...el.shadowRoot!.querySelectorAll('.group-section li.session-item.archived .session-name')].map(
      (n) => n.textContent,
    )
    expect(labels).toEqual(['New session', 'Mid session', 'Old session'])
    el.remove()
  })

  it('does not render a branch name in the project row (D8), probing still runs', async () => {
    mockOf(apiMock.projects.branch).mockResolvedValue({
      project_id: 'proj-alpha',
      branch: 'feat/webui',
      accessible: true,
    })
    const el = await mount()
    await el.refresh()
    await el.updateComplete
    expect(el.shadowRoot!.querySelector('.row .branch')).toBeNull()
    expect(el.shadowRoot!.textContent).not.toContain('feat/webui')
    // 探测链路保留：branch 接口仍被调用（删除线告警依赖它）。
    expect(apiMock.projects.branch).toHaveBeenCalledWith('proj-alpha')
    el.remove()
  })

  it('archive meta shows the basename for Windows backslash paths, not the whole path (round4 3.2)', async () => {
    mockOf(apiMock.archiveList).mockResolvedValue({
      archived_sessions: [
        {
          session_key: 'oc_win%00',
          project_path: 'C:\\workbench\\repos-ai\\sebas',
          label: 'win session',
          archived_at: 5000,
          retention_deadline: 9000,
        },
      ],
    })
    const el = await mount()
    const heads = [...el.shadowRoot!.querySelectorAll('.group-head')]
    const historyHead = heads.find((h) => h.textContent?.includes('History'))
    ;(historyHead as HTMLElement).click()
    await el.updateComplete
    const meta = el.shadowRoot!.querySelector(
      '.group-section li.session-item.archived .archive-meta',
    )
    expect(meta?.textContent).toBe('sebas')
    el.remove()
  })

  it('archive meta and session name truncate with ellipsis instead of stretching the rail (round4 3.2)', async () => {
    const { readFileSync } = await import('node:fs')
    const { join, dirname } = await import('node:path')
    const here = dirname(new URL(import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1'))
    const src = readFileSync(join(here, 'project-rail.ts'), 'utf8')
    // .archive-meta 必须自带省略号截断（min-width:0 是 flex 子项省略号的
    // 生效前提）——长路径不撑出横向滚动。
    expect(src).toMatch(
      /\.archive-meta\s*\{[^}]*min-width:\s*0;[^}]*overflow:\s*hidden;[^}]*text-overflow:\s*ellipsis;[^}]*white-space:\s*nowrap;/,
    )
  })
})

describe('project registration degraded hint (harden-core-channel-deployment 4.3)', () => {
  it('shows the honest degraded hint when the core is unreachable and the add lands in the local registry', async () => {
    mockOf(apiMock.projects.add).mockResolvedValue({
      ...projects[0],
      degraded: { cause: 'socket absent' },
    } as any)
    const el = await mount()
    expect(el.shadowRoot!.querySelector('[data-testid="project-degraded-hint"]')).toBeNull()

    // Drive the same submit path the dialog's Add button uses.
    ;(el as any).addPath = '/home/me/alpha'
    await (el as any).submitAddProject()
    await el.updateComplete

    const hint = el.shadowRoot!.querySelector<HTMLElement>(
      '[data-testid="project-degraded-hint"]',
    )
    expect(hint).toBeTruthy()
    expect(hint!.textContent).toContain('核心不可达')
    expect(hint!.textContent).toContain('已写入本地注册表')
    el.remove()
  })

  it('shows no hint for a healthy (status-store) registration, and a later refresh clears a stale hint', async () => {
    mockOf(apiMock.projects.add).mockResolvedValue({ ...projects[0] } as any)
    const el = await mount()
    ;(el as any).addPath = '/home/me/alpha'
    await (el as any).submitAddProject()
    await el.updateComplete
    expect(el.shadowRoot!.querySelector('[data-testid="project-degraded-hint"]')).toBeNull()
    el.remove()
  })

  it('clears the degraded hint once the core recovers (refresh-driven)', async () => {
    mockOf(apiMock.projects.add).mockResolvedValue({
      ...projects[0],
      degraded: { cause: 'socket absent' },
    } as any)
    const el = await mount()
    ;(el as any).addPath = '/home/me/alpha'
    await (el as any).submitAddProject()
    await el.updateComplete
    expect(el.shadowRoot!.querySelector('[data-testid="project-degraded-hint"]')).toBeTruthy()

    // core 恢复 → 任一次成功 refresh（ws refetch / 重试）即清除降级态。
    await el.refresh()
    await el.updateComplete
    expect(el.shadowRoot!.querySelector('[data-testid="project-degraded-hint"]')).toBeNull()
    el.remove()
  })
})

/**
 * add-remote-execution-node 8.1/8.2/8.4/8.5：项目带节点维度注册、离线呈现与
 * composer 阻止、免刷新恢复、等待分组。
 */
describe('project node dimension (add-remote-execution-node 8.1/8.2)', () => {
  const remoteNode = { id: 'dev-box', status: 'online', created_unix: 1 }
  const localNode = { id: 'local', status: 'online', local: true }

  it('registers a project against a named node', async () => {
    mockOf(apiMock.nodes).mockResolvedValue({ nodes: [localNode, remoteNode], remote_available: true })
    mockOf(apiMock.projects.add).mockResolvedValue({
      id: 'proj-remote',
      path: '/srv/repo',
      name: 'repo',
      node_id: 'dev-box',
      added_at: 0,
    } as any)
    const el = await mount()
    ;(el as any).addDialogOpen = true
    ;(el as any).addPath = '/srv/repo'
    ;(el as any).addNodeId = 'dev-box'
    await el.updateComplete
    await (el as any).submitAddProject()
    await el.updateComplete
    expect(apiMock.projects.add).toHaveBeenCalledWith('/srv/repo', 'dev-box')
    el.remove()
  })

  it('registers without a node against the local node (implicit)', async () => {
    mockOf(apiMock.projects.add).mockResolvedValue({ ...projects[0] } as any)
    const el = await mount()
    ;(el as any).addPath = '/home/me/alpha'
    ;(el as any).addNodeId = ''
    await (el as any).submitAddProject()
    await el.updateComplete
    // 缺省不带 node_id：本机隐式注册的既有行为不变。
    expect(apiMock.projects.add).toHaveBeenCalledWith('/home/me/alpha', null)
    el.remove()
  })

  it('renders the same path on two nodes as two distinct entries naming their nodes', async () => {
    mockOf(apiMock.projects.list).mockResolvedValue({
      projects: [
        { id: 'proj-a', path: '/srv/repo', name: 'repo', added_at: 0, node_id: 'node-1' },
        { id: 'proj-b', path: '/srv/repo', name: 'repo', added_at: 1, node_id: 'node-2' },
      ],
    })
    mockOf(apiMock.nodes).mockResolvedValue({
      nodes: [localNode, { id: 'node-1', status: 'online' }, { id: 'node-2', status: 'online' }],
      remote_available: true,
    })
    const el = await mount()
    const rows = [...el.shadowRoot!.querySelectorAll('.row')]
    expect(rows).toHaveLength(2)
    const nodeLabels = [...el.shadowRoot!.querySelectorAll('[data-testid="project-node"]')].map(
      (n) => n.textContent?.trim(),
    )
    expect(nodeLabels).toContain('node-1')
    expect(nodeLabels).toContain('node-2')
    el.remove()
  })

  it('surfaces the node-side rejection naming the node and the path', async () => {
    mockOf(apiMock.nodes).mockResolvedValue({ nodes: [localNode, remoteNode], remote_available: true })
    mockOf(apiMock.projects.add).mockRejectedValue(
      new Error('节点 dev-box 上路径不存在: /srv/repo'),
    )
    const el = await mount()
    ;(el as any).addDialogOpen = true
    ;(el as any).addPath = '/srv/repo'
    ;(el as any).addNodeId = 'dev-box'
    await el.updateComplete
    await (el as any).submitAddProject()
    await el.updateComplete
    expect(el.shadowRoot!.textContent).toContain('节点 dev-box')
    expect(el.shadowRoot!.textContent).toContain('/srv/repo')
    el.remove()
  })

  it('marks an offline node project with its cause and blocks starting a session', async () => {
    mockOf(apiMock.projects.list).mockResolvedValue({
      projects: [{ id: 'proj-r', path: '/srv/repo', name: 'repo', added_at: 0, node_id: 'dev-box' }],
    })
    mockOf(apiMock.nodes).mockResolvedValue({
      nodes: [localNode, { id: 'dev-box', status: 'offline', last_seen_unix: 1_700_000_000 }],
      remote_available: true,
    })
    const el = await mount()

    const cause = el.shadowRoot!.querySelector<HTMLElement>('[data-testid="project-node-cause"]')
    expect(cause).toBeTruthy()
    expect(cause!.textContent).toContain('dev-box')
    expect(cause!.textContent).toContain('离线')

    const plus = el.shadowRoot!.querySelector<HTMLButtonElement>(
      'button[aria-label^="New session"]',
    )
    expect(plus!.disabled).toBe(true)

    // 离线节点的项目不可达创建入口（workbench-interaction-polish 3.2：创建
    // 唯一入口是「+」→ 对话框）——「+」禁用即拦截；成因点名节点。
    plus!.click()
    await el.updateComplete
    expect(apiMock.createSession).not.toHaveBeenCalled()
    // 「+」禁用即拦截：对话框根本没有打开（5.3 起关闭 = 不渲染）。
    expect(el.shadowRoot!.querySelector('sebas-new-session-dialog')).toBeNull()
    expect(el.shadowRoot!.textContent).toContain('dev-box')
    el.remove()
  })

  it('recovers without a page reload once the node is back (same element)', async () => {
    mockOf(apiMock.projects.list).mockResolvedValue({
      projects: [{ id: 'proj-r', path: '/srv/repo', name: 'repo', added_at: 0, node_id: 'dev-box' }],
    })
    mockOf(apiMock.nodes).mockResolvedValue({
      nodes: [localNode, { id: 'dev-box', status: 'offline' }],
      remote_available: true,
    })
    const el = await mount()
    expect(el.shadowRoot!.querySelector('[data-testid="project-node-cause"]')).toBeTruthy()

    // 节点回来（轮询会走同一条 refresh 路径——不重挂组件、不刷新页面）。
    mockOf(apiMock.nodes).mockResolvedValue({
      nodes: [localNode, { id: 'dev-box', status: 'online' }],
      remote_available: true,
    })
    await el.refresh()
    await el.updateComplete
    expect(el.shadowRoot!.querySelector('[data-testid="project-node-cause"]')).toBeNull()
    const plus = el.shadowRoot!.querySelector<HTMLButtonElement>(
      'button[aria-label^="New session"]',
    )
    expect(plus!.disabled).toBe(false)
    el.remove()
  })

  it('groups sessions parked on an approval as waiting, distinguishable from working', async () => {
    const waitingRow = row({
      status: 'active',
      status_slug: 'waiting',
      status_label: 'Waiting',
      project_id: 'proj-alpha',
      remote: {
        node_id: 'dev-box',
        node_status: 'online',
        parked_approvals: 2,
      },
    })
    mockOf(apiMock.sessions).mockResolvedValue(sessionList([sessionRows[0], waitingRow]))
    mockOf(apiMock.nodes).mockResolvedValue({ nodes: [localNode, remoteNode], remote_available: true })
    const el = await mount()

    const heads = [...el.shadowRoot!.querySelectorAll('.group-head')]
    const waitingHead = heads.find((h) => h.textContent?.includes('Waiting on you'))
    expect(waitingHead).toBeTruthy()
    expect(waitingHead!.querySelector('.group-count')?.textContent).toBe('1')

    const badge = el.shadowRoot!.querySelector<HTMLElement>('[data-testid="session-waiting"]')
    expect(badge).toBeTruthy()
    expect(badge!.textContent).toContain('2')

    const dots = [...el.shadowRoot!.querySelectorAll('.session-dot')].map((d) =>
      d.getAttribute('data-status'),
    )
    expect(dots).toContain('waiting')
    el.remove()
  })

  it('a fresh placeholder session does not light the project wait-dot (3.6 regression)', async () => {
    // 全新占位/排队会话没有待审批——「需介入」橙点不得误报（区分审批挂起
    // 与占位/排队态；perm 挂起才亮橙点，既有 waiting 用例已覆盖）。
    const placeholder = row({
      project_id: 'proj-alpha',
      prompt_preview: null,
      session_id_short: 'aaaa0009',
      status: 'queued',
      status_slug: 'queued',
      status_label: 'Queued',
      remote: null,
    })
    mockOf(apiMock.sessions).mockResolvedValue(sessionList([placeholder]))
    mockOf(apiMock.nodes).mockResolvedValue({ nodes: [localNode], remote_available: true })
    const el = await mount()
    expect(el.shadowRoot!.querySelector('.wait-dot')).toBeNull()
    expect(el.shadowRoot!.querySelector('[data-testid="session-waiting"]')).toBeNull()
    el.remove()
  })
})

// ─── workbench-interaction-polish 3.2：项目行「+」→ 创建对话框 ────────────────

describe('creation dialog wiring (workbench-interaction-polish 3.2)', () => {
  beforeEach(() => {
    // 本文件的整体 beforeEach 不清 mock（既有用例依赖自定义编排）——
    // 创建面板用例各自关心 createSession 的调用记录，这里统一清。
    mockOf(apiMock.createSession).mockClear()
  })

  async function dialogOf(el: SebasProjectRail) {
    const dialog = el.shadowRoot!.querySelector(
      'sebas-new-session-dialog',
    ) as unknown as HTMLElement & {
      updateComplete: Promise<boolean>
      open: boolean
      projectId: string | null
      projectName: string | null
      defaultAgent: string | null
    }
    await dialog.updateComplete
    return dialog
  }

  it('opens the dialog from a project row + button, bound to that project', async () => {
    mockOf(apiMock.nodes).mockResolvedValue({
      nodes: [{ id: 'local', status: 'online', local: true }],
      remote_available: true,
    })
    const el = await mount()
    const plus = el.shadowRoot!.querySelector<HTMLButtonElement>(
      'button[aria-label="New session in alpha"]',
    )!
    plus.click()
    await el.updateComplete
    const dialog = await dialogOf(el)
    expect(dialog.open).toBe(true)
    expect(dialog.projectId).toBe('proj-alpha')
    expect(dialog.projectName).toBe('alpha')
    el.remove()
  })

  it('confirms creation with the dialog detail and keeps the workbench route', async () => {
    mockOf(apiMock.createSession).mockResolvedValue({ key: 'oc_new' })
    mockOf(apiMock.nodes).mockResolvedValue({
      nodes: [{ id: 'local', status: 'online', local: true }],
      remote_available: true,
    })
    const el = await mount()
    ;(
      el.shadowRoot!.querySelector('button[aria-label="New session in alpha"]') as HTMLButtonElement
    ).click()
    await el.updateComplete
    const dialog = await dialogOf(el)
    dialog.dispatchEvent(
      new CustomEvent('dialog-confirm', {
        detail: { agent: 'codex', model: 'deepseek-chat', mode: 'allow' },
        bubbles: true,
        composed: true,
      }),
    )
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete

    expect(mockOf(apiMock.createSession)).toHaveBeenCalledWith({
      projectId: 'proj-alpha',
      agent: 'codex',
      model: 'deepseek-chat',
      mode: 'allow',
    })
    // 成功后对话框收起、无错误。（5.3）关闭即整棵移出 DOM，不留 ARIA 残影。
    expect(el.shadowRoot!.querySelector('sebas-new-session-dialog')).toBeNull()
    el.remove()
  })

  it('keeps the dialog open with the typed rejection when creation fails', async () => {
    mockOf(apiMock.createSession).mockRejectedValue(new Error('HTTP 409: 已有会话'))
    mockOf(apiMock.nodes).mockResolvedValue({
      nodes: [{ id: 'local', status: 'online', local: true }],
      remote_available: true,
    })
    const el = await mount()
    ;(
      el.shadowRoot!.querySelector('button[aria-label="New session in beta"]') as HTMLButtonElement
    ).click()
    await el.updateComplete
    const dialog = await dialogOf(el)
    dialog.dispatchEvent(
      new CustomEvent('dialog-confirm', {
        detail: { agent: 'claude', model: null, mode: null },
        bubbles: true,
        composed: true,
      }),
    )
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete

    // 失败不假装成功：对话框仍在、错误就地呈现。
    expect((await dialogOf(el)).open).toBe(true)
    expect(dialog.shadowRoot!.querySelector('[data-testid="dialog-error"]')?.textContent).toContain(
      '已有会话',
    )
    el.remove()
  })

  it('a confirm with no bound project is not silent: inline error, no request (3.4)', async () => {
    // polish-workbench-walkthrough-ux 3.4：分派条件不成立（目标项目不可用）
    // 时绝不假装创建成功——就地 inline 错误、不派发任何创建请求（守卫层
    // 断言；后端拒绝时对话框保持打开的就地呈现由上一个用例覆盖）。
    mockOf(apiMock.createSession).mockClear()
    mockOf(apiMock.nodes).mockResolvedValue({
      nodes: [{ id: 'local', status: 'online', local: true }],
      remote_available: true,
    })
    const el = await mount()
    ;(el as unknown as { newSessionTarget: null }).newSessionTarget = null
    await el.updateComplete
    await (el as unknown as { confirmNewSession: (e: CustomEvent) => Promise<void> }).confirmNewSession(
      new CustomEvent('dialog-confirm', {
        detail: { agent: 'claude', model: null, mode: null },
      }),
    )
    await el.updateComplete
    // 没有静默：无请求、inline 错误状态写入（对话框重开即呈现）。
    expect(mockOf(apiMock.createSession)).not.toHaveBeenCalled()
    expect((el as unknown as { newSessionError: string | null }).newSessionError).toContain(
      '无法创建会话',
    )
    el.remove()
  })

  it('cancel closes the dialog and creates nothing', async () => {
    mockOf(apiMock.nodes).mockResolvedValue({
      nodes: [{ id: 'local', status: 'online', local: true }],
      remote_available: true,
    })
    const el = await mount()
    ;(
      el.shadowRoot!.querySelector('button[aria-label="New session in alpha"]') as HTMLButtonElement
    ).click()
    await el.updateComplete
    const dialog = await dialogOf(el)
    dialog.dispatchEvent(new CustomEvent('dialog-cancel', { bubbles: true, composed: true }))
    await el.updateComplete
    // （5.3）关闭即整棵移出 DOM。
    expect(el.shadowRoot!.querySelector('sebas-new-session-dialog')).toBeNull()
    expect(mockOf(apiMock.createSession)).not.toHaveBeenCalled()
    el.remove()
  })
})

// ─── workbench-rail-polish：rail 高亮分层 + 创建焦点链 ────────────────────────

describe('rail highlight layering (workbench-rail-polish 2.1)', () => {
  it('project active goes neutral while the focused session keeps the accent', async () => {
    const el = await mount()
    // happy-dom 下 Lit 走 adoptedStyleSheets（shadow 里没有 <style> 元素）：
    // 直接断言组件静态样式表的 cssText（与挂载环境无关）。
    const styles = SebasProjectRail.styles
    const styleText = (Array.isArray(styles) ? styles : [styles])
      .map((s) => (s as unknown as { cssText: string }).cssText)
      .join('\n')
    // 项目行选中 = 中性提亮（surface 底 + 亮文字），绝不沾 accent。
    const activeRule = styleText.match(/\.row\.active\s*\{[^}]*\}/)?.[0] ?? ''
    expect(activeRule).toContain('var(--sebas-surface')
    expect(activeRule).toContain('var(--sebas-text-bright)')
    expect(activeRule).not.toContain('accent')
    // accent 覆盖连计数徽标一并撤干净：项目行上所有规则整体中性。
    for (const line of styleText.split('\n')) {
      if (line.includes('.row.active')) expect(line).not.toContain('accent')
    }
    // 会话行「当前」标记保留 accent 底（驱动语义 activePath /
    // active_session_key 均不变，这里只看皮）。
    const currentRule = styleText.match(/li\.session-item\.current\s*\{[^}]*\}/)?.[0] ?? ''
    expect(currentRule).toContain('var(--sebas-accent-soft)')
    expect(currentRule).toContain('var(--sebas-accent)')
    // 两态样式组合互不相同（spec：clearly different visual treatments）。
    expect(activeRule).not.toBe(currentRule)
    el.remove()
  })
})

// 渲染 DOM 面：样式表断言之上的三条 scenario 行级可测面——同屏点亮、
// current 标记不随项目选中态翻转、active 类落在且只落在选中项目行上。
describe('rail highlight layering — rendered rows (workbench-rail-polish scenarios)', () => {
  /** 挂载时把焦点指针钉在 alpha 的首个会话上（/api/sessions 的 active_session_key）。 */
  async function mountWithFocusedSession(): Promise<SebasProjectRail> {
    mockOf(apiMock.sessions).mockResolvedValue({
      ...sessionList(sessionRows),
      active_session_key: sessionRows[0]!.encoded_key,
    })
    return mount()
  }

  it('highlights the selected project row and its focused session row at once, with distinct treatments', async () => {
    const el = await mountWithFocusedSession()
    el.activePath = '/home/me/alpha'
    await el.updateComplete
    // （4.2）聚焦会话所在项目缺省展开——会话行天然同屏渲染，无需点击。

    const activeRow = el.shadowRoot!.querySelector('.row.active')
    expect(activeRow).toBeTruthy()
    expect(activeRow!.getAttribute('aria-current')).toBe('true')
    const currentItem = activeRow!.closest('li')!.querySelector('li.session-item.current')
    expect(currentItem).toBeTruthy()
    expect(currentItem!.getAttribute('aria-current')).toBe('true')
    // 同屏两态 = 两个不同元素、两套类名；皮的具体差异由样式表断言看管。
    expect(currentItem).not.toBe(activeRow)
    expect(activeRow!.classList.contains('current')).toBe(false)
    expect(currentItem!.classList.contains('active')).toBe(false)
    el.remove()
  })

  it('keeps the focused-session marker regardless of which project is selected', async () => {
    const el = await mountWithFocusedSession()
    // （4.2）alpha 组随聚焦缺省展开——current 标记无需手动展开即可见。
    await el.updateComplete
    expect(el.shadowRoot!.querySelector('li.session-item.current')).toBeTruthy()

    // 项目选中态切到另一个项目：聚焦会话行的 current 标记原样保留。
    el.activePath = '/home/me/beta'
    await el.updateComplete
    const activeRows = [...el.shadowRoot!.querySelectorAll('.row.active')]
    expect(activeRows).toHaveLength(1)
    expect(activeRows[0]!.textContent).toContain('beta')
    const current = el.shadowRoot!.querySelector('li.session-item.current')
    expect(current).toBeTruthy()
    expect(current!.getAttribute('aria-current')).toBe('true')
    el.remove()
  })

  it('emphasizes exactly the selected project row with the active class', async () => {
    const el = await mount()
    expect(el.shadowRoot!.querySelector('.row.active')).toBeNull()
    el.activePath = '/home/me/beta'
    await el.updateComplete
    const rows = [...el.shadowRoot!.querySelectorAll('.row')]
    const active = rows.filter((r) => r.classList.contains('active'))
    expect(active).toHaveLength(1)
    expect(active[0]!.textContent).toContain('beta')
    expect(active[0]!.getAttribute('aria-current')).toBe('true')
    // 中性强调在类名层面即与会话行的 current 标记互斥（无 accent 的皮由
    // 样式表断言看管）。
    expect(active[0]!.classList.contains('current')).toBe(false)
    el.remove()
  })
})

describe('creation focus chain (workbench-rail-polish 3.1/3.2)', () => {
  beforeEach(() => {
    mockOf(apiMock.createSession).mockReset()
  })

  /** 打开 alpha 的创建对话框并确认（走 confirmNewSession 的成功路径）。 */
  async function confirmCreation(el: SebasProjectRail): Promise<void> {
    const plus = el.shadowRoot!.querySelector<HTMLButtonElement>(
      'button[aria-label="New session in alpha"]',
    )!
    plus.click()
    await el.updateComplete
    ;(el.shadowRoot!.querySelector('sebas-new-session-dialog') as HTMLElement).dispatchEvent(
      new CustomEvent('dialog-confirm', {
        detail: { agent: 'codex', model: null, mode: null },
        bubbles: true,
        composed: true,
      }),
    )
    // 两拍：createSession 落定 + setTimeout(0) 的对焦请求派发落定。
    await new Promise((r) => setTimeout(r, 0))
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
  }

  it('creating from an expanded project keeps the group expanded (no toggle-collapse)', async () => {
    mockOf(apiMock.createSession).mockResolvedValue({ key: 'oc_new' })
    const el = await mount()
    ;(el.shadowRoot!.querySelectorAll('.row')[0] as HTMLElement).click()
    await el.updateComplete
    expect(el.shadowRoot!.querySelector('li.session-item')).toBeTruthy()

    await confirmCreation(el)

    // 旧实现（onSelect toggle）在这里会把已展开的组折叠掉——组必须仍展开。
    expect(el.shadowRoot!.querySelector('li.session-item')).toBeTruthy()
    expect(
      (el as unknown as { expanded: Record<string, boolean> }).expanded['/home/me/alpha'],
    ).toBe(true)
    el.remove()
  })

  it('creating from a collapsed project expands it (placeholder row visible)', async () => {
    mockOf(apiMock.createSession).mockResolvedValue({ key: 'oc_new' })
    const el = await mount()
    expect(el.shadowRoot!.querySelector('li.session-item')).toBeNull()

    await confirmCreation(el)

    expect(
      (el as unknown as { expanded: Record<string, boolean> }).expanded['/home/me/alpha'],
    ).toBe(true)
    expect(el.shadowRoot!.querySelector('li.session-item')).toBeTruthy()
    el.remove()
  })

  it('marks the new placeholder row current and visible under its project (server set_focus backfill)', async () => {
    mockOf(apiMock.createSession).mockResolvedValue({ key: 'oc_new%00' })
    // 创建成功后的 refresh() 回填（服务端 create_session 已 set_focus）：
    // /api/sessions 带上新占位行，焦点指针指向它。
    const placeholderRow = row({
      encoded_key: 'oc_new%00',
      project_id: 'proj-alpha',
      status: 'starting',
      status_slug: 'starting',
    })
    mockOf(apiMock.sessions).mockResolvedValue({
      ...sessionList([
        ...sessionRows.filter((r) => r.project_id === 'proj-alpha'),
        placeholderRow,
      ]),
      active_session_key: 'oc_new%00',
    })
    const el = await mount()
    await confirmCreation(el)

    const placeholder = el.shadowRoot!.querySelector('li.session-item.current')
    expect(placeholder).toBeTruthy()
    expect(placeholder!.getAttribute('aria-current')).toBe('true')
    expect(placeholder!.textContent).toContain(placeholderRow.session_id_short!)
    // 占位行就挂在被创建项目（alpha）的组下——保持展开，新行立即可见。
    const alphaLi = (el.shadowRoot!.querySelectorAll('.row')[0] as HTMLElement).closest('li')!
    expect(alphaLi.querySelector('li.session-item.current')).toBe(placeholder)
    el.remove()
  })

  it('does not dispatch rail-select on the creation path', async () => {
    mockOf(apiMock.createSession).mockResolvedValue({ key: 'oc_new' })
    const el = await mount()
    const selected = vi.fn()
    el.addEventListener('rail-select', selected)
    await confirmCreation(el)
    // 创建不再冒充项目选择：主区切换语义不掺进创建路径。
    expect(selected).not.toHaveBeenCalled()
    el.remove()
  })

  it('requests composer focus exactly once after a successful creation', async () => {
    mockOf(apiMock.createSession).mockResolvedValue({ key: 'oc_new' })
    const el = await mount()
    const requested = vi.fn()
    window.addEventListener('sebas:composer-focus', requested)
    await confirmCreation(el)
    expect(requested).toHaveBeenCalledTimes(1)
    window.removeEventListener('sebas:composer-focus', requested)
    el.remove()
  })

  it('a failed creation neither expands nor requests focus', async () => {
    mockOf(apiMock.createSession).mockRejectedValue(new Error('HTTP 409: 已有会话'))
    const el = await mount()
    const requested = vi.fn()
    window.addEventListener('sebas:composer-focus', requested)
    await confirmCreation(el)
    expect(requested).not.toHaveBeenCalled()
    expect(
      (el as unknown as { expanded: Record<string, boolean> }).expanded['/home/me/alpha'],
    ).toBeFalsy()
    // 失败留在对话框内就地呈现（既有语义不变）。
    const dialog = el.shadowRoot!.querySelector(
      'sebas-new-session-dialog',
    ) as unknown as HTMLElement & { open: boolean }
    expect(dialog.open).toBe(true)
    window.removeEventListener('sebas:composer-focus', requested)
    el.remove()
  })
})

describe('rail focus event dispatch (fix-webui-qa-defects 4.1)', () => {
  it('a successful in-place switch dispatches the focus event with the active key', async () => {
    mockOf(apiMock.switchSession).mockResolvedValue({
      status: 'switched',
      redirect: '/sessions/oc_1%00',
      active_session_key: 'oc_1%00',
    })
    window.history.replaceState({}, '', '/')
    const events: CustomEvent[] = []
    const listener = (e: Event) => events.push(e as CustomEvent)
    window.addEventListener('sebas:rail-focus', listener)
    try {
      const el = await mount()
      ;(el.shadowRoot!.querySelectorAll('.row')[0] as HTMLElement).click()
      await el.updateComplete
      const first = el.shadowRoot!.querySelector('li.session-item') as HTMLElement
      first.click()
      await new Promise((r) => setTimeout(r, 0))
      await el.updateComplete
      const focus = events.find((e) => (e.detail as { key?: string })?.key === 'oc_1%00')
      expect(focus, 'the focus event must carry the switch response key').toBeTruthy()
      el.remove()
    } finally {
      window.removeEventListener('sebas:rail-focus', listener)
    }
  })

  it('a failed switch dispatches no focus event', async () => {
    mockOf(apiMock.switchSession).mockRejectedValue(new Error('404'))
    window.history.replaceState({}, '', '/')
    const events: CustomEvent[] = []
    const listener = (e: Event) => events.push(e as CustomEvent)
    window.addEventListener('sebas:rail-focus', listener)
    try {
      const el = await mount()
      ;(el.shadowRoot!.querySelectorAll('.row')[0] as HTMLElement).click()
      await el.updateComplete
      const first = el.shadowRoot!.querySelector('li.session-item') as HTMLElement
      first.click()
      await new Promise((r) => setTimeout(r, 0))
      await el.updateComplete
      expect(events).toHaveLength(0)
      el.remove()
    } finally {
      window.removeEventListener('sebas:rail-focus', listener)
    }
  })
})

// ── fix-webui-approval-restore-and-session-identity ──────────────────────

describe('rail expansion persistence (4.2)', () => {
  beforeEach(() => {
    window.localStorage.clear()
  })

  it('parse/serialize round-trip and tolerate junk (4.2)', () => {
    expect(parseRailExpanded(null)).toEqual({})
    expect(parseRailExpanded('not json')).toEqual({})
    expect(parseRailExpanded('{"object":"not array"}')).toEqual({})
    expect(parseRailExpanded(JSON.stringify(['/a', '', 42, '/b']))).toEqual({ '/a': true, '/b': true })
    expect(serializeRailExpanded(parseRailExpanded(JSON.stringify(['/a', '/b'])))).toBe('["/a","/b"]')
  })

  it('recorded expansion wins over the focused default (4.2)', () => {
    expect(railExpandedDefault({ '/a': false }, '/a', true)).toBe(false)
    expect(railExpandedDefault({ '/a': true }, '/a', false)).toBe(true)
  })

  it('a project without a record default-expands when its session is focused (4.2)', () => {
    expect(railExpandedDefault({}, '/a', true)).toBe(true)
    expect(railExpandedDefault({}, '/a', false)).toBe(false)
  })

  it('toggling persists the expansion and a fresh mount restores it (4.2)', async () => {
    // Toggle the first project row (alpha) — no focused session (default mock).
    const el = await mount()
    const row = el.shadowRoot!.querySelectorAll<HTMLElement>('.row')[0]!
    expect(row.getAttribute('aria-expanded')).toBe('false')
    row.click()
    await el.updateComplete
    expect(row.getAttribute('aria-expanded')).toBe('true')
    expect(window.localStorage.getItem(RAIL_EXPANDED_KEY)).toBe(JSON.stringify(['/home/me/alpha']))
    el.remove()

    // A fresh mount (page reload analogue) restores the recorded expansion.
    const el2 = await mount()
    const row2 = el2.shadowRoot!.querySelectorAll<HTMLElement>('.row')[0]!
    expect(row2.getAttribute('aria-expanded')).toBe('true')
    // The un-toggled project stays collapsed (recorded-only state).
    expect(
      el2.shadowRoot!.querySelectorAll<HTMLElement>('.row')[1]!.getAttribute('aria-expanded'),
    ).toBe('false')
    el2.remove()
  })

  it('a focused session expands its project by default when no record exists (4.2)', async () => {
    // Focus a session that belongs to proj-beta; no persisted record.
    const focused = row({ project_id: 'proj-beta', status: 'done', status_slug: 'done' })
    mockOf(apiMock.sessions).mockResolvedValue({
      ...sessionList([focused]),
      active_session_key: focused.encoded_key,
    })
    const el = await mount()
    const rows = el.shadowRoot!.querySelectorAll<HTMLElement>('.row')
    // beta（index 1）缺省展开；alpha（无聚焦、无记录）保持收起。
    expect(rows[0]!.getAttribute('aria-expanded')).toBe('false')
    expect(rows[1]!.getAttribute('aria-expanded')).toBe('true')
    el.remove()
  })

  it('expansion does not flip on its own across refreshes (4.2)', async () => {
    const el = await mount()
    const row = el.shadowRoot!.querySelectorAll<HTMLElement>('.row')[0]!
    row.click()
    await el.updateComplete
    expect(row.getAttribute('aria-expanded')).toBe('true')

    // A background refresh (ws refetch / node poll) must not touch expansion.
    await el.refresh()
    await el.updateComplete
    expect(
      el.shadowRoot!.querySelectorAll<HTMLElement>('.row')[0]!.getAttribute('aria-expanded'),
    ).toBe('true')
    el.remove()
  })
})

describe('session naming by operator label (5.1)', () => {
  it('label takes precedence over the first-prompt preview; clearing falls back (5.1)', async () => {
    const named = row({
      project_id: 'proj-alpha',
      prompt_preview: 'first prompt',
      label: '重构计划',
    })
    expect(fullSessionLabel(named)).toBe('重构计划')
    // 清空（null/缺省）→ 首条 prompt 预览（旧行为完全一致）。
    expect(fullSessionLabel({ ...named, label: null })).toBe('first prompt')
    expect(fullSessionLabel({ ...named, label: undefined })).toBe('first prompt')
  })

  it('the rename action sets the label via the seam and refreshes (5.1)', async () => {
    mockOf(apiMock.setSessionLabel).mockResolvedValue({ status: 'ok' })
    const el = await mount()
    const target = (el as any).sessions[0] as SessionRow
    ;(el as any).openRenameDialog(new Event('click'), target)
    await (el as any).updateComplete
    const dialog = el.shadowRoot!.querySelector('wa-dialog[label="重命名会话"]')
    expect(dialog).toBeTruthy()
    expect((el as any).renameValue).toBe('')

    ;(el as any).renameValue = '  我的项目会话  '
    // 保存从 DOM（wa-input）取值（round5 2.1）——先让 Lit 把状态渲染进
    // DOM（真实流程：输入 → 渲染 → 点击保存）。
    await (el as any).updateComplete
    await (el as any).confirmRename()
    expect(mockOf(apiMock.setSessionLabel)).toHaveBeenCalledWith(target.encoded_key, '我的项目会话')
    expect((el as any).renameTarget).toBeNull()
    el.remove()
  })

  it('an empty rename clears the label (falls back to the prompt preview) (5.1)', async () => {
    mockOf(apiMock.setSessionLabel).mockResolvedValue({ status: 'ok' })
    const el = await mount()
    const named = row({ project_id: 'proj-alpha', prompt_preview: 'prompt', label: '旧名' })
    ;(el as any).openRenameDialog(new Event('click'), named)
    ;(el as any).renameValue = '   '
    await (el as any).confirmRename()
    expect(mockOf(apiMock.setSessionLabel)).toHaveBeenCalledWith(named.encoded_key, null)
    el.remove()
  })

  // （fix-webui-qa-defects-round5 2.1，design 决策 3）组件级回归：渲染对话框
  // → 填值 → 保存 → 请求体携带该值且成功后关闭。宿主 value 与内部原生
  // input 各自独立（QA D3a：宿主属性不同步让输入值在保存链路丢失，保存成
  // 静默清空）——保存必须以内部原生 input 为锚。
  it('the save reads the value from the internal native input and closes on success (round5 2.1)', async () => {
    mockOf(apiMock.setSessionLabel).mockResolvedValue({ status: 'ok' })
    const el = await mount()
    const target = (el as any).sessions[0] as SessionRow
    ;(el as any).openRenameDialog(new Event('click'), target)
    await (el as any).updateComplete
    const host = el.shadowRoot!.querySelector('wa-input[data-testid="rename-input"]') as
      | (HTMLElement & { value?: string; shadowRoot?: ShadowRoot | null })
    expect(host).toBeTruthy()
    // 模拟真实 WA 内部结构：原生 input 携带输入值，宿主属性保持陈旧空值
    // ——正是缺陷场景。测试环境 wa-input 未升级，手动附 shadowRoot。
    const shadow = host.shadowRoot ?? host.attachShadow({ mode: 'open' })
    const native = document.createElement('input')
    native.value = '  来自原生输入的名字  '
    shadow.appendChild(native)
    host.value = '';
    (el as any).renameValue = ''
    await (el as any).updateComplete
    await (el as any).confirmRename()
    expect(mockOf(apiMock.setSessionLabel)).toHaveBeenCalledWith(
      target.encoded_key,
      '来自原生输入的名字',
    )
    expect((el as any).renameTarget).toBeNull()
    el.remove()
  })

  it('a failed save keeps the dialog open with the inline error (round5 2.1)', async () => {
    mockOf(apiMock.setSessionLabel).mockRejectedValue(new Error('会话不存在'))
    const el = await mount()
    const target = (el as any).sessions[0] as SessionRow
    ;(el as any).openRenameDialog(new Event('click'), target)
    ;(el as any).renameValue = '新名'
    await (el as any).confirmRename()
    expect((el as any).renameTarget).toBe(target)
    const err = el.shadowRoot!.querySelector('[data-testid="rename-error"]')
    expect(err?.textContent).toContain('会话不存在')
    el.remove()
  })
})

// ── fix-webui-qa-defects-round5 3.2/6.3：label 变更实时刷新（label 比对收窄）┘
describe('label write liveness via session.updated (round5 3.2/6.3)', () => {
  it('unrelated frames skip the re-projection; a label-change frame triggers exactly one debounced refetch', async () => {
    vi.useFakeTimers()
    try {
      const el = document.createElement('sebas-project-rail') as SebasProjectRail
      document.body.appendChild(el)
      // connectedCallback 的 refresh 是宏任务串——fake timers 下手动推进。
      await vi.advanceTimersByTimeAsync(0)
      await el.updateComplete
      await vi.advanceTimersByTimeAsync(0)
      await el.updateComplete
      mockOf(apiMock.sessions).mockClear()
      expect(mockOf(apiMock.sessions)).toHaveBeenCalledTimes(0)

      // 无关帧（相位/队列翻转；帧 label 与行已知 label 一致——同为空）：
      // 只就地补丁，绝不调度重取（6.3 收窄——活跃 turn 的帧连发不再放大
      // 请求量），相位补丁本身照常落地。
      wsMocks.emit({
        type: 'session.updated',
        session_id: sessionRows[0]!.encoded_key,
        status_slug: 'done',
        turn_engaged: false,
        msg_count: 3,
        pending: [],
        label: null,
      })
      await el.updateComplete
      await vi.advanceTimersByTimeAsync(LABEL_REFRESH_DEBOUNCE_MS + 1)
      expect(mockOf(apiMock.sessions)).toHaveBeenCalledTimes(0)
      expect((el as any).sessions[0].status_slug).toBe('done')
      expect((el as any).sessions[0].msg_count).toBe(3)

      // label 变化帧（API 写 label 成功后的 Updated，载荷携带新 label）：
      // 进入防抖；窗口内的第二帧（另一会话的 label 变化）合并为一次重取。
      const relabeled = sessionRows.map((r, i) => (i === 0 ? { ...r, label: 'API 命名' } : r))
      mockOf(apiMock.sessions).mockResolvedValue(sessionList(relabeled))
      wsMocks.emit({
        type: 'session.updated',
        session_id: sessionRows[0]!.encoded_key,
        status_slug: 'working',
        turn_engaged: true,
        msg_count: 4,
        pending: [],
        label: 'API 命名',
      })
      await vi.advanceTimersByTimeAsync(LABEL_REFRESH_DEBOUNCE_MS - 1)
      const relabeled2 = relabeled.map((r, i) => (i === 1 ? { ...r, label: '第二行' } : r))
      mockOf(apiMock.sessions).mockResolvedValue(sessionList(relabeled2))
      wsMocks.emit({
        type: 'session.updated',
        session_id: sessionRows[1]!.encoded_key,
        status_slug: 'done',
        turn_engaged: false,
        msg_count: 5,
        pending: [],
        label: '第二行',
      })
      await vi.advanceTimersByTimeAsync(LABEL_REFRESH_DEBOUNCE_MS + 1)
      await el.updateComplete
      await vi.advanceTimersByTimeAsync(0)
      await el.updateComplete
      expect(mockOf(apiMock.sessions)).toHaveBeenCalledTimes(1)
      expect((el as any).sessions[0].label).toBe('API 命名')
      expect((el as any).sessions[1].label).toBe('第二行')
      // 展开项目组后行名以 label 重渲染（无刷新）。
      ;(el.shadowRoot!.querySelectorAll('.row')[0] as HTMLElement).click()
      await el.updateComplete
      expect(el.shadowRoot!.textContent).toContain('API 命名')
      el.remove()
    } finally {
      vi.useRealTimers()
    }
  })

  it('label-clearing frames (label → null) count as a naming change and refetch', async () => {
    vi.useFakeTimers()
    try {
      const labeled = sessionRows.map((r, i) => (i === 0 ? { ...r, label: '旧名' } : r))
      mockOf(apiMock.sessions).mockResolvedValue(sessionList(labeled))
      const el = document.createElement('sebas-project-rail') as SebasProjectRail
      document.body.appendChild(el)
      await vi.advanceTimersByTimeAsync(0)
      await el.updateComplete
      await vi.advanceTimersByTimeAsync(0)
      await el.updateComplete
      mockOf(apiMock.sessions).mockClear()

      // 清空 label 的帧：帧 label null ≠ 行已知 '旧名' → 真实命名变化，
      // 调度重取（行名回退预览/短 id 由重取收敛）。
      wsMocks.emit({
        type: 'session.updated',
        session_id: labeled[0]!.encoded_key,
        status_slug: 'done',
        turn_engaged: false,
        msg_count: 3,
        pending: [],
        label: null,
      })
      await vi.advanceTimersByTimeAsync(LABEL_REFRESH_DEBOUNCE_MS + 1)
      await el.updateComplete
      await vi.advanceTimersByTimeAsync(0)
      await el.updateComplete
      expect(mockOf(apiMock.sessions)).toHaveBeenCalledTimes(1)
      el.remove()
    } finally {
      vi.useRealTimers()
    }
  })

  it('a frame whose label equals the row\u2019s known non-null label skips the refetch', async () => {
    // 6.3 比对的「同为已设值」半边：上面的用例钉了同为空（null==null）跳过
    // 与清空触发；这里钉 label 已设且帧携带同一值（相位照变）也不调度——
    // 比较的是 label 本身（null 归一），不是退化渲染名。
    vi.useFakeTimers()
    try {
      const labeled = sessionRows.map((r, i) => (i === 0 ? { ...r, label: '同名' } : r))
      mockOf(apiMock.sessions).mockResolvedValue(sessionList(labeled))
      const el = document.createElement('sebas-project-rail') as SebasProjectRail
      document.body.appendChild(el)
      await vi.advanceTimersByTimeAsync(0)
      await el.updateComplete
      await vi.advanceTimersByTimeAsync(0)
      await el.updateComplete
      mockOf(apiMock.sessions).mockClear()

      wsMocks.emit({
        type: 'session.updated',
        session_id: labeled[0]!.encoded_key,
        status_slug: 'working',
        turn_engaged: true,
        msg_count: 4,
        pending: [],
        label: '同名',
      })
      await el.updateComplete
      await vi.advanceTimersByTimeAsync(LABEL_REFRESH_DEBOUNCE_MS + 1)
      expect(mockOf(apiMock.sessions)).toHaveBeenCalledTimes(0)
      // 相位补丁照常落地（跳过的是重取调度，不是帧本身）。
      expect((el as any).sessions[0].status_slug).toBe('working')
      expect((el as any).sessions[0].msg_count).toBe(4)
      el.remove()
    } finally {
      vi.useRealTimers()
    }
  })

  it('session.created and updated frames for rows the rail does not know keep the old behavior', async () => {
    vi.useFakeTimers()
    try {
      const el = document.createElement('sebas-project-rail') as SebasProjectRail
      document.body.appendChild(el)
      await vi.advanceTimersByTimeAsync(0)
      await el.updateComplete
      await vi.advanceTimersByTimeAsync(0)
      await el.updateComplete
      mockOf(apiMock.sessions).mockClear()

      // created：rail 还没有该行——保持现行为（要建行，调度重取）。
      wsMocks.emit({
        type: 'session.created',
        session_id: 'oc_new%00',
        status_slug: 'starting',
        turn_engaged: true,
        msg_count: 0,
        pending: [],
        label: null,
      })
      // updated 但 rail 中不存在该会话：同样调度（要建行）。
      wsMocks.emit({
        type: 'session.updated',
        session_id: 'oc_unknown%00',
        status_slug: 'working',
        turn_engaged: true,
        msg_count: 1,
        pending: [],
        label: null,
      })
      await vi.advanceTimersByTimeAsync(LABEL_REFRESH_DEBOUNCE_MS + 1)
      await el.updateComplete
      await vi.advanceTimersByTimeAsync(0)
      await el.updateComplete
      expect(mockOf(apiMock.sessions)).toHaveBeenCalledTimes(1)
      el.remove()
    } finally {
      vi.useRealTimers()
    }
  })

  it('the closed row menu keeps its items out of the a11y tree (round5 4.1)', async () => {
    const { readFileSync } = await import('node:fs')
    const { join, dirname } = await import('node:path')
    const here = dirname(new URL(import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1'))
    const src = readFileSync(join(here, 'project-rail.ts'), 'utf8')
    // 关闭态（wa-dropdown 无 open 反射属性）菜单项 display:none——a11y 树
    // 不再暴露「重命名/归档/移除项目」；打开翻转与 popup 激活同帧。
    expect(src).toMatch(/wa-dropdown:not\(\[open\]\)\s+wa-dropdown-item\s*\{[^}]*display:\s*none/)
  })
})

describe('add-project scope hint (5.2)', () => {
  it('out-of-workspace errors (both wordings) name the boundary', () => {
    expect(addPathScopeHintFrom('路径超出允许范围: 不在 workspace root 内')).toContain(
      'workspace root 之外',
    )
    // browse-dirs 对根外绝对路径的实际文案（此前漏配——静默禁用的根因）。
    expect(addPathScopeHintFrom('路径超出根目录范围')).toContain('workspace root 之外')
  })

  it('nonexistent paths get their own reason and not-a-directory too (5.2)', () => {
    expect(addPathScopeHintFrom('路径不存在或无法访问: /nope')).toContain('不存在')
    expect(addPathScopeHintFrom('不是目录')).toContain('不是目录')
    // 未知失败不拦截（留给注册接口点名）。
    expect(addPathScopeHintFrom('boom')).toBeNull()
  })

  it('submit stays disabled while a scope hint is present (5.2)', async () => {
    const el = await mount()
    ;(el as any).addDialogOpen = true
    ;(el as any).addPath = '/home/me/alpha'
    ;(el as any).addPathScopeHint = '路径在 workspace root 之外——只能注册工作区内的目录'
    await (el as any).updateComplete
    const submit = [...el.shadowRoot!.querySelectorAll('wa-button')].find((b) =>
      b.textContent!.includes('Add project'),
    ) as HTMLElement & { hasAttribute: (n: string) => boolean }
    expect(submit.hasAttribute('disabled')).toBe(true)
    // 原因在输入框旁可见（data-testid 钩子）。
    expect(
      el.shadowRoot!.querySelector('[data-testid="add-project-scope-hint"]'),
    ).toBeTruthy()
    el.remove()
  })
})

// ── fix-webui-qa-defects-round3：路径展示归一（4.4）+ 创建立锚（3.1）──────

describe('display path normalization (round3 4.4)', () => {
  it('normalizes backslashes to forward slashes for display', () => {
    expect(normalizeDisplayPath('D:\\work\\repos\\sebas')).toBe('D:/work/repos/sebas')
    expect(normalizeDisplayPath('/home/me/alpha')).toBe('/home/me/alpha')
    expect(normalizeDisplayPath('already /mixed\\path')).toBe('already /mixed/path')
    expect(normalizeDisplayPath('')).toBe('')
  })

  it('folder-picker fills the add-project input with the normalized path', async () => {
    mockOf(apiMock.fsBrowseDirs).mockResolvedValue({ entries: [], root: 'D:/work' })
    const el = await mount()
    ;(el as any).addDialogOpen = true
    await (el as any).updateComplete
    const picker = el.shadowRoot!.querySelector('sebas-folder-picker')
    expect(picker).toBeTruthy()
    picker!.dispatchEvent(
      new CustomEvent('folder-selected', { detail: { path: 'D:\\work\\repos\\sebas' } }),
    )
    await (el as any).updateComplete
    // 填充值即展示值：反斜杠普通形归一为正斜杠，与服务端错误消息同词。
    expect((el as any).addPath).toBe('D:/work/repos/sebas')
    el.remove()
  })

  it('the already-registered server error is shown with normalized separators', async () => {
    mockOf(apiMock.projects.add).mockRejectedValue(
      new Error('项目已注册：D:\\work\\repos\\sebas'),
    )
    mockOf(apiMock.fsBrowseDirs).mockResolvedValue({ entries: [], root: 'D:/work' })
    const el = await mount()
    ;(el as any).addDialogOpen = true
    ;(el as any).addPath = 'D:/work/repos/sebas'
    await (el as any).updateComplete
    await (el as any).submitAddProject()
    await (el as any).updateComplete
    expect((el as any).addError).toBe('项目已注册：D:/work/repos/sebas')
    el.remove()
  })
})

describe('creation establishes the read anchor (round3 3.1)', () => {
  it('creating a session writes the local anchor at zero for the new key', async () => {
    // 创建路径没有 rail 点击（服务端 set_focus 直达焦点），锚原本永远缺位，
    // 无锚 = fully read——新会话之后的非聚焦新回复推不出未读徽章（QA 缺陷
    // 3 根因）。创建成功即在本浏览器立锚（0 轮占位）。
    mockOf(apiMock.createSession).mockResolvedValue({ key: 'oc_new' })
    mockOf(apiMock.nodes).mockResolvedValue({
      nodes: [{ id: 'local', status: 'online', local: true }],
      remote_available: true,
    })
    const el = await mount()
    ;(
      el.shadowRoot!.querySelector('button[aria-label="New session in alpha"]') as HTMLButtonElement
    ).click()
    await el.updateComplete
    ;(el as any).confirmNewSession(
      new CustomEvent('dialog-confirm', {
        detail: { agent: 'codex', model: null, mode: 'ask' },
        bubbles: true,
        composed: true,
      }),
    )
    await new Promise((r) => setTimeout(r, 0))
    expect(JSON.parse(localStorage.getItem('sebas:seen:oc_new')!)).toEqual({ anchor_count: 0 })
    el.remove()
  })
})

// ── fix-webui-qa-defects-round3 1.2/1.3：真实确认按钮穿过双层 guard 的集成 ──

describe('in-flight creation guard through the real confirm button (round3 1.2/1.3)', () => {
  beforeEach(() => {
    // 本文件各 describe 自管 mock 生命周期：清掉先前用例留下的调用记录与
    // 实现（含 Once 队列）——本组断言「恰好一次 POST」，计数必须从零起算。
    mockOf(apiMock.createSession).mockReset()
  })

  async function openReadyDialog(): Promise<{
    el: SebasProjectRail
    dialog: HTMLElement & { busy: boolean; updateComplete: Promise<boolean> }
    confirmButton: HTMLElement
  }> {
    // agent 目录就位：预选首个可达 agent → 确认钮可激活（目录不可得时确认
    // 被 agent 门禁拦下，走不进本用例要压的 in-flight 路径）。
    mockOf(apiMock.agents).mockResolvedValue({
      agents: [{ id: 'claude', display: 'Claude Code', reachable: true }],
    })
    mockOf(apiMock.providers).mockResolvedValue({ providers: [] })
    mockOf(apiMock.providerDefaults).mockResolvedValue({
      default_provider: null,
      default_model: null,
    })
    mockOf(apiMock.nodes).mockResolvedValue({
      nodes: [{ id: 'local', status: 'online', local: true }],
      remote_available: true,
    })
    const el = await mount()
    ;(
      el.shadowRoot!.querySelector('button[aria-label="New session in alpha"]') as HTMLButtonElement
    ).click()
    await el.updateComplete
    const dialog = el.shadowRoot!.querySelector(
      'sebas-new-session-dialog',
    ) as unknown as HTMLElement & { busy: boolean; updateComplete: Promise<boolean> }
    // loadAgents/loadCatalog 是 fire-and-forget：多拍冲刷到 agent 预选落位。
    for (let i = 0; i < 4; i += 1) {
      await dialog.updateComplete
      await new Promise((r) => setTimeout(r, 0))
    }
    const confirmButton = dialog.shadowRoot!.querySelector(
      '[data-testid="dialog-confirm"]',
    ) as unknown as HTMLElement
    expect(confirmButton.hasAttribute('disabled')).toBe(false)
    return { el, dialog, confirmButton }
  }

  it('a double activation in flight issues exactly one POST; the dialog shows busy until it resolves', async () => {
    // spec 场景 2 的集成半边：dialog 的 busy 属性来自 rail 的
    // creatingSession，属性下传有一个渲染拍——同步连点两下时第二下仍会
    // 穿过 dialog guard，第二道 rail 守卫（creatingSession）必须兜住。
    let resolveCreate!: (v: { key: string }) => void
    mockOf(apiMock.createSession).mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveCreate = resolve
        }),
    )
    const { el, dialog, confirmButton } = await openReadyDialog()

    confirmButton.click()
    confirmButton.click() // 在途（属性尚未下传）的第二击
    await el.updateComplete
    await dialog.updateComplete

    expect(mockOf(apiMock.createSession)).toHaveBeenCalledTimes(1)
    // busy 下传：对话框呈忙态（rail creatingSession → dialog.busy）。
    // （round3 1.2 备选路径）确认控件是原生 button：忙态指示经 aria-busy。
    expect(dialog.busy).toBe(true)
    expect(confirmButton.hasAttribute('disabled')).toBe(true)
    expect(confirmButton.getAttribute('aria-busy')).toBe('true')
    expect(confirmButton.textContent).toContain('创建中')

    // 请求落定（成功）：恰好这一次创建，对话框收起。
    resolveCreate({ key: 'oc_new' })
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    expect(mockOf(apiMock.createSession)).toHaveBeenCalledTimes(1)
    expect(el.shadowRoot!.querySelector('sebas-new-session-dialog')).toBeNull()
    el.remove()
  })

  it('busy releases with the failed request so the operator can retry in place', async () => {
    // spec「busy state for the duration of the request」的收尾半边：忙态精确
    // 覆盖在途窗口——请求以失败告终时必须随之解除，按钮恢复可激活，重试
    // 就在对话框内完成。（忙态窗口要可观察，请求得先悬在在途再失败。）
    let rejectCreate!: (e: Error) => void
    mockOf(apiMock.createSession).mockImplementation(
      () =>
        new Promise((_, reject) => {
          rejectCreate = reject
        }),
    )
    const { el, dialog, confirmButton } = await openReadyDialog()

    confirmButton.click()
    await el.updateComplete
    await dialog.updateComplete
    expect(dialog.busy).toBe(true)

    // 请求失败落定：忙态解除、按钮复活、错误就地呈现（既有用例钉文案，
    // 这里钉忙态收尾）。
    rejectCreate(new Error('HTTP 409: 已有会话'))
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    await dialog.updateComplete
    expect(dialog.busy).toBe(false)
    expect(confirmButton.hasAttribute('disabled')).toBe(false)
    expect(confirmButton.hasAttribute('loading')).toBe(false)
    expect(dialog.shadowRoot!.querySelector('[data-testid="dialog-error"]')).toBeTruthy()

    // 原地重试成功 → 创建请求共两次、对话框收起。
    mockOf(apiMock.createSession).mockResolvedValue({ key: 'oc_new' })
    confirmButton.click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    expect(mockOf(apiMock.createSession)).toHaveBeenCalledTimes(2)
    expect(el.shadowRoot!.querySelector('sebas-new-session-dialog')).toBeNull()
    el.remove()
  })
})

describe('unread badge journey after creation (round3 3.1)', () => {
  it('a created session earns its badge from a later non-focused reply', async () => {
    // 缺陷 3 的旅程级链路：创建立 0 锚 → 该会话在非聚焦下收到新回复帧 →
    // rail 行冒出未读计数。创建半边与帧推导半边各有单测，这里钉它们的接缝。
    mockOf(apiMock.createSession).mockResolvedValue({ key: 'oc_new' })
    mockOf(apiMock.sessions).mockResolvedValue(
      sessionList([row({ encoded_key: 'oc_new', project_id: 'proj-alpha', msg_count: 0 })]),
    )
    mockOf(apiMock.nodes).mockResolvedValue({
      nodes: [{ id: 'local', status: 'online', local: true }],
      remote_available: true,
    })
    const el = await mount()
    ;(
      el.shadowRoot!.querySelector('button[aria-label="New session in alpha"]') as HTMLButtonElement
    ).click()
    await el.updateComplete
    ;(el as any).confirmNewSession(
      new CustomEvent('dialog-confirm', {
        detail: { agent: 'codex', model: null, mode: 'ask' },
        bubbles: true,
        composed: true,
      }),
    )
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete

    // 创建成功：0 段占位锚 + 创建流强制展开 alpha 组，新会话行可见、无徽标。
    const items = () => [...el.shadowRoot!.querySelectorAll('li.session-item')]
    expect(items().length).toBe(1)
    expect(items()[0]!.querySelector('[data-testid="session-unread"]')).toBeNull()

    // 非聚焦新回复（帧带 msg_count 1）：锚 0 → 徽标 1，无需列表刷新。
    wsMocks.emit({
      type: 'session.updated',
      session_id: 'oc_new',
      status_slug: 'working',
      turn_engaged: true,
      msg_count: 1,
      pending: [],
      label: null,
    })
    await el.updateComplete
    const badge = items()[0]!.querySelector('[data-testid="session-unread"]')
    expect(badge).toBeTruthy()
    expect(badge!.textContent).toBe('1')
    el.remove()
  })
})
