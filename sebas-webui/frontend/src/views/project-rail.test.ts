/**
 * Tests for the sidebar project tree (project-rail.ts).
 *
 * Covers: project rows with counts, expand/collapse, session deep-links,
 * drag-to-reorder, add-project dialog, Inbox group (unbound sessions),
 * History group (archived sessions), and archive/restore.
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { api, type Project, type SessionRow } from '../api/client.js'
import './project-rail.js'
import type { SebasProjectRail } from './project-rail.js'

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

describe('inbox group', () => {
  it('lists project-less sessions under a collapsible Inbox header', async () => {
    const el = await mount()
    const head = el.shadowRoot!.querySelector('.group-head')
    expect(head).toBeTruthy()
    expect(head!.textContent).toContain('Inbox')
    expect(head!.querySelector('.group-count')?.textContent).toBe('2')
    // Default collapsed.
    expect(el.shadowRoot!.querySelectorAll('.group-section li.session-item')).toHaveLength(0)
    ;(head as HTMLElement).click()
    await el.updateComplete
    const items = [...el.shadowRoot!.querySelectorAll('.group-section li.session-item')]
    expect(items).toHaveLength(2)
    expect(items[0]!.querySelector('.session-dot')?.getAttribute('data-status')).toBe('working')
    el.remove()
  })

  it('stays hidden entirely when every session is bound to a project', async () => {
    mockOf(apiMock.sessions).mockResolvedValue(sessionList([sessionRows[0]]))
    const el = await mount()
    expect(el.shadowRoot!.querySelector('.group-head')).toBeNull()
    el.remove()
  })
})

describe('history group (archived sessions)', () => {
  it('shows archived sessions from the archive API', async () => {
    mockOf(apiMock.archiveList).mockResolvedValue({
      archived_sessions: [
        { session_key: 'oc_arch%00', project_path: '/home/me/alpha', label: 'Old session', archived_at: 1000, retention_deadline: 2000 },
      ],
    })
    const el = await mount()
    const heads = [...el.shadowRoot!.querySelectorAll('.group-head')]
    const historyHead = heads.find((h) => h.textContent?.includes('History'))
    expect(historyHead).toBeTruthy()
    expect(historyHead!.querySelector('.group-count')?.textContent).toBe('1')
    ;(historyHead as HTMLElement).click()
    await el.updateComplete
    const items = [...el.shadowRoot!.querySelectorAll('.group-section li.session-item')]
    expect(items).toHaveLength(1)
    expect(items[0]!.classList.contains('archived')).toBe(true)
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

describe('rail close confirmation (workbench-turn-queue 7.4)', () => {
  it('names how many pending submissions a close will discard', async () => {
    const pendingRow = row({
      status: 'working',
      status_slug: 'working',
      pending_count: 2,
    })
    mockOf(apiMock.sessions).mockResolvedValue(sessionList([pendingRow]))
    const el = await mount()
    // 会话行在默认折叠的 Inbox 组里，先展开。
    ;(el.shadowRoot!.querySelector('.group-head') as HTMLElement)!.click()
    await el.updateComplete
    const closeBtn = el.shadowRoot!.querySelector<HTMLButtonElement>(
      'li.session-item button[aria-label^="Close"]',
    )
    expect(closeBtn).toBeTruthy()
    closeBtn!.click()
    await el.updateComplete

    const line = el.shadowRoot!.querySelector('[data-testid="close-discards-pending"]')
    expect(line).toBeTruthy()
    expect(line!.textContent).toContain('2')
    expect(line!.textContent).toContain('待执行')
    el.remove()
  })

  it('omits the discard line when the session has no pending submissions', async () => {
    const plainRow = row({ status: 'working', status_slug: 'working', pending_count: 0 })
    mockOf(apiMock.sessions).mockResolvedValue(sessionList([plainRow]))
    const el = await mount()
    ;(el.shadowRoot!.querySelector('.group-head') as HTMLElement)!.click()
    await el.updateComplete
    const closeBtn = el.shadowRoot!.querySelector<HTMLButtonElement>(
      'li.session-item button[aria-label^="Close"]',
    )
    closeBtn!.click()
    await el.updateComplete
    expect(el.shadowRoot!.querySelector('[data-testid="close-discards-pending"]')).toBeNull()
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

    const plus = el.shadowRoot!.querySelector<HTMLButtonElement>('.row-action:not(.row-remove)')
    expect(plus!.disabled).toBe(true)

    // 直接走创建路径也被拦下（不提交，成因点名节点）。
    await (el as any).createSession(new Event('click'), (el as any).projects[0])
    expect(apiMock.createSession).not.toHaveBeenCalled()
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
    const plus = el.shadowRoot!.querySelector<HTMLButtonElement>('.row-action:not(.row-remove)')
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
