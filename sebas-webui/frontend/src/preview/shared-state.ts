/**
 * Shared reactive state for the preview prototype.
 *
 * Projects and active project are shared between the sidebar (project list)
 * and the Workbench view (current project context). Components subscribe to
 * changes rather than reaching into each other's shadow DOM.
 */

export interface ProjectInfo {
  id: string
  name: string
  path: string
  sessions: number
  hasActive: boolean
  hasWaiting: boolean
  gitBranch: string | null
}

// ── Default seed data ──────────────────────────────────────────────────

let projects: ProjectInfo[] = [
  {
    id: 'sebas',
    name: 'sebas',
    path: '/home/user/work/sebas',
    sessions: 2,
    hasActive: true,
    hasWaiting: true,
    gitBranch: 'main',
  },
  {
    id: 'beads',
    name: 'beads',
    path: '/home/user/work/beads',
    sessions: 1,
    hasActive: false,
    hasWaiting: false,
    gitBranch: 'main',
  },
  {
    id: 'dotfiles',
    name: 'dotfiles',
    path: '/home/user/work/dotfiles',
    sessions: 0,
    hasActive: false,
    hasWaiting: false,
    gitBranch: null,
  },
]

let activeProjectId = 'sebas'

// ── Settings drawer ────────────────────────────────────────────────────

let settingsOpen = false

// ── Public accessors (projects) ────────────────────────────────────────

export function getProjects(): ProjectInfo[] {
  return projects
}

export function getActiveProjectId(): string {
  return activeProjectId
}

export function setActiveProjectId(id: string): void {
  activeProjectId = id
  // Keep it pointing at a valid session in the newly-selected project.
  activeSessionId = sessionsByProject[id]?.[0]?.id ?? ''
  notify()
}

export function getActiveProject(): ProjectInfo | undefined {
  return projects.find((p) => p.id === activeProjectId)
}

export function addProject(p: ProjectInfo): void {
  projects = [...projects, p]
  sessionsByProject[p.id] = []
  activeProjectId = p.id
  notify()
}

// ── Sessions (belong to projects) ──────────────────────────────────────

export interface SessionInfo {
  id: string
  label: string
  status: 'active' | 'done'
  turns: number
  lastActive: string
}

export interface ArchivedSession extends SessionInfo {
  projectId: string
  projectName: string
  archivedAt: string
}

let sessionsByProject: Record<string, SessionInfo[]> = {
  sebas: [
    { id: 'sess-1', label: 'Session 1', status: 'active', turns: 5, lastActive: '15:14' },
    { id: 'sess-2', label: 'Session 2', status: 'done', turns: 3, lastActive: '12:30' },
  ],
  beads: [
    { id: 'sess-b1', label: 'Beads Session', status: 'active', turns: 2, lastActive: '11:20' },
  ],
  dotfiles: [],
}

let activeSessionId = 'sess-1'

let archivedSessions: ArchivedSession[] = []

export function getSessionsByProject(): Record<string, SessionInfo[]> {
  return sessionsByProject
}

export function getProjectSessions(projectId: string): SessionInfo[] {
  return sessionsByProject[projectId] ?? []
}

export function getActiveSessionId(): string {
  return activeSessionId
}

export function getArchivedSessions(): ArchivedSession[] {
  return archivedSessions
}

/** Switch project + optionally activate a session within it. */
export function openSession(projectId: string, sessionId?: string): void {
  activeProjectId = projectId
  activeSessionId = sessionId ?? sessionsByProject[projectId]?.[0]?.id ?? ''
  notify()
}

export function addSessionToProject(projectId: string, s: SessionInfo): void {
  sessionsByProject = {
    ...sessionsByProject,
    [projectId]: [...(sessionsByProject[projectId] ?? []), s],
  }
  activeSessionId = s.id
  notify()
}

/** Move a session from its project into the archived (history) list. */
export function archiveSession(projectId: string, sessionId: string): void {
  const project = projects.find((p) => p.id === projectId)
  const session = (sessionsByProject[projectId] ?? []).find((s) => s.id === sessionId)
  if (!session) return
  sessionsByProject = {
    ...sessionsByProject,
    [projectId]: (sessionsByProject[projectId] ?? []).filter((s) => s.id !== sessionId),
  }
  archivedSessions = [
    {
      ...session,
      projectId,
      projectName: project?.name ?? projectId,
      archivedAt: new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }),
    },
    ...archivedSessions,
  ]
  if (activeSessionId === sessionId) {
    const remaining = sessionsByProject[projectId] ?? []
    activeSessionId = remaining.length > 0 ? remaining[0]!.id : ''
  }
  notify()
}

/** Restore an archived session back into its original project. */
export function restoreSession(archiveId: string): void {
  const item = archivedSessions.find((a) => a.id === archiveId)
  if (!item) return
  archivedSessions = archivedSessions.filter((a) => a.id !== archiveId)
  const { archivedAt, projectName, ...rest } = item
  void archivedAt
  void projectName
  sessionsByProject = {
    ...sessionsByProject,
    [item.projectId]: [...(sessionsByProject[item.projectId] ?? []), rest],
  }
  activeProjectId = item.projectId
  activeSessionId = item.id
  notify()
}

// ── Theme & Language ──────────────────────────────────────────────────

let theme: 'dark' | 'light' = 'dark'
let language: 'zh' | 'en' = 'zh'

export function getTheme(): 'dark' | 'light' {
  return theme
}

export function setTheme(t: 'dark' | 'light'): void {
  theme = t
  document.documentElement.classList.toggle('wa-dark', t === 'dark')
  document.documentElement.classList.toggle('wa-light', t === 'light')
  notify()
}

export function getLanguage(): 'zh' | 'en' {
  return language
}

export function setLanguage(l: 'zh' | 'en'): void {
  language = l
  notify()
}

// ── Public accessors (settings) ────────────────────────────────────────

export function isSettingsOpen(): boolean {
  return settingsOpen
}

export function setSettingsOpen(v: boolean): void {
  settingsOpen = v
  notify()
}

// ── Subscription (simple observer) ─────────────────────────────────────

const listeners = new Set<() => void>()

export function subscribe(fn: () => void): () => void {
  listeners.add(fn)
  return () => listeners.delete(fn)
}

function notify(): void {
  listeners.forEach((fn) => fn())
}