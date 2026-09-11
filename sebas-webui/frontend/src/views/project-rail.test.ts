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
import type { SebasProjectRail } from './project-rail.js'

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
  return {
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
    ...overrides,
  }
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

afterEach(() => {
  document.body.innerHTML = ''
  window.history.replaceState({}, '', '/')
  // 徽标读锚在 localStorage 里：用例间清空，防串扰。
  seenStore.clear()
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
    // 展开含 oc_2 的组（oc_1/oc_2 都是 proj-alpha 组 → 点第一行展开）。
    ;(el.shadowRoot!.querySelectorAll('.row')[0] as HTMLElement).click()
    await el.updateComplete
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

  it('session row has a single … menu holding Archive and Close (danger); no inline buttons', async () => {
    const el = await mount()
    ;(el.shadowRoot!.querySelectorAll('.row')[0] as HTMLElement).click()
    await el.updateComplete
    const item = el.shadowRoot!.querySelector('li.session-item')!
    // 行内不再有直删/归档按钮。
    expect(item.querySelector('button[title="Archive this session"]')).toBeNull()
    expect(item.querySelector('button[title="Close (delete) this session"]')).toBeNull()
    // 唯一的 … 菜单：归档 + 关闭（danger）。
    expect(item.querySelectorAll('wa-dropdown').length).toBe(1)
    const archive = item.querySelector('wa-dropdown-item[value="archive"]')
    const close = item.querySelector('wa-dropdown-item[value="close"]')
    expect(archive).toBeTruthy()
    expect(close).toBeTruthy()
    expect(close!.getAttribute('variant')).toBe('danger')
    el.remove()
  })

  it('the … menu triggers archive through the menu item (3.2)', async () => {
    mockOf(apiMock.archiveSession).mockResolvedValue({ status: 'archived' })
    const el = await mount()
    ;(el.shadowRoot!.querySelectorAll('.row')[0] as HTMLElement).click()
    await el.updateComplete
    const item = el.shadowRoot!.querySelector('li.session-item')!
    ;(item.querySelector('wa-dropdown-item[value="archive"]') as HTMLElement).click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    expect(apiMock.archiveSession).toHaveBeenCalledWith('oc_1%00')
    el.remove()
  })

  it('the … menu closes a dormant session immediately and asks first for a working one (3.2)', async () => {
    mockOf(apiMock.closeSession).mockResolvedValue({ status: 'closed', discarded_pending: 0 })
    const dormant = row({ project_id: 'proj-alpha', status: 'done', status_slug: 'done' })
    const working = row({ project_id: 'proj-alpha', status: 'working', status_slug: 'working' })
    mockOf(apiMock.sessions).mockResolvedValue(sessionList([dormant, working]))
    const el = await mount()
    ;(el.shadowRoot!.querySelectorAll('.row')[0] as HTMLElement).click()
    await el.updateComplete
    const items = [...el.shadowRoot!.querySelectorAll('li.session-item')]
    // inactive：直删。
    ;(items[0]!.querySelector('wa-dropdown-item[value="close"]') as HTMLElement).click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    expect(apiMock.closeSession).toHaveBeenCalledTimes(1)
    // active：先弹确认，不直接删。
    ;(items[1]!.querySelector('wa-dropdown-item[value="close"]') as HTMLElement).click()
    await el.updateComplete
    expect(apiMock.closeSession).toHaveBeenCalledTimes(1)
    const dialog = el.shadowRoot!.querySelector('wa-dialog[label="Close session"]')
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

  it('the close confirmation names the session by the same label', async () => {
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
    ;(item.querySelector('wa-dropdown-item[value="close"]') as HTMLElement).click()
    await el.updateComplete
    const dialog = el.shadowRoot!.querySelector('wa-dialog[label="Close session"]')
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
    ;(el as any).addPath = '/srv/repo'
    ;(el as any).addNodeId = 'dev-box'
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
    ;(el as any).addPath = '/srv/repo'
    ;(el as any).addNodeId = 'dev-box'
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
    expect(
      (el.shadowRoot!.querySelector('sebas-new-session-dialog') as HTMLElement & {
        open: boolean
      }).open,
    ).toBe(false)
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
    // 成功后对话框收起、无错误。
    expect((await dialogOf(el)).open).toBe(false)
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
    expect((await dialogOf(el)).open).toBe(false)
    expect(mockOf(apiMock.createSession)).not.toHaveBeenCalled()
    el.remove()
  })
})
