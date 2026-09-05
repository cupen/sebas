/**
 * Preview app shell: sidebar with project list + settings at bottom.
 * Settings opens as a centered modal dialog with left-nav sections:
 *   UI (theme/language), Models, Services, Env, About.
 * Launch via `npm run dev` and open `http://127.0.0.1:5273/preview.html`.
 */

import { LitElement, css, html, nothing } from 'lit'
import { customElement, state } from 'lit/decorators.js'
import { icon } from '../components/icons.js'
import {
  getProjects,
  getActiveProjectId,
  getActiveSessionId,
  getProjectSessions,
  getArchivedSessions,
  setActiveProjectId,
  openSession,
  addProject,
  addSessionToProject,
  archiveSession,
  restoreSession,
  isSettingsOpen,
  setSettingsOpen,
  getTheme,
  setTheme,
  getLanguage,
  setLanguage,
  subscribe,
  type ProjectInfo,
  type SessionInfo,
  type ArchivedSession,
} from './shared-state.js'

// Web Awesome primitives
import '@awesome.me/webawesome/dist/styles/webawesome.css'
import '@awesome.me/webawesome/dist/styles/themes/default.css'
import '../styles/wa-overrides.css'
import '@awesome.me/webawesome/dist/components/button/button.js'
import '@awesome.me/webawesome/dist/components/icon/icon.js'
import '@awesome.me/webawesome/dist/components/divider/divider.js'
import '@awesome.me/webawesome/dist/components/select/select.js'
import '@awesome.me/webawesome/dist/components/option/option.js'
import '@awesome.me/webawesome/dist/components/input/input.js'
import '@awesome.me/webawesome/dist/components/textarea/textarea.js'
import '@awesome.me/webawesome/dist/components/dialog/dialog.js'
import '@awesome.me/webawesome/dist/components/tab/tab.js'
import '@awesome.me/webawesome/dist/components/tab-panel/tab-panel.js'
import '@awesome.me/webawesome/dist/components/tab-group/tab-group.js'
import '@awesome.me/webawesome/dist/components/badge/badge.js'
import '@awesome.me/webawesome/dist/components/tag/tag.js'
import '@awesome.me/webawesome/dist/components/checkbox/checkbox.js'
import '@awesome.me/webawesome/dist/components/switch/switch.js'
import '@awesome.me/webawesome/dist/components/spinner/spinner.js'
import '@awesome.me/webawesome/dist/components/dropdown/dropdown.js'
import '@awesome.me/webawesome/dist/components/dropdown-item/dropdown-item.js'

// Preview views
import './views/workbench.js'

type SettingsSection = 'ui' | 'models' | 'services' | 'network' | 'env' | 'about'

// ── Provider model ─────────────────────────────────────────────────────

interface ProviderState {
  id: string
  name: string
  preset: string | null // preset key, or null for custom
  protocol: string
  baseUrl: string
  apiKeyConfigured: boolean
  defaultModel: string | null
  modelRatings: Record<string, ModelRating | null> // model id → rating flag
}

// Rating flags available on the optional per-model tier chip.
type ModelRating = 'T0' | 'T1' | 'T2'

// Well-known model ids offered in the "add model" submenu.
const MODEL_OPTIONS: string[] = [
  'deepseek-v4-flash',
  'deepseek-v4-pro',
  'claude-sonnet-4',
  'claude-opus-4',
  'gpt-4o',
  'gpt-4o-mini',
  'o4-mini',
]

interface PresetDef {
  key: string
  label: string
  protocol: string
  baseUrl: string
  defaultModel: string
  models: string[]
}

function modelsToRatings(list: string[]): Record<string, ModelRating | null> {
  const out: Record<string, ModelRating | null> = {}
  for (const m of list) out[m.trim()] = null
  return out
}

const PRESETS: PresetDef[] = [
  {
    key: 'deepseek',
    label: 'DeepSeek',
    protocol: 'anthropic',
    baseUrl: 'https://api.deepseek.com/v1',
    defaultModel: 'deepseek-v4-flash',
    models: ['deepseek-v4-flash', 'deepseek-v4-pro'],
  },
  {
    key: 'anthropic',
    label: 'Anthropic',
    protocol: 'anthropic',
    baseUrl: 'https://api.anthropic.com/v1',
    defaultModel: 'claude-sonnet-4',
    models: ['claude-sonnet-4', 'claude-opus-4'],
  },
  {
    key: 'openai',
    label: 'OpenAI',
    protocol: 'openai',
    baseUrl: 'https://api.openai.com/v1',
    defaultModel: 'gpt-4o',
    models: ['gpt-4o', 'gpt-4o-mini'],
  },
]

function presetLabel(key: string): string {
  return PRESETS.find((p) => p.key === key)?.label ?? key
}

@customElement('sebas-preview-app')
export class SebasPreviewApp extends LitElement {
  @state() private projects: ProjectInfo[] = getProjects()
  @state() private activeProjectId = getActiveProjectId()
  @state() private settingsOpen = isSettingsOpen()
  @state() private settingsSection: SettingsSection = 'ui'
  @state() private addProjectDialogOpen = false
  @state() private manualPath = ''

  // Provider editor
  @state() private providers: ProviderState[] = [
    {
      id: 'alpha',
      name: 'alpha',
      preset: 'deepseek',
      protocol: 'anthropic',
      baseUrl: 'https://api.deepseek.com/v1',
      apiKeyConfigured: true,
      defaultModel: 'deepseek-v4-flash',
      modelRatings: { 'deepseek-v4-flash': 'T0', 'deepseek-v4-pro': 'T0' },
    },
    {
      id: 'zeta',
      name: 'zeta',
      preset: null,
      protocol: 'openai',
      baseUrl: 'https://api.openai.com/v1',
      apiKeyConfigured: false,
      defaultModel: null,
      modelRatings: {},
    },
  ]
  @state() private providerEditorOpen = false
  @state() private editingProviderId: string | null = null

  // Provider editor form fields
  @state() private formPresetKey: string | null = null
  @state() private formName = ''
  @state() private formProtocol = 'anthropic'
  @state() private formBaseUrl = ''
  @state() private formApiKey = ''
  @state() private formDefaultModel = ''
  @state() private formModelRatings: Record<string, ModelRating | null> = {}
  @state() private formModelsFilter = ''
  @state() private presetMenuOpen = false
  @state() private modelsMenuOpen = false

  // Theme & language
  @state() private theme = getTheme()
  @state() private language = getLanguage()

  // Session tree in sidebar
  @state() private activeSessionId = getActiveSessionId()
  @state() private expandedProjects: Record<string, boolean> = { sebas: true, beads: true }
  // Bumped on every shared-state notify so the sidebar tree re-reads sessions
  @state() private sessionsTick = 0
  @state() private archivedSessions: ArchivedSession[] = getArchivedSessions()
  // History section collapsed by default
  @state() private historyExpanded = false

  // Mobile: switch between project list and chat panel
  @state() private mobileTab: 'projects' | 'chat' = 'chat'
  @state() private isMobile = window.matchMedia('(max-width: 900px)').matches

  // Network proxy — committed state + editable draft
  @state() private proxyEnabled = true
  @state() private proxyMode: 'http' | 'https' | 'socks' | 'socks5h' = 'http'
  @state() private proxyHost = '127.0.0.1'
  @state() private proxyPort = '7890'
  @state() private proxyUsername = ''
  @state() private proxyPassword = ''
  // Draft values edited in the form; only Save copies them onto the committed state.
  @state() private draftProxyEnabled = true
  @state() private draftProxyMode: 'http' | 'https' | 'socks' | 'socks5h' = 'http'
  @state() private draftProxyHost = '127.0.0.1'
  @state() private draftProxyPort = '7890'
  @state() private draftProxyUsername = ''
  @state() private draftProxyPassword = ''

  /** All model ids across every provider, used as options in the add-model submenu. */
  private get allModelOptions(): string[] {
    const known = [...MODEL_OPTIONS]
    for (const p of this.providers) {
      for (const m of Object.keys(p.modelRatings)) if (!known.includes(m)) known.push(m)
    }
    return [...new Set(known)]
  }

  private unsub?: () => void
  private mobileMq?: MediaQueryList
  private onMqChange: () => void = () => {}

  connectedCallback(): void {
    super.connectedCallback()
    let prevOpen = isSettingsOpen()
    this.unsub = subscribe(() => {
      this.projects = getProjects()
      this.activeProjectId = getActiveProjectId()
      this.activeSessionId = getActiveSessionId()
      this.sessionsTick++
      this.archivedSessions = getArchivedSessions()
      this.settingsOpen = isSettingsOpen()
      // When the settings dialog opens, reset the Network draft to committed
      // values so stale edits never leak into a fresh open.
      if (this.settingsOpen && !prevOpen) this.resetProxyDraft()
      prevOpen = this.settingsOpen
      this.theme = getTheme()
      this.language = getLanguage()
    })
    this.mobileMq = window.matchMedia('(max-width: 900px)')
    this.onMqChange = () => (this.isMobile = this.mobileMq!.matches)
    this.mobileMq.addEventListener('change', this.onMqChange)
  }

  disconnectedCallback(): void {
    this.unsub?.()
    this.mobileMq?.removeEventListener('change', this.onMqChange)
    super.disconnectedCallback()
  }

  static styles = css`
    :host {
      display: flex;
      width: 100vw;
      height: 100vh;
      min-height: 0;
      overflow: hidden;
      background: var(--sebas-bg);
      background-image: radial-gradient(1100px 480px at 82% -12%, rgba(91,100,242,0.09), transparent 62%),
                        radial-gradient(900px 420px at -8% 108%, rgba(56,209,221,0.05), transparent 60%);
      background-attachment: fixed;
      color: var(--sebas-text);
      font-family: var(--sebas-font-sans);
      font-size: 15px;
      line-height: 1.55;
      -webkit-font-smoothing: antialiased;
    }
    nav {
      width: 220px;
      flex: 0 0 auto;
      position: sticky;
      top: 0;
      height: 100%;
      min-height: 0;
      overflow-y: auto;
      background: var(--sebas-surface);
      border-right: 1px solid var(--sebas-border);
      padding: var(--sebas-space-4) var(--sebas-space-3);
      display: flex;
      flex-direction: column;
      gap: 2px;
    }
    nav .sidebar-footer {
      position: sticky;
      bottom: -12px; /* pull just below the nav's bottom padding so the button rides the visible edge */
      margin-top: auto;
      background: var(--sebas-surface);
      padding-top: var(--sebas-space-2);
      z-index: 1;
    }
    .brand {
      display: flex;
      align-items: center;
      gap: var(--sebas-space-3);
      padding: 0 var(--sebas-space-2) var(--sebas-space-4);
      text-decoration: none;
      color: var(--sebas-text-bright);
    }
    .brand .mark {
      display: grid;
      place-items: center;
      width: 28px;
      height: 28px;
      flex: 0 0 auto;
      border-radius: var(--sebas-radius-md);
      background: linear-gradient(135deg, var(--sebas-accent-strong), #4338ca);
      color: var(--sebas-accent-ink);
      font-family: var(--sebas-font-mono);
      font-size: 0.9rem;
      font-weight: 700;
      box-shadow: 0 1px 2px rgba(0,0,0,0.3), inset 0 1px 0 rgba(255,255,255,0.18);
    }
    .brand .name { font-weight: 700; font-size: 1rem; letter-spacing: 0.01em; }
    .brand .name small { display: block; font-weight: 500; font-size: 0.66rem; letter-spacing: 0.09em; text-transform: uppercase; color: var(--sebas-text-faint); }

    .section-label {
      display: flex;
      align-items: center;
      gap: var(--sebas-space-1);
      padding: var(--sebas-space-3) var(--sebas-space-2) var(--sebas-space-1);
      font-size: 0.7rem;
      font-weight: 600;
      text-transform: uppercase;
      letter-spacing: 0.08em;
      color: var(--sebas-text-faint);
    }
    .section-label .add-btn {
      margin-left: auto;
      background: none;
      border: none;
      color: var(--sebas-text-faint);
      cursor: pointer;
      padding: 2px;
      border-radius: var(--sebas-radius-sm);
      display: grid;
      place-items: center;
      width: 18px;
      height: 18px;
    }
    .section-label .add-btn:hover {
      color: var(--sebas-accent);
      background: var(--sebas-accent-soft);
    }
    .project-item {
      display: flex;
      align-items: center;
      gap: 8px;
      padding: 6px 10px;
      border-radius: var(--sebas-radius-md);
      color: var(--sebas-text-dim);
      font-size: 0.85rem;
      font-weight: 500;
      cursor: pointer;
      transition: background var(--sebas-dur) var(--sebas-ease), color var(--sebas-dur) var(--sebas-ease);
      margin: 1px 0;
    }
    .project-item:hover {
      background: var(--sebas-surface-2);
      color: var(--sebas-text-bright);
    }
    .project-item[aria-current='true'] {
      background: var(--sebas-accent-soft);
      color: var(--sebas-accent);
    }
    .project-item[aria-current='true'] .project-name { color: var(--sebas-accent); }
    .project-item .active-dot {
      width: 6px; height: 6px; border-radius: 50%; flex: 0 0 auto;
      background: var(--sebas-status-working);
    }
    .project-item .project-name {
      flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
    }
    .project-item .project-branch {
      font-family: var(--sebas-font-mono); font-size: 0.62rem; color: var(--sebas-accent);
      background: var(--sebas-accent-soft); border-radius: var(--sebas-radius-full);
      padding: 0 6px; line-height: 16px;
    }
    .project-item .project-count {
      font-size: 0.65rem;
      color: var(--sebas-text-faint);
      font-variant-numeric: tabular-nums;
    }
    .project-item .chevron {
      display: grid; place-items: center;
      width: 14px; height: 14px; flex: 0 0 auto;
      color: var(--sebas-text-faint);
      transition: transform var(--sebas-dur) var(--sebas-ease);
      font-size: 9px;
    }
    .project-item .chevron.open { transform: rotate(90deg); }

    /* Sessions nested under a project */
    .session-item {
      display: flex;
      align-items: center;
      gap: 8px;
      padding: 4px 10px 4px 30px;
      border-radius: var(--sebas-radius-md);
      color: var(--sebas-text-dim);
      font-size: 0.8rem;
      cursor: pointer;
      transition: background var(--sebas-dur) var(--sebas-ease), color var(--sebas-dur) var(--sebas-ease);
      margin: 1px 0;
    }
    .session-item:hover {
      background: var(--sebas-surface-2);
      color: var(--sebas-text-bright);
    }
    .session-item[aria-current='true'] {
      background: var(--sebas-accent-soft);
      color: var(--sebas-accent);
    }
    .session-item .session-dot {
      width: 6px; height: 6px; border-radius: 50%; flex: 0 0 auto;
    }
    .session-item .session-dot.active { background: var(--sebas-status-working); }
    .session-item .session-dot.done { background: var(--sebas-status-done); }
    .session-item .session-name {
      flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
    }

    /* Archive (icon) button on each session row */
    .session-archive-btn {
      background: none;
      border: none;
      color: var(--sebas-text-faint);
      cursor: pointer;
      padding: 2px 4px;
      border-radius: var(--sebas-radius-sm);
      display: grid;
      place-items: center;
      opacity: 0;
      transition: opacity var(--sebas-dur) var(--sebas-ease), color var(--sebas-dur) var(--sebas-ease), background var(--sebas-dur) var(--sebas-ease);
    }
    .session-item:hover .session-archive-btn { opacity: 1; }
    .session-archive-btn:hover {
      color: var(--sebas-status-warning);
      background: var(--sebas-surface-3);
    }

    /* Per-project +button: create new session */
    .project-add-btn {
      width: 20px;
      height: 20px;
      margin-left: 2px;
      background: none;
      border: 1px solid var(--sebas-border);
      border-radius: var(--sebas-radius-sm);
      color: var(--sebas-text-faint);
      cursor: pointer;
      font-size: 14px;
      line-height: 1;
      display: grid;
      place-items: center;
      transition: color var(--sebas-dur) var(--sebas-ease), background var(--sebas-dur) var(--sebas-ease), border-color var(--sebas-dur) var(--sebas-ease);
    }
    .project-item:hover .project-add-btn { color: var(--sebas-accent); border-color: var(--sebas-accent-border); }
    .project-add-btn:hover { color: var(--sebas-accent); background: var(--sebas-accent-soft); }

    /* History (archived sessions) section */
    .history-section { margin-top: var(--sebas-space-3); }
    .history-head {
      display: flex;
      align-items: center;
      gap: 6px;
      padding: 4px 10px;
      font-size: 0.66rem;
      font-weight: 600;
      text-transform: uppercase;
      letter-spacing: 0.08em;
      color: var(--sebas-text-faint);
      cursor: pointer;
      user-select: none;
      border-radius: var(--sebas-radius-md);
    }
    .history-head:hover { background: var(--sebas-surface-2); color: var(--sebas-text-dim); }
    .history-head .chevron { font-size: 9px; transition: transform var(--sebas-dur) var(--sebas-ease); }
    .history-head .chevron.open { transform: rotate(90deg); }
    .history-head .history-count {
      margin-left: auto;
      font-variant-numeric: tabular-nums;
      background: var(--sebas-surface-3);
      border-radius: var(--sebas-radius-full);
      padding: 0 7px;
      font-size: 0.62rem;
    }
    .session-item.archived {
      padding-left: 20px;
      color: var(--sebas-text-faint);
    }
    .session-item.archived:hover {
      color: var(--sebas-accent);
      cursor: pointer;
    }
    .session-item.archived .archive-meta {
      font-size: 0.65rem;
      color: var(--sebas-text-faint);
      font-family: var(--sebas-font-mono);
      padding-right: 4px;
    }
    .session-item.archived[title]::after { content: ''; }
    .spacer { flex: 1; }
    .settings-btn {
      display: flex; align-items: center; gap: 10px;
      padding: 7px 10px; border-radius: var(--sebas-radius-md);
      color: var(--sebas-text-dim); font-size: 0.85rem; font-weight: 500;
      cursor: pointer; background: none; border: none; width: 100%; text-align: left;
      transition: background var(--sebas-dur) var(--sebas-ease), color var(--sebas-dur) var(--sebas-ease);
    }
    .settings-btn:hover { background: var(--sebas-surface-2); color: var(--sebas-text-bright); }
    .settings-btn svg { opacity: 0.8; flex: 0 0 auto; }

    main {
      flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column;
    }
    .outlet { flex: 1; min-height: 0; display: flex; flex-direction: column; }
    .outlet > * { animation: sebas-view-in 0.28s var(--sebas-ease) both; }
    .outlet > * { min-height: 0; }
    @keyframes sebas-view-in {
      from { opacity: 0; transform: translateY(6px); }
      to { opacity: 1; transform: none; }
    }
    @media (prefers-reduced-motion: reduce) { .outlet > * { animation: none; } }
    /* Mobile: nav collapses to a top bar, workbench fills the rest */
    @media (max-width: 900px) {
      :host { flex-direction: column; }
      nav {
        position: static; height: auto; min-height: 0; width: auto; flex: 0 0 auto;
        flex-direction: row; align-items: center; flex-wrap: wrap;
        gap: var(--sebas-space-1);
        border-right: none; border-bottom: 1px solid var(--sebas-border);
        padding: var(--sebas-space-2) var(--sebas-space-3);
      }
      .brand { padding: 0 var(--sebas-space-3) 0 0; }
      .brand .mark { width: 24px; height: 24px; font-size: 0.8rem; }
      .brand .name { font-size: 0.9rem; }
      .brand .name small { display: none; }
      .section-label, .project-item, .spacer { display: none; }
      nav .sidebar-footer { position: static; margin-top: 0; padding-top: 0; background: none; }
      .settings-btn {
        width: auto;
        margin-left: auto;
        padding: 6px;
        gap: 0;
      }
      .settings-btn .settings-label { display: none; }
    }
    /* Mobile top tabs: Projects / Chat */
    .mobile-tabs {
      display: none;
    }
    .mobile-projects-panel {
      display: none;
    }
    @media (max-width: 900px) {
      .mobile-tabs {
        display: flex;
        flex: 0 0 auto;
        border-bottom: 1px solid var(--sebas-border);
        background: var(--sebas-surface);
      }
      .mobile-tab {
        flex: 1;
        display: flex;
        align-items: center;
        justify-content: center;
        gap: 6px;
        padding: 9px 0;
        font-size: 0.85rem;
        font-weight: 500;
        color: var(--sebas-text-dim);
        background: none;
        border: none;
        border-bottom: 2px solid transparent;
        cursor: pointer;
        transition: color var(--sebas-dur) var(--sebas-ease), border-color var(--sebas-dur) var(--sebas-ease);
      }
      .mobile-tab:hover { color: var(--sebas-text-bright); }
      .mobile-tab[aria-current='true'] {
        color: var(--sebas-accent);
        border-bottom-color: var(--sebas-accent);
      }
      .mobile-projects-panel {
        display: none;
      }
      .mobile-projects-panel.panel-active {
        display: flex;
        flex: 1;
        min-height: 0;
        flex-direction: column;
        overflow-y: auto;
        padding: var(--sebas-space-3);
      }
      .mobile-projects-panel .mobile-panel-head {
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: 0 var(--sebas-space-2) var(--sebas-space-2);
        font-size: 0.7rem;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.08em;
        color: var(--sebas-text-faint);
      }
      .mobile-projects-panel .mobile-panel-head .add-btn {
        background: none;
        border: none;
        color: var(--sebas-text-faint);
        cursor: pointer;
        padding: 2px;
        border-radius: var(--sebas-radius-sm);
        font-size: 1rem;
        line-height: 1;
      }
      .mobile-projects-panel .mobile-panel-head .add-btn:hover {
        color: var(--sebas-accent);
        background: var(--sebas-accent-soft);
      }
      .mobile-projects-panel .project-item,
      .mobile-projects-panel .session-item {
        display: flex;
      }
      .mobile-hidden {
        display: none !important;
      }
    }

    /* ── Settings dialog: centered modal with left nav ── */
    .settings-dialog {
      --width: 900px;
    }
    .settings-dialog::part(dialog) {
      width: 900px;
      max-width: 94vw;
      height: 560px;
      max-height: 80vh;
      border-radius: var(--sebas-radius-xl);
      overflow: hidden;
    }
    .settings-dialog::part(title) {
      display: none; /* we have our own header */
    }
    .settings-dialog::part(body) {
      padding: 0;
      overflow: hidden;
    }

    /* ── Provider editor dialog ── */
    .provider-dialog {
      --width: 560px;
    }
    .provider-dialog::part(body) {
      padding: var(--sebas-space-4) var(--sebas-space-4) 0;
      max-height: 60vh;
      overflow-y: auto;
    }
    .provider-form-field {
      display: flex;
      flex-direction: column;
      gap: var(--sebas-space-1);
    }
    .provider-form-field .form-label {
      font-size: 0.75rem;
      font-weight: 600;
      text-transform: uppercase;
      letter-spacing: 0.06em;
      color: var(--sebas-text-faint);
    }
    .provider-form-grid {
      display: grid;
      grid-template-columns: 1fr 1fr;
      gap: var(--sebas-space-3);
    }
    .preset-grid {
      display: grid;
      grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
      gap: var(--sebas-space-2);
    }
    .preset-card {
      display: flex;
      flex-direction: column;
      gap: 2px;
      padding: var(--sebas-space-2) var(--sebas-space-3);
      background: var(--sebas-surface-2);
      border: 1px solid var(--sebas-border);
      border-radius: var(--sebas-radius-md);
      color: var(--sebas-text-dim);
      cursor: pointer;
      text-align: left;
      transition: border-color var(--sebas-dur) var(--sebas-ease), background var(--sebas-dur) var(--sebas-ease), color var(--sebas-dur) var(--sebas-ease);
    }
    .preset-card:hover {
      border-color: var(--sebas-accent-border);
      color: var(--sebas-text-bright);
    }
    .preset-card.active {
      border-color: var(--sebas-accent-border);
      background: var(--sebas-accent-soft);
      color: var(--sebas-accent);
    }
    .preset-card .preset-name {
      font-size: 0.85rem;
      font-weight: 600;
    }
    .preset-card .preset-models {
      font-size: 0.7rem;
      font-family: var(--sebas-font-mono);
      color: var(--sebas-text-faint);
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
    }
    .preset-card.active .preset-models {
      color: var(--sebas-accent);
      opacity: 0.8;
    }
    .settings-layout {
      display: flex;
      height: 100%;
      position: relative;
    }
    .settings-close {
      position: absolute;
      top: 6px;
      right: 6px;
      z-index: 10;
      background: none;
      border: none;
      color: var(--sebas-text-faint);
      cursor: pointer;
      padding: 4px;
      border-radius: var(--sebas-radius-sm);
      display: grid;
      place-items: center;
      width: 24px;
      height: 24px;
      transition: color var(--sebas-dur) var(--sebas-ease), background var(--sebas-dur) var(--sebas-ease);
    }
    .settings-close:hover {
      color: var(--sebas-text-bright);
      background: var(--sebas-surface-3);
    }
    .settings-nav {
      width: 130px;
      flex: 0 0 auto;
      background: var(--sebas-surface-2);
      border-right: 1px solid var(--sebas-border);
      padding: var(--sebas-space-2) 0;
      display: flex;
      flex-direction: column;
      gap: 0;
      overflow-y: auto;
    }
    .settings-nav .nav-item {
      display: flex;
      align-items: center;
      gap: 6px;
      padding: 6px 12px;
      font-size: 0.78rem;
      font-weight: 500;
      color: var(--sebas-text-dim);
      cursor: pointer;
      border: none;
      background: none;
      text-align: left;
      transition: background var(--sebas-dur) var(--sebas-ease), color var(--sebas-dur) var(--sebas-ease);
    }
    .settings-nav .nav-item:hover {
      background: var(--sebas-surface-3);
      color: var(--sebas-text-bright);
    }
    .settings-nav .nav-item[aria-current='true'] {
      background: var(--sebas-accent-soft);
      color: var(--sebas-accent);
    }
    .settings-nav .nav-item svg { opacity: 0.7; flex: 0 0 auto; }
    .settings-nav .nav-item[aria-current='true'] svg { opacity: 1; }
    .settings-nav .nav-section-title {
      font-size: 0.6rem;
      font-weight: 600;
      text-transform: uppercase;
      letter-spacing: 0.08em;
      color: var(--sebas-text-faint);
      padding: var(--sebas-space-2) 12px var(--sebas-space-1);
    }
    .settings-content {
      flex: 1;
      padding: var(--sebas-space-3) var(--sebas-space-4);
      overflow-y: auto;
    }
    .settings-content h3 {
      margin: 0 0 var(--sebas-space-2);
      font-size: 1rem;
      font-weight: 700;
      color: var(--sebas-text-bright);
    }
    .settings-content .desc {
      font-size: 0.8rem;
      color: var(--sebas-text-dim);
      margin: 0 0 var(--sebas-space-4);
    }

    /* UI section */
    .pref-row {
      display: flex;
      align-items: center;
      gap: var(--sebas-space-3);
      padding: var(--sebas-space-3) var(--sebas-space-4);
      background: var(--sebas-surface-2);
      border: 1px solid var(--sebas-border);
      border-radius: var(--sebas-radius-lg);
      margin-bottom: var(--sebas-space-3);
    }
    .pref-row label {
      font-size: 0.85rem;
      color: var(--sebas-text);
      min-width: 80px;
    }

    /* Models section — compact provider list */
    .provider-list {
      display: flex;
      flex-direction: column;
      gap: var(--sebas-space-2);
    }
    .provider-row {
      display: flex;
      align-items: center;
      gap: var(--sebas-space-3);
      padding: var(--sebas-space-2) var(--sebas-space-3);
      background: var(--sebas-surface-2);
      border: 1px solid var(--sebas-border);
      border-radius: var(--sebas-radius-lg);
      transition: border-color var(--sebas-dur) var(--sebas-ease);
    }
    .provider-row:hover {
      border-color: var(--sebas-accent-border);
    }
    .provider-row-name {
      font-weight: 600;
      font-size: 0.88rem;
      color: var(--sebas-text-bright);
      min-width: 0;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
    .provider-row-preset {
      font-size: 0.72rem;
      color: var(--sebas-accent);
      font-family: var(--sebas-font-mono);
      background: var(--sebas-accent-soft);
      border-radius: var(--sebas-radius-full);
      padding: 1px 9px;
      flex: 0 0 auto;
    }
    .provider-row-count {
      font-size: 0.7rem;
      color: var(--sebas-text-faint);
      font-variant-numeric: tabular-nums;
      margin-left: auto;
    }
    .provider-row-actions {
      display: flex;
      gap: 2px;
    }
    .row-action {
      display: grid;
      place-items: center;
      width: 26px;
      height: 26px;
      background: none;
      border: none;
      border-radius: var(--sebas-radius-sm);
      color: var(--sebas-text-faint);
      cursor: pointer;
      transition: background var(--sebas-dur) var(--sebas-ease), color var(--sebas-dur) var(--sebas-ease);
    }
    .row-action:hover {
      background: var(--sebas-surface-3);
      color: var(--sebas-text-bright);
    }
    .row-action.danger:hover {
      background: var(--sebas-status-failed-bg, rgba(244, 63, 94, 0.12));
      color: var(--sebas-status-failed);
    }
    .add-provider-card {
      display: flex;
      align-items: center;
      justify-content: center;
      gap: var(--sebas-space-2);
      padding: var(--sebas-space-4);
      border: 1px dashed var(--sebas-border);
      border-radius: var(--sebas-radius-lg);
      color: var(--sebas-text-faint);
      font-size: 0.85rem;
      cursor: pointer;
      transition: border-color var(--sebas-dur) var(--sebas-ease), color var(--sebas-dur) var(--sebas-ease);
    }
    .add-provider-card:hover { border-color: var(--sebas-accent-border); color: var(--sebas-accent); }

    /* Provider form: dynamic model chip editor */
    .model-editor {
      display: flex;
      flex-direction: column;
      gap: var(--sebas-space-2);
    }
    .model-editor-row {
      display: flex;
      flex-wrap: wrap;
      gap: 6px;
      align-items: center;
      min-height: 32px;
    }
    .model-chip {
      display: inline-flex;
      align-items: center;
      gap: 6px;
      height: 26px;
      padding: 0 8px 0 10px;
      border-radius: var(--sebas-radius-full);
      background: var(--sebas-surface-3);
      border: 1px solid var(--sebas-border);
      font-family: var(--sebas-font-mono);
      font-size: 0.74rem;
      color: var(--sebas-text);
      cursor: pointer;
      user-select: none;
      transition: border-color var(--sebas-dur) var(--sebas-ease), background var(--sebas-dur) var(--sebas-ease);
    }
    .model-chip:hover { border-color: var(--sebas-accent-border); }
    .model-chip .model-chip-name {
      max-width: 200px;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
    .model-chip .model-chip-x {
      display: grid;
      place-items: center;
      width: 16px;
      height: 16px;
      border-radius: 50%;
      color: var(--sebas-text-faint);
      line-height: 1;
      font-size: 10px;
      background: transparent;
      border: none;
      cursor: pointer;
      transition: background var(--sebas-dur) var(--sebas-ease), color var(--sebas-dur) var(--sebas-ease);
    }
    .model-chip .model-chip-x:hover {
      background: var(--sebas-status-failed-bg, rgba(244, 63, 94, 0.15));
      color: var(--sebas-status-failed);
    }
    .model-chip.rating-null { color: var(--sebas-text-dim); }
    .model-chip.rating-T0 { border-color: var(--sebas-status-done); color: var(--sebas-status-done); background: rgba(34, 197, 94, 0.1); }
    .model-chip.rating-T1 { border-color: var(--sebas-status-working); color: var(--sebas-status-working); background: rgba(234, 179, 8, 0.1); }
    .model-chip.rating-T2 { border-color: var(--sebas-status-queued); color: var(--sebas-status-queued); background: rgba(148, 163, 184, 0.12); }
    .model-add-btn {
      display: inline-flex;
      align-items: center;
      justify-content: center;
      gap: 4px;
      height: 26px;
      padding: 0 12px;
      border-radius: var(--sebas-radius-full);
      border: 1px dashed var(--sebas-border);
      background: none;
      color: var(--sebas-text-faint);
      font-size: 0.74rem;
      cursor: pointer;
      transition: border-color var(--sebas-dur) var(--sebas-ease), color var(--sebas-dur) var(--sebas-ease);
    }
    .model-add-btn:hover {
      border-color: var(--sebas-accent-border);
      color: var(--sebas-accent);
    }
    .model-add-btn svg { flex: 0 0 auto; }
    .model-editor-hint {
      font-size: 0.72rem;
      color: var(--sebas-text-faint);
    }
    .model-editor-hint .chip-demo { color: var(--sebas-text-dim); font-family: var(--sebas-font-mono); }

    /* Provider form: add-model dropdown */
    .model-dropdown { display: inline-flex; }
    .model-dropdown::part(menu) {
      background: var(--sebas-surface);
      border: 1px solid var(--sebas-border);
      border-radius: var(--sebas-radius-lg);
      box-shadow: var(--sebas-shadow-l);
      padding: 4px;
      min-width: 220px;
      max-height: 260px;
      overflow-y: auto;
    }
    wa-dropdown-item.model-option {
      --wa-color-text-normal: var(--sebas-text);
      --wa-color-text-quiet: var(--sebas-text-dim);
    }
    wa-dropdown-item.model-option::part(base) {
      padding: 7px 10px;
      font-size: 0.82rem;
      border-radius: var(--sebas-radius-md);
      font-family: var(--sebas-font-mono);
    }

    /* Provider form: searchable preset dropdown */
    .preset-dropdown { display: inline-block; }
    .preset-dropdown::part(menu) {
      background: var(--sebas-surface);
      border: 1px solid var(--sebas-border);
      border-radius: var(--sebas-radius-lg);
      box-shadow: var(--sebas-shadow-l);
      padding: 4px;
      min-width: 200px;
    }
    wa-dropdown-item.preset-option::part(base) {
      padding: 7px 10px;
      font-size: 0.84rem;
      border-radius: var(--sebas-radius-md);
    }
    .preset-trigger-label {
      font-size: 0.84rem;
      font-weight: 500;
    }
    .preset-trigger-chev {
      font-size: 0.72rem;
      opacity: 0.7;
      margin-left: 8px;
    }
    .model-menu-search {
      padding: 6px 6px 2px;
      border-bottom: 1px solid var(--sebas-border);
      margin-bottom: 2px;
    }

    /* Services section */
    .service-card {
      display: flex;
      align-items: center;
      gap: var(--sebas-space-3);
      padding: var(--sebas-space-3) var(--sebas-space-4);
      background: var(--sebas-surface-2);
      border: 1px solid var(--sebas-border);
      border-radius: var(--sebas-radius-lg);
      margin-bottom: var(--sebas-space-3);
    }
    .service-card .service-info { flex: 1; }
    .service-card .service-info .service-name { font-weight: 600; font-size: 0.9rem; color: var(--sebas-text-bright); }
    .service-card .service-info .service-desc { font-size: 0.78rem; color: var(--sebas-text-dim); margin-top: 2px; }
    .service-card .service-status { display: flex; align-items: center; gap: 6px; font-size: 0.78rem; }
    .service-card .service-status .dot { width: 8px; height: 8px; border-radius: 50%; }
    .service-card .service-status .dot.on { background: var(--sebas-status-done); }
    .service-card .service-status .dot.off { background: var(--sebas-text-faint); }

    /* Network section */
    .network-card {
      display: flex;
      flex-direction: column;
      gap: var(--sebas-space-3);
      padding: var(--sebas-space-3) var(--sebas-space-4);
      background: var(--sebas-surface-2);
      border: 1px solid var(--sebas-border);
      border-radius: var(--sebas-radius-lg);
      margin-bottom: var(--sebas-space-3);
      transition: opacity var(--sebas-dur) var(--sebas-ease);
    }
    .network-switch-row {
      display: flex;
      align-items: center;
      justify-content: space-between;
    }
    .network-switch-row label {
      font-size: 0.85rem;
      color: var(--sebas-text);
      font-weight: 500;
    }
    .network-field {
      display: grid;
      grid-template-columns: 90px 1fr;
      align-items: center;
      gap: var(--sebas-space-2);
    }
    .network-field .field-label {
      font-size: 0.8rem;
      color: var(--sebas-text-dim);
    }
    .network-summary {
      display: flex;
      align-items: center;
      gap: var(--sebas-space-2);
      padding: var(--sebas-space-2) var(--sebas-space-4);
      background: var(--sebas-surface-2);
      border: 1px dashed var(--sebas-border);
      border-radius: var(--sebas-radius-lg);
    }
    .network-summary .summary-label {
      font-size: 0.8rem;
      color: var(--sebas-text-dim);
    }
    .network-summary code {
      font-family: var(--sebas-font-mono);
      font-size: 0.8rem;
      color: var(--sebas-accent);
      background: var(--sebas-accent-soft);
      border-radius: var(--sebas-radius-sm);
      padding: 1px 8px;
    }
    .network-footer {
      display: flex;
      justify-content: flex-end;
      gap: var(--sebas-space-2);
    }

    /* Env vars */
    .env-table {
      width: 100%;
      border-collapse: collapse;
      font-size: 0.82rem;
    }
    .env-table th {
      text-align: left;
      padding: var(--sebas-space-2) var(--sebas-space-3);
      font-size: 0.7rem;
      font-weight: 600;
      text-transform: uppercase;
      letter-spacing: 0.06em;
      color: var(--sebas-text-faint);
      border-bottom: 1px solid var(--sebas-border);
    }
    .env-table td {
      padding: var(--sebas-space-2) var(--sebas-space-3);
      border-bottom: 1px solid var(--sebas-border);
      color: var(--sebas-text);
      font-family: var(--sebas-font-mono);
      font-size: 0.78rem;
    }
    .env-table td:first-child { color: var(--sebas-accent); }
    .env-table td.masked { color: var(--sebas-text-faint); }
    .env-table tr:last-child td { border-bottom: none; }

    /* About section */
    .about-grid {
      display: grid;
      grid-template-columns: 100px 1fr;
      gap: var(--sebas-space-2) var(--sebas-space-4);
      padding: var(--sebas-space-3) var(--sebas-space-4);
      background: var(--sebas-surface-2);
      border: 1px solid var(--sebas-border);
      border-radius: var(--sebas-radius-lg);
    }
    .about-grid .label { font-size: 0.82rem; color: var(--sebas-text-dim); }
    .about-grid .value { font-size: 0.82rem; color: var(--sebas-text); font-family: var(--sebas-font-mono); }
    .about-grid .value.status { display: flex; align-items: center; gap: 6px; }
    .about-grid .value .dot { width: 8px; height: 8px; border-radius: 50%; }
    .about-grid .value .dot.ok { background: var(--sebas-status-done); }

    /* ── wa-dropdown / wa-popup overrides ─────────────────────────── */
    ::part(menu) {
      background: var(--sebas-surface);
      border: 1px solid var(--sebas-border);
      border-radius: var(--sebas-radius-lg);
      box-shadow: var(--sebas-shadow-l);
      padding: 4px;
      min-width: 180px;
      z-index: 60;
    }
    ::part(popup) { z-index: 60; }
    ::part(option) {
      padding: 7px 10px;
      font-size: 0.84rem;
      border-radius: var(--sebas-radius-md);
      color: var(--sebas-text);
      cursor: pointer;
    }
    ::part(option):hover { background: var(--sebas-surface-3); color: var(--sebas-text-bright); }
    ::part(option)[aria-selected='true'],
    ::part(option).active { background: var(--sebas-accent-soft); color: var(--sebas-accent); }
  `

  private async openDirectoryPicker() {
    try {
      const handle = await (window as any).showDirectoryPicker()
      const name = handle.name
      this.addProjectDialogOpen = false
      addProject({
        id: name.toLowerCase().replace(/\s+/g, '-'),
        name,
        path: handle.name,
        sessions: 0,
        hasActive: false,
        hasWaiting: false,
        gitBranch: null,
      })
    } catch (e: any) {
      if (e.name !== 'AbortError' && e.name !== 'SecurityError') {
        this.addProjectDialogOpen = true
      }
    }
  }

  private confirmAddProject() {
    const path = this.manualPath.trim()
    if (!path) return
    const segments = path.split('/')
    const name = segments[segments.length - 1] || 'unnamed'
    this.addProjectDialogOpen = false
    this.manualPath = ''
    addProject({
      id: name.toLowerCase().replace(/\s+/g, '-'),
      name,
      path,
      sessions: 0,
      hasActive: false,
      hasWaiting: false,
      gitBranch: null,
    })
  }

  private closeSettings() {
    // Work around a Web Awesome wa-dialog bug: when open→close happens while
    // the show animation is still running, requestClose awaits a "hide"
    // animation whose animationend never fires, so .open never goes false.
    // Force-close the native <dialog> and drop the stuck animation classes;
    // this lets the component's pending requestClose resolve cleanly.
    const dialog = this.shadowRoot?.querySelector('.settings-dialog') as any
    const inner: HTMLDialogElement | null = dialog?.shadowRoot?.querySelector('dialog') ?? null
    inner?.close()
    inner?.classList.remove('show', 'hide')
    if (dialog) dialog.open = false
    setSettingsOpen(false)
  }

  /**
   * Settings-dialog hide guard. Nested wa-dropdowns (preset/model menus) and
   * the provider editor dialog fire composed `wa-hide` events that bubble up
   * to this host with their `target` retargeted to themselves; those must not
   * close the settings dialog. Only a hide dispatched by the settings dialog
   * host itself has `target === currentTarget`.
   */
  private onSettingsDialogHide(e: any) {
    if (e.target !== e.currentTarget) return
    this.closeSettings()
  }

  // ── Provider editor ────────────────────────────────────────────────

  private openNewProvider() {
    this.editingProviderId = null
    this.formPresetKey = 'deepseek'
    this.formName = ''
    this.formProtocol = 'anthropic'
    this.formBaseUrl = 'https://api.deepseek.com/v1'
    this.formApiKey = ''
    this.formDefaultModel = 'deepseek-v4-flash'
    this.formModelRatings = modelsToRatings(['deepseek-v4-flash', 'deepseek-v4-pro'])
    this.formModelsFilter = ''
    this.providerEditorOpen = true
  }

  private openProviderEditor(p: ProviderState) {
    this.editingProviderId = p.id
    this.formPresetKey = p.preset
    this.formName = p.name
    this.formProtocol = p.protocol
    this.formBaseUrl = p.baseUrl
    this.formApiKey = ''
    this.formDefaultModel = p.defaultModel ?? ''
    this.formModelRatings = { ...p.modelRatings }
    this.formModelsFilter = ''
    this.providerEditorOpen = true
  }

  private deleteProvider(id: string) {
    this.providers = this.providers.filter((p) => p.id !== id)
  }

  private applyPreset(key: string) {
    const preset = PRESETS.find((p) => p.key === key)
    if (!preset) return
    this.formPresetKey = key
    this.formProtocol = preset.protocol
    this.formBaseUrl = preset.baseUrl
    this.formDefaultModel = preset.defaultModel
    this.formModelRatings = modelsToRatings(preset.models)
  }

  private applyCustomPreset() {
    this.formPresetKey = null
    this.formProtocol = 'anthropic'
    this.formBaseUrl = ''
    this.formDefaultModel = ''
    this.formModelRatings = {}
  }

  private closeProviderEditor() {
    this.providerEditorOpen = false
    this.presetMenuOpen = false
    this.modelsMenuOpen = false
    this.formModelsFilter = ''
  }

  /**
   * Provider-dialog hide guard. The preset/model wa-dropdowns are nested
   * inside this dialog and fire composed `wa-hide` events that bubble up
   * through the shadow root; their `target` is retargeted to the dropdown
   * host, so they must NOT close the editor. Only a hide dispatched by the
   * dialog host itself (Esc, backdrop, or programmatic close) has
   * `target === currentTarget` and is honored.
   */
  private onProviderDialogHide(e: any) {
    if (e.target !== e.currentTarget) return
    this.closeProviderEditor()
  }

  /** Small hint under the model editor that changes with the chosen preset. */
  private presetHint() {
    const preset = PRESETS.find((p) => p.key === this.formPresetKey)
    if (!preset) {
      return html`<span class="chip-demo">Custom preset — add any model id.</span>`
    }
    return html`<span class="chip-demo">${preset.label} preset ships with ${preset.models.join(' · ')}.</span>`
  }

  private saveProvider() {
    const fallbackId = this.formName.trim().toLowerCase().replace(/[^a-z0-9-]/g, '-') || `provider-${Date.now()}`
    const id = this.editingProviderId ?? fallbackId
    const name = this.formName.trim() || (this.formPresetKey ?? id)
    const modelIds = Object.keys(this.formModelRatings).filter((m) => m.trim().length > 0)
    const defaultModel = this.formDefaultModel.trim() || (modelIds.length > 0 ? modelIds[0]! : null)
    const provider: ProviderState = {
      id,
      name,
      preset: this.formPresetKey,
      protocol: this.formProtocol.trim() || 'anthropic',
      baseUrl: this.formBaseUrl.trim(),
      apiKeyConfigured: this.formApiKey.trim().length > 0,
      defaultModel,
      modelRatings: { ...this.formModelRatings },
    }
    if (this.editingProviderId) {
      this.providers = this.providers.map((p) => (p.id === this.editingProviderId ? provider : p))
    } else {
      this.providers = [...this.providers, provider]
    }
    this.closeProviderEditor()
  }

  // Model chip editor helpers ----------------------------------------
  private addEditedModel(m: string, rating: ModelRating | null = null) {
    const id = m.trim()
    if (!id || id in this.formModelRatings) return
    this.formModelRatings = { ...this.formModelRatings, [id]: rating }
    this.formModelsFilter = ''
  }

  private removeEditedModel(m: string) {
    const next = { ...this.formModelRatings }
    delete next[m]
    this.formModelRatings = next
    if (this.formDefaultModel === m) this.formDefaultModel = ''
  }

  private cycleEditedModelRating(m: string) {
    const order: (ModelRating | null)[] = [null, 'T0', 'T1', 'T2']
    const cur = this.formModelRatings[m] ?? null
    const next = order[(order.indexOf(cur) + 1) % order.length]!
    this.formModelRatings = { ...this.formModelRatings, [m]: next }
  }

  // Network proxy draft helpers --------------------------------------
  private saveProxy() {
    this.proxyEnabled = this.draftProxyEnabled
    this.proxyMode = this.draftProxyMode
    this.proxyHost = this.draftProxyHost
    this.proxyPort = this.draftProxyPort
    this.proxyUsername = this.draftProxyUsername
    this.proxyPassword = this.draftProxyPassword
  }

  private resetProxyDraft() {
    this.draftProxyEnabled = this.proxyEnabled
    this.draftProxyMode = this.proxyMode
    this.draftProxyHost = this.proxyHost
    this.draftProxyPort = this.proxyPort
    this.draftProxyUsername = this.proxyUsername
    this.draftProxyPassword = this.proxyPassword
  }

  private renderSettingsContent() {
    switch (this.settingsSection) {
      case 'ui':
        return html`
          <h3>UI</h3>
          <p class="desc">Appearance and language preferences.</p>

          <div class="pref-row">
            <label>Theme</label>
            <wa-button variant=${this.theme === 'dark' ? 'brand' : 'neutral'} appearance="filled" size="xs" @click=${() => setTheme('dark')}>Dark</wa-button>
            <wa-button variant=${this.theme === 'light' ? 'brand' : 'neutral'} appearance="filled" size="xs" @click=${() => setTheme('light')}>Light</wa-button>
          </div>

          <div class="pref-row">
            <label>Language</label>
            <wa-button variant=${this.language === 'zh' ? 'brand' : 'neutral'} appearance="filled" size="xs" @click=${() => setLanguage('zh')}>中文</wa-button>
            <wa-button variant=${this.language === 'en' ? 'brand' : 'neutral'} appearance="filled" size="xs" @click=${() => setLanguage('en')}>English</wa-button>
          </div>
        `

      case 'models':
        return html`
          <h3>Models</h3>
          <p class="desc">Configure providers and models. Presets pre-fill addresses and models; custom providers need all fields.</p>

          <div class="provider-list">
            ${this.providers.map((p) => html`
              <div class="provider-row">
                <span class="provider-row-name">${p.name}</span>
                <span class="provider-row-preset">${p.preset ? presetLabel(p.preset) : 'custom'}</span>
                <span class="provider-row-count">${Object.keys(p.modelRatings).length} models</span>
                <span class="provider-row-actions">
                  <button class="row-action" title="Edit ${p.name}" aria-label="Edit ${p.name}" @click=${() => this.openProviderEditor(p)}>${icon('pencil', 13)}</button>
                  <button class="row-action danger" title="Delete ${p.name}" aria-label="Delete ${p.name}" @click=${() => this.deleteProvider(p.id)}>${icon('trash', 13)}</button>
                </span>
              </div>
            `)}
          </div>

          <div class="add-provider-card" @click=${this.openNewProvider}>
            <wa-icon name="plus" style="font-size:12px;"></wa-icon>
            <span>Add provider</span>
          </div>
        `

      case 'services':
        return html`
          <h3>Services</h3>
          <p class="desc">Manage background services that run alongside sebas.</p>

          <div class="service-card">
            <div class="service-info">
              <div class="service-name">Router</div>
              <div class="service-desc">ACP router — listens on 127.0.0.1:8787</div>
            </div>
            <div class="service-status">
              <span class="dot on"></span> Running
              <wa-button size="xs" appearance="outlined" style="color:var(--sebas-status-failed);">Stop</wa-button>
              <wa-button size="xs" appearance="plain">Restart</wa-button>
            </div>
          </div>

          <div class="service-card">
            <div class="service-info">
              <div class="service-name">Feishu Bot</div>
              <div class="service-desc">飞书消息推送 — 连接 Feishu 应用</div>
            </div>
            <div class="service-status">
              <span class="dot off"></span> Stopped
              <wa-button size="xs" variant="brand">Start</wa-button>
            </div>
          </div>

          <div class="service-card">
            <div class="service-info">
              <div class="service-name">Health Check</div>
              <div class="service-desc">Periodic keep-alive & connectivity check</div>
            </div>
            <div class="service-status">
              <span class="dot on"></span> Running
              <wa-button size="xs" appearance="outlined" style="color:var(--sebas-status-failed);">Stop</wa-button>
            </div>
          </div>
        `

      case 'network':
        return html`
          <h3>Network</h3>
          <p class="desc">Configure an outbound proxy for all sebas connections (provider APIs, router, feishu).</p>

          <div class="network-card">
            <div class="network-switch-row">
              <label>Enable proxy</label>
              <wa-switch ?checked=${this.draftProxyEnabled} @change=${(e: any) => (this.draftProxyEnabled = e.target.checked)}></wa-switch>
            </div>
          </div>

          <div class="network-card" style=${this.draftProxyEnabled ? '' : 'opacity:0.5;pointer-events:none;'}>
            <div class="network-field">
              <span class="field-label">Type</span>
              <wa-select size="sm" value=${this.draftProxyMode} style="--width:170px;" @change=${(e: any) => (this.draftProxyMode = e.target.value)}>
                <wa-option value="http">http</wa-option>
                <wa-option value="https">https</wa-option>
                <wa-option value="socks">socks</wa-option>
                <wa-option value="socks5h">socks5h</wa-option>
              </wa-select>
            </div>
            <div class="network-field">
              <span class="field-label">Host</span>
              <wa-input size="sm" value=${this.draftProxyHost} style="--width:100%;" placeholder="127.0.0.1" @wa-input=${(e: any) => (this.draftProxyHost = e.target.value)}></wa-input>
            </div>
            <div class="network-field">
              <span class="field-label">Port</span>
              <wa-input size="sm" value=${this.draftProxyPort} style="--width:120px;" placeholder="7890" @wa-input=${(e: any) => (this.draftProxyPort = e.target.value)}></wa-input>
            </div>
            <div class="network-field">
              <span class="field-label">Username</span>
              <wa-input size="sm" value=${this.draftProxyUsername} style="--width:100%;" placeholder="(optional)" @wa-input=${(e: any) => (this.draftProxyUsername = e.target.value)}></wa-input>
            </div>
            <div class="network-field">
              <span class="field-label">Password</span>
              <wa-input size="sm" type="password" value=${this.draftProxyPassword} style="--width:100%;" placeholder="(optional)" @wa-input=${(e: any) => (this.draftProxyPassword = e.target.value)}></wa-input>
            </div>
          </div>

          <div class="network-footer">
            <wa-button size="sm" variant="brand" @click=${this.saveProxy}>Save</wa-button>
            <wa-button size="sm" appearance="plain" @click=${this.resetProxyDraft}>Cancel</wa-button>
          </div>

          <div class="network-summary">
            <span class="summary-label">Proxy URL</span>
            <code>${this.proxyEnabled ? html`<span>${this.proxyMode}://${this.proxyHost}:${this.proxyPort}${this.proxyUsername ? html` · auth enabled` : ''}</span>` : 'off'}</code>
          </div>
        `

      case 'env':
        return html`
          <h3>Environment Variables</h3>
          <p class="desc">Current environment configuration. Editable via your shell profile.</p>

          <table class="env-table">
            <thead><tr><th>Variable</th><th>Value</th></tr></thead>
            <tbody>
              <tr><td>SEBAS_HOME</td><td>/home/user/.config/sebas</td></tr>
              <tr><td>SEBAS_ROUTER_ADDR</td><td>127.0.0.1:8787</td></tr>
              <tr><td>SEBAS_WEBUI_PORT</td><td>9797</td></tr>
              <tr><td>SEBAS_LOG_LEVEL</td><td>info</td></tr>
              <tr><td>SEBAS_DEFAULT_PROVIDER</td><td>alpha</td></tr>
              <tr><td>SEBAS_CONTROL_SECRET</td><td class="masked">··· (set)</td></tr>
              <tr><td>SEBAS_DATA_DIR</td><td>/home/user/.local/share/sebas</td></tr>
            </tbody>
          </table>
        `

      case 'about':
        return html`
          <h3>About</h3>
          <p class="desc">sebas — local agent router</p>

          <div class="about-grid">
            <span class="label">Version</span><span class="value">0.1.0</span>
            <span class="label">Build</span><span class="value">8da5e25 (2026-08-31)</span>
            <span class="label">Node</span><span class="value">cupen-dev</span>
            <span class="label">Core Status</span><span class="value status"><span class="dot ok"></span> connected</span>
            <span class="label">Uptime</span><span class="value">2h 14m</span>
            <span class="label">Runtime</span><span class="value">Rust 1.82 / Node 22</span>
            <span class="label">License</span><span class="value">MIT</span>
          </div>
        `
    }
  }

  private renderProjectsTree() {
    return this.projects.map((p) => {
      const projectSessions = getProjectSessions(p.id)
      const expanded = this.expandedProjects[p.id] ?? false
      const isCurrent = p.id === this.activeProjectId
      const selectProject = () => {
        setActiveProjectId(p.id)
        this.expandedProjects = { ...this.expandedProjects, [p.id]: true }
        // On mobile, jump straight to the chat panel after picking a project
        if (this.isMobile) this.mobileTab = 'chat'
      }
      const newSession = (e: Event) => {
        e.stopPropagation()
        const id = `sess-${Date.now()}`
        addSessionToProject(p.id, {
          id,
          label: `Session ${projectSessions.length + 1}`,
          status: 'active',
          turns: 0,
          lastActive: 'just now',
        })
        // addSessionToProject already sets active session + active project
        this.expandedProjects = { ...this.expandedProjects, [p.id]: true }
        if (this.isMobile) this.mobileTab = 'chat'
      }
      return html`
        <div>
          <div
            class="project-item"
            aria-current=${isCurrent ? 'true' : 'false'}
            @click=${() => {
              if (isCurrent) {
                this.expandedProjects = { ...this.expandedProjects, [p.id]: !expanded }
              } else {
                selectProject()
              }
            }}
          >
            <span class="chevron ${expanded ? 'open' : ''}">▶</span>
            ${p.hasActive ? html`<span class="active-dot"></span>` : nothing}
            <span class="project-name">${p.name}</span>
            ${p.gitBranch ? html`<span class="project-branch">${p.gitBranch}</span>` : nothing}
            <span class="project-count">${projectSessions.length}</span>
            <button
              class="project-add-btn"
              title="New session in ${p.name}"
              aria-label="New session in ${p.name}"
              @click=${newSession}
            >+</button>
          </div>
          ${expanded
            ? projectSessions.map(
                (s: SessionInfo) => html`
                  <div
                    class="session-item"
                    aria-current=${s.id === this.activeSessionId ? 'true' : 'false'}
                    @click=${() => {
                      openSession(p.id, s.id)
                      if (this.isMobile) this.mobileTab = 'chat'
                    }}
                  >
                    <span class="session-dot ${s.status}"></span>
                    <span class="session-name">${s.label}</span>
                    <button
                      class="session-archive-btn"
                      title="Archive this session"
                      aria-label="Archive ${s.label}"
                      @click=${(e: Event) => {
                        e.stopPropagation()
                        archiveSession(p.id, s.id)
                      }}
                    >${icon('inbox', 11)}</button>
                  </div>
                `
              )
            : nothing}
        </div>
      `
    })
  }

  private renderHistoryTree() {
    if (this.archivedSessions.length === 0) return nothing
    return html`
      <div class="history-section">
        <div
          class="history-head"
          @click=${() => (this.historyExpanded = !this.historyExpanded)}
        >
          <span class="chevron ${this.historyExpanded ? 'open' : ''}">▶</span>
          <span>History</span>
          <span class="history-count">${this.archivedSessions.length}</span>
        </div>
        ${this.historyExpanded
          ? this.archivedSessions.map(
              (a) => html`
                <div
                  class="session-item archived"
                  @click=${() => {
                    restoreSession(a.id)
                    if (this.isMobile) this.mobileTab = 'chat'
                  }}
                >
                  <span class="session-dot done"></span>
                  <span class="session-name">${a.label}</span>
                  <span class="archive-meta">${a.projectName}</span>
                </div>
              `
            )
          : nothing}
      </div>
    `
  }

  render() {
    return html`
      <nav aria-label="Primary">
        <a class="brand" href="/" aria-label="sebas console home">
          <span class="mark" aria-hidden="true">❯</span>
          <span class="name">sebas<small>preview</small></span>
        </a>

        <div class="section-label">
          <span>Projects</span>
          <button class="add-btn" @click=${this.openDirectoryPicker} aria-label="Add project" title="Add project">+</button>
        </div>

        ${this.renderProjectsTree()}

        ${this.renderHistoryTree()}

        <div class="sidebar-footer">
          <button class="settings-btn" @click=${() => setSettingsOpen(true)}>
            ${icon('settings', 16)} <span class="settings-label">Settings</span>
          </button>
        </div>
      </nav>

      <!-- Mobile: tab bar switching Projects / Chat -->
      ${this.isMobile ? html`
        <div class="mobile-tabs">
          <button class="mobile-tab" aria-current=${this.mobileTab === 'projects' ? 'true' : 'false'} @click=${() => (this.mobileTab = 'projects')}>
            ${icon('folder', 14)} Projects
          </button>
          <button class="mobile-tab" aria-current=${this.mobileTab === 'chat' ? 'true' : 'false'} @click=${() => (this.mobileTab = 'chat')}>
            ${icon('message', 14)} Chat
          </button>
        </div>

        <!-- Mobile: projects panel (only when tab=projects) -->
        <div class="mobile-projects-panel ${this.mobileTab === 'projects' ? 'panel-active' : ''}">
          <div class="mobile-panel-head">
            <span>Projects</span>
            <button class="add-btn" @click=${this.openDirectoryPicker} aria-label="Add project" title="Add project">+</button>
          </div>
          ${this.renderProjectsTree()}
          ${this.renderHistoryTree()}
        </div>
      ` : nothing}

      <main class=${this.isMobile && this.mobileTab === 'projects' ? 'mobile-hidden' : ''}>
        <div class="outlet"><sebas-preview-workbench></sebas-preview-workbench></div>
      </main>

      <!-- Settings: centered modal dialog -->
      <wa-dialog class="settings-dialog" .open=${this.settingsOpen} @wa-hide=${this.onSettingsDialogHide} no-header>
        <div class="settings-layout">
          <button class="settings-close" @click=${this.closeSettings} aria-label="Close settings">✕</button>
          <nav class="settings-nav">
            <div class="nav-section-title">Preferences</div>
            <button class="nav-item" aria-current=${this.settingsSection === 'ui' ? 'true' : 'false'} @click=${() => (this.settingsSection = 'ui')}>
              ${icon('settings', 14)} UI
            </button>
            <button class="nav-item" aria-current=${this.settingsSection === 'models' ? 'true' : 'false'} @click=${() => (this.settingsSection = 'models')}>
              ${icon('zap', 14)} Models
            </button>
            <button class="nav-item" aria-current=${this.settingsSection === 'services' ? 'true' : 'false'} @click=${() => (this.settingsSection = 'services')}>
              ${icon('shield', 14)} Services
            </button>
            <button class="nav-item" aria-current=${this.settingsSection === 'network' ? 'true' : 'false'} @click=${() => (this.settingsSection = 'network')}>
              ${icon('globe', 14)} Network
            </button>
            <button class="nav-item" aria-current=${this.settingsSection === 'env' ? 'true' : 'false'} @click=${() => (this.settingsSection = 'env')}>
              ${icon('inbox', 14)} Environment
            </button>
            <div class="nav-section-title" style="margin-top:var(--sebas-space-1);">Info</div>
            <button class="nav-item" aria-current=${this.settingsSection === 'about' ? 'true' : 'false'} @click=${() => (this.settingsSection = 'about')}>
              ${icon('about', 14)} About
            </button>
          </nav>
          <div class="settings-content">
            ${this.renderSettingsContent()}
          </div>
        </div>
      </wa-dialog>

      <!-- Provider editor dialog -->
      <wa-dialog class="provider-dialog" label=${this.editingProviderId ? 'Edit Provider' : 'Add Provider'} .open=${this.providerEditorOpen} @wa-hide=${this.onProviderDialogHide}>
        <div class="wa-stack" style="gap:var(--sebas-space-4);">

          <div class="provider-form-field">
            <span class="form-label">Preset</span>
            <wa-dropdown
              class="preset-dropdown"
              placement="bottom-start"
              .open=${this.presetMenuOpen}
              @wa-show=${() => (this.presetMenuOpen = true)}
              @wa-hide=${() => (this.presetMenuOpen = false)}
              @wa-select=${(e: any) => {
                const key: string | null = e.detail?.item?.value ?? null
                if (key) this.applyPreset(key)
                else if (e.detail?.item?.value === undefined || e.detail?.item?.value === 'custom') this.applyCustomPreset()
              }}
            >
              <wa-button slot="trigger" size="sm" appearance="outlined" ?disabled=${this.presetMenuOpen}>
                <span class="preset-trigger-label">${this.formPresetKey ? presetLabel(this.formPresetKey) : 'Custom preset'}</span>
                <span class="preset-trigger-chev">▾</span>
              </wa-button>
              ${PRESETS.map((ps) => html`
                <wa-dropdown-item value=${ps.key} class="preset-option">${ps.label}</wa-dropdown-item>
              `)}
              <wa-dropdown-item value="custom" class="preset-option">Custom</wa-dropdown-item>
            </wa-dropdown>
          </div>

          <div class="provider-form-field">
            <span class="form-label">Name</span>
            <wa-input size="sm" placeholder="e.g. alpha" .value=${this.formName} @wa-input=${(e: any) => (this.formName = e.target.value)}></wa-input>
          </div>

          <div class="provider-form-grid">
            <div class="provider-form-field">
              <span class="form-label">Protocol</span>
              <wa-select size="sm" value=${this.formProtocol} style="--width:100%;" @change=${(e: any) => (this.formProtocol = e.target.value)}>
                <wa-option value="anthropic">anthropic</wa-option>
                <wa-option value="openai">openai</wa-option>
              </wa-select>
            </div>
            <div class="provider-form-field">
              <span class="form-label">API Key</span>
              <wa-input size="sm" type="password" placeholder="sk-…" .value=${this.formApiKey} @wa-input=${(e: any) => (this.formApiKey = e.target.value)}></wa-input>
            </div>
          </div>

          <div class="provider-form-field">
            <span class="form-label">Base URL</span>
            <wa-input size="sm" placeholder="https://api.example.com/v1" .value=${this.formBaseUrl} @wa-input=${(e: any) => (this.formBaseUrl = e.target.value)}></wa-input>
          </div>

          <div class="provider-form-field">
            <span class="form-label">Default model</span>
            <wa-input size="sm" placeholder="model-id" .value=${this.formDefaultModel} @wa-input=${(e: any) => (this.formDefaultModel = e.target.value)}></wa-input>
          </div>

          <div class="provider-form-field">
            <span class="form-label">Models</span>
            <div class="model-editor">
              <div class="model-editor-row">
                ${Object.keys(this.formModelRatings).map((m) => {
                  const rating = this.formModelRatings[m] ?? null
                  return html`
                    <span
                      class="model-chip rating-${rating === null ? 'null' : rating}"
                      title=${rating === null ? 'No rating — click to set T0/T1/T2' : `Rating ${rating} — click to cycle`}
                      @click=${() => this.cycleEditedModelRating(m)}
                    >
                      <span class="model-chip-name">${m}</span>
                      <span class="model-chip-rating">${rating ?? '·'}</span>
                      <button
                        class="model-chip-x"
                        title="Remove ${m}"
                        aria-label="Remove ${m}"
                        @click=${(e: Event) => {
                          e.stopPropagation()
                          this.removeEditedModel(m)
                        }}
                      >✕</button>
                    </span>
                  `
                })}

                <wa-dropdown
                  class="model-dropdown"
                  placement="bottom-start"
                  .open=${this.modelsMenuOpen}
                  @wa-show=${() => (this.modelsMenuOpen = true)}
                  @wa-hide=${() => { this.modelsMenuOpen = false; this.formModelsFilter = '' }}
                  @wa-select=${(e: any) => this.addEditedModel(e.detail?.item?.value ?? '')}
                >
                  <button slot="trigger" class="model-add-btn" title="Add a model">
                    <span style="font-size:12px;line-height:1;">+</span> Add model
                  </button>
                  <div class="model-menu-search" @click=${(e: Event) => e.stopPropagation()}>
                    <wa-input
                      size="sm"
                      placeholder="Type a model id…"
                      .value=${this.formModelsFilter}
                      autofocus
                      @wa-input=${(e: any) => (this.formModelsFilter = e.target.value)}
                    ></wa-input>
                  </div>
                  ${(this.formModelsFilter
                    ? this.allModelOptions.filter((m) => this.formModelsFilter && m.toLowerCase().includes(this.formModelsFilter.toLowerCase()))
                    : this.allModelOptions
                  ).map((m) => html`
                    <wa-dropdown-item
                      value=${m}
                      class="model-option"
                      ?disabled=${m in this.formModelRatings}
                    >${m}${m in this.formModelRatings ? ' · added' : ''}</wa-dropdown-item>
                  `)}
                </wa-dropdown>
              </div>
              <div class="model-editor-hint">
                Click a model to cycle its rating (T0 · T1 · T2 · unrated). ${this.presetHint()}
              </div>
            </div>
          </div>

        </div>
        <wa-button slot="footer" variant="brand" @click=${this.saveProvider}>Save</wa-button>
        <wa-button slot="footer" appearance="plain" @click=${() => (this.providerEditorOpen = false)}>Cancel</wa-button>
      </wa-dialog>

      <!-- Add project: directory picker dialog -->
      <wa-dialog label="Add project" style="--width: 440px;" .open=${this.addProjectDialogOpen} @wa-hide=${() => (this.addProjectDialogOpen = false)}>
        <div class="wa-stack" style="gap:var(--sebas-space-4);">
          <p style="font-size:0.85rem;color:var(--sebas-text);margin:0;">Choose a directory to add as a project:</p>
          <wa-button variant="brand" @click=${this.openDirectoryPicker} style="width:100%;">
            <wa-icon slot="start" name="folder-open" style="font-size:14px;"></wa-icon>
            Browse Directories…
          </wa-button>
          <p style="font-size:0.8rem;color:var(--sebas-text-faint);margin:0;text-align:center;">or</p>
          <wa-input label="Project path" placeholder="/home/user/work/repo" .value=${this.manualPath} @wa-input=${(e: any) => (this.manualPath = e.target.value)} autofocus>
            <wa-icon slot="start" name="folder" aria-hidden="true"></wa-icon>
          </wa-input>
          <wa-checkbox checked>Auto-detect git branch</wa-checkbox>
        </div>
        <wa-button slot="footer" variant="brand" @click=${this.confirmAddProject} ?disabled=${!this.manualPath.trim()}>Add project</wa-button>
        <wa-button slot="footer" appearance="plain" @click=${() => (this.addProjectDialogOpen = false)}>Cancel</wa-button>
      </wa-dialog>
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-preview-app': SebasPreviewApp
  }
}