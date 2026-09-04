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

const apiMock = vi.mocked(api)

const projects: Project[] = [
  { path: '/home/me/alpha', name: 'alpha', added_at: 0 },
  { path: '/home/me/beta', name: 'beta', added_at: 1 },
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
    prompt_preview: null,
    ...overrides,
  }
}

const sessionRows: SessionRow[] = [
  row({ project_dir: '/home/me/alpha', status: 'working', status_slug: 'working' }),
  row({ project_dir: '/home/me/alpha', status: 'done', status_slug: 'done' }),
  row({ project_dir: null, status: 'working', status_slug: 'working' }),
  row({ project_dir: null, status: 'queued', status_slug: 'queued' }),
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

beforeEach(() => {
  apiMock.projects.list.mockResolvedValue({ projects })
  apiMock.projects.branch.mockRejectedValue(new Error('no branch lookup here'))
  // Mock sessions.
  apiMock.sessions.mockResolvedValue({ recent_sessions: sessionRows as any })
  // Mock archive list (empty by default).
  apiMock.archiveList.mockResolvedValue({ archived_sessions: [] })
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

  it('navigates to /sessions/:key when a nested session row is clicked', async () => {
    const el = await mount()
    ;(el.shadowRoot!.querySelectorAll('.row')[0] as HTMLElement).click()
    await el.updateComplete

    const first = el.shadowRoot!.querySelector('li.session-item') as HTMLElement
    first.click()
    await el.updateComplete
    expect(window.location.pathname).toBe('/sessions/oc_1%00')
    el.remove()
  })

  it('keeps drag-to-reorder persistence via POST /api/projects/reorder', async () => {
    apiMock.projects.reorder.mockResolvedValue({
      projects: [projects[1], projects[0]],
    })
    const el = await mount()
    const rows = () => [...el.shadowRoot!.querySelectorAll('.row')]
    expect(rows()[0]!.textContent).toContain('alpha')

    rows()[0]!.dispatchEvent(new Event('dragstart', { bubbles: true }))
    rows()[1]!.dispatchEvent(new Event('drop', { bubbles: true }))
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete

    expect(apiMock.projects.reorder).toHaveBeenCalledWith(['/home/me/beta', '/home/me/alpha'])
    expect(rows()[0]!.textContent).toContain('beta')
    el.remove()
  })

  it('opens add-project dialog when the + button is clicked', async () => {
    apiMock.fsBrowse.mockResolvedValue({ path: '/home/me', entries: [{ name: 'alpha', is_dir: true }] })
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
    head!.click()
    await el.updateComplete
    const items = [...el.shadowRoot!.querySelectorAll('.group-section li.session-item')]
    expect(items).toHaveLength(2)
    expect(items[0]!.querySelector('.session-dot')?.getAttribute('data-status')).toBe('working')
    el.remove()
  })

  it('stays hidden entirely when every session is bound to a project', async () => {
    apiMock.sessions.mockResolvedValue({ recent_sessions: [sessionRows[0]] as any })
    const el = await mount()
    expect(el.shadowRoot!.querySelector('.group-head')).toBeNull()
    el.remove()
  })
})

describe('history group (archived sessions)', () => {
  it('shows archived sessions from the archive API', async () => {
    apiMock.archiveList.mockResolvedValue({
      archived_sessions: [
        { session_key: 'oc_arch%00', project_path: '/home/me/alpha', label: 'Old session', archived_at: 1000, retention_deadline: 2000 },
      ],
    })
    const el = await mount()
    const heads = [...el.shadowRoot!.querySelectorAll('.group-head')]
    const historyHead = heads.find((h) => h.textContent?.includes('History'))
    expect(historyHead).toBeTruthy()
    expect(historyHead!.querySelector('.group-count')?.textContent).toBe('1')
    historyHead!.click()
    await el.updateComplete
    const items = [...el.shadowRoot!.querySelectorAll('.group-section li.session-item')]
    expect(items).toHaveLength(1)
    expect(items[0]!.classList.contains('archived')).toBe(true)
    el.remove()
  })
})