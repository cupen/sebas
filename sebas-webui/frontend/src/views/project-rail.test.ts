// @vitest-environment jsdom
/**
 * Sidebar project tree（IA v2）：项目行（计数/分支/悬停动作）、展开后的
 * 嵌套会话行（短 id + 状态点，点击深链 /sessions/:key）、History 组
 * （收纳 project_dir === null 的会话，组头带 "All sessions →" 链接）、
 * 拖拽排序持久化与添加项目表单的错误态。api client 全量 mock，
 * sharedWs 打桩（不真实连 WS）。
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { SessionRow } from '../api/client.js'

const apiMocks = vi.hoisted(() => ({
  sessions: vi.fn(),
  projectsList: vi.fn(),
  projectsBranch: vi.fn(),
  projectsAdd: vi.fn(),
  projectsReorder: vi.fn(),
}))

vi.mock('../api/client.js', () => ({
  api: {
    sessions: apiMocks.sessions,
    projects: {
      list: apiMocks.projectsList,
      branch: apiMocks.projectsBranch,
      add: apiMocks.projectsAdd,
      reorder: apiMocks.projectsReorder,
    },
  },
}))

vi.mock('../api/shared-ws.js', () => ({
  sharedWs: { subscribe: () => () => {} },
}))

import './project-rail.js'
import type { SebasProjectRail } from './project-rail.js'

const projects = [
  { path: '/home/me/alpha', name: 'alpha', added_at: 1, branch: 'main' },
  { path: '/home/me/beta', name: 'beta', added_at: 2, branch: null },
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
    project_dir: null,
    ...overrides,
  }
}

const sessionRows: SessionRow[] = [
  row({ project_dir: '/home/me/alpha' }),
  row({ status_slug: 'done', status: 'done', project_dir: '/home/me/alpha' }),
  row({ status_slug: 'queued', status: 'queued', project_dir: '/home/me/beta' }),
  row({ project_dir: null }), // inbox → History 组
  row({ status_slug: 'dormant', project_dir: null }),
]

async function mount(): Promise<SebasProjectRail> {
  const el = document.createElement('sebas-project-rail') as SebasProjectRail
  document.body.appendChild(el)
  await el.updateComplete
  await new Promise((r) => setTimeout(r, 0))
  await el.updateComplete
  return el
}

beforeEach(() => {
  apiMocks.projectsList.mockResolvedValue({ projects })
  apiMocks.projectsBranch.mockRejectedValue(new Error('no branch lookup here'))
  apiMocks.sessions.mockResolvedValue({ recent_sessions: sessionRows })
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
    // 计数徽标：alpha 2 个会话，beta 1 个。
    const counts = rows.map((r) => r.querySelector('.meta .count')?.textContent ?? '')
    expect(counts).toEqual(['2', '1'])
    // 添加项目的 "+" 挂在分区标签上。
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

    // 展开后：两个嵌套会话行，短 session id + 状态点。
    const items = [...el.shadowRoot!.querySelectorAll('li.session-item')]
    expect(items).toHaveLength(2)
    expect(el.shadowRoot!.textContent).toContain('aaaa0001')
    expect(items[0]!.querySelector('.session-dot')?.getAttribute('data-status')).toBe('working')
    expect(items[1]!.querySelector('.session-dot')?.getAttribute('data-status')).toBe('done')
    el.remove()
  })

  it('navigates to /sessions/:key when a nested session row is clicked', async () => {
    const el = await mount()
    ;(el.shadowRoot!.querySelectorAll('.row')[0] as HTMLElement).click()
    await el.updateComplete

    const first = el.shadowRoot!.querySelector('li.session-item') as HTMLElement
    first.click()
    await el.updateComplete
    // 深链保持 RAW encoded key（含 %00）。
    expect(window.location.pathname).toBe('/sessions/oc_1%00')
    el.remove()
  })

  it('keeps drag-to-reorder persistence via POST /api/projects/reorder', async () => {
    apiMocks.projectsReorder.mockResolvedValue({
      projects: [projects[1], projects[0]],
    })
    const el = await mount()
    const rows = () => [...el.shadowRoot!.querySelectorAll('.row')]
    expect(rows()[0]!.textContent).toContain('alpha')

    // jsdom 没有 DragEvent：拖拽处理器对缺失 dataTransfer 已做防御，
    // 用普通 Event 驱动同一状态机即可。
    rows()[0]!.dispatchEvent(new Event('dragstart', { bubbles: true }))
    rows()[1]!.dispatchEvent(new Event('drop', { bubbles: true }))
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete

    expect(apiMocks.projectsReorder).toHaveBeenCalledWith(['/home/me/beta', '/home/me/alpha'])
    // 服务端返回的顺序回写（beta 置顶）。
    expect(rows()[0]!.textContent).toContain('beta')
    el.remove()
  })

  it('surfaces add-project validation and API errors in the form', async () => {
    const el = await mount()
    ;(el.shadowRoot!.querySelector('.section-label .add-btn') as HTMLElement).click()
    await el.updateComplete
    const form = el.shadowRoot!.querySelector('.add-form')
    expect(form).toBeTruthy()

    // 空路径 → 校验错误。
    ;(form!.querySelector('button.primary') as HTMLElement).click()
    await el.updateComplete
    expect(el.shadowRoot!.textContent).toContain('请输入路径')

    // API 失败 → 错误透出。
    apiMocks.projectsAdd.mockRejectedValue(new Error('directory does not exist'))
    const input = form!.querySelector('input') as HTMLInputElement
    input.value = '/no/such/dir'
    input.dispatchEvent(new Event('input', { bubbles: true }))
    ;(form!.querySelector('button.primary') as HTMLElement).click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    expect(el.shadowRoot!.textContent).toContain('directory does not exist')
    el.remove()
  })
})

describe('history group', () => {
  it('lists project-less sessions under a collapsible header with count and /sessions link', async () => {
    const el = await mount()
    const head = el.shadowRoot!.querySelector('.history-head') as HTMLElement
    expect(head).toBeTruthy()
    expect(head.textContent).toContain('History')
    expect(head.querySelector('.history-count')?.textContent).toBe('2')
    // 旧深链入口：All sessions →
    const all = head.querySelector<HTMLAnchorElement>('a.history-all')
    expect(all?.getAttribute('href')).toBe('/sessions')
    // 默认折叠；点击头展开 inbox 会话行。
    expect(el.shadowRoot!.querySelectorAll('.history-section li.session-item')).toHaveLength(0)
    head.click()
    await el.updateComplete
    const items = [...el.shadowRoot!.querySelectorAll('.history-section li.session-item')]
    expect(items).toHaveLength(2)
    expect(items[0]!.querySelector('.session-dot')?.getAttribute('data-status')).toBe('working')
    el.remove()
  })

  it('stays hidden entirely when every session is bound to a project', async () => {
    apiMocks.sessions.mockResolvedValue({ recent_sessions: [sessionRows[0]] })
    const el = await mount()
    expect(el.shadowRoot!.querySelector('.history-head')).toBeNull()
    el.remove()
  })
})
