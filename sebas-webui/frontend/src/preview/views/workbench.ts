/**
 * Preview workbench view: full interactive prototype.
 * Uses shared project state from preview/shared-state.ts.
 * Sessions are per-project. Model selector floats at textarea bottom-right.
 */

import { LitElement, css, html, nothing } from 'lit'
import { customElement, state } from 'lit/decorators.js'
import { unsafeHTML } from 'lit/directives/unsafe-html.js'
import { viewStyles } from '../../styles/shared.js'
import { icon } from '../../components/icons.js'
import {
  getProjects,
  getActiveProjectId,
  getActiveSessionId,
  subscribe,
  type ProjectInfo,
} from '../shared-state.js'

interface TurnItem {
  role: 'user' | 'assistant' | 'tool' | 'thought' | 'seam'
  author?: string
  time?: string
  content?: string
  count?: string
  label?: string
  /** Short summary used when folding (e.g. tool name, thought label). */
  summary?: string
}

@customElement('sebas-preview-workbench')
export class SebasPreviewWorkbench extends LitElement {
  @state() private activeProject: ProjectInfo | undefined
  @state() private activeProjectId = getActiveProjectId()
  @state() private activeSession = getActiveSessionId()
  @state() private selectedModelKey = 'direct/claude-sonnet-4'
  @state() private messageText = ''

  // Composer mode menu
  @state() private modeMenuOpen = false
  @state() private mode: 'ask' | 'auto' | 'plan' | 'full' = 'full'
  @state() private attachmentCount = 2

  // Turn folding: outer "折腾中" group is collapsed by default.
  // Each tool / thought inside it has its own per-block collapsed flag.
  @state() private expandedGroups: Record<string, boolean> = {}
  @state() private expandedBlocks: Record<string, boolean> = {}

  private modeLabels: Record<typeof this.mode, { label: string; title: string; short: string }> = {
    ask: { label: 'Ask before changes', title: 'Ask before file changes.', short: 'Ask' },
    auto: { label: 'Edit automatically', title: 'Edit files automatically.', short: 'Auto' },
    plan: { label: 'Plan mode', title: 'Plan before editing.', short: 'Plan' },
    full: { label: 'Full access', title: 'Run with fewer confirmations.', short: 'Full' },
  }

  private handleDocClick = (e: MouseEvent) => {
    if (!this.modeMenuOpen) return
    const path = e.composedPath()
    for (const node of path) {
      if (node instanceof HTMLElement && node.classList?.contains('mode-menu-pop')) return
      if (node instanceof HTMLElement && node.classList?.contains('mode-trigger')) return
    }
    this.modeMenuOpen = false
  }

  private handleDocKey = (e: KeyboardEvent) => {
    if (this.modeMenuOpen && e.key === 'Escape') {
      this.modeMenuOpen = false
      e.preventDefault()
    }
  }

  // Turns per session (kept local to the workbench)
  private turnsBySession: Record<string, TurnItem[]> = {
    'sess-1': [
      { role: 'user', author: 'you', time: '14:02', content: '重构 webui 的状态色阶' },
      {
        role: 'thought',
        author: 'claude',
        time: '14:02',
        summary: '思考色阶重构方案',
        content:
          '<p>先看一下当前 <code>tokens.css</code> 里 status 色阶是怎么分的。需要保留 6 个状态的语义化命名，同时让 3 级色调之间的对比度更稳定。</p>',
      },
      {
        role: 'tool',
        author: 'claude',
        time: '14:02',
        summary: 'read style.css',
        content:
          '<div class="tool-call"><span class="file">▸ read  style.css</span><span style="margin-left: auto; color: var(--sebas-text-faint);">+120 −0</span></div>',
      },
      {
        role: 'assistant',
        author: 'claude',
        time: '14:02',
        content:
          '<p>以下是改动方案。我看了 <code>tokens.css</code> 里的 status 色阶，目前 6 个状态各有 3 级，结构整齐但有几处偏差：</p><ul><li>queued 和 done 的中间档位对比度偏低</li><li>working 与 accent 颜色过近，容易混淆</li></ul>',
      },
      {
        role: 'tool',
        author: 'claude',
        time: '14:03',
        summary: 'edit style.css',
        content:
          '<div class="tool-call"><span class="file">▸ edit  style.css</span><span style="margin-left: auto; color: var(--sebas-text-faint);">+42 −18</span></div>',
      },
      { role: 'user', author: 'you', time: '14:31', content: '继续' },
      { role: 'seam', count: '3', label: 'while you were away' },
      {
        role: 'thought',
        author: 'claude',
        time: '15:12',
        summary: '思考 seam 色阶的语义',
        content: '<p>需要为 seam 单独新增一个色阶，不应该复用 status 的任意档位。</p>',
      },
      {
        role: 'assistant',
        author: 'claude',
        time: '15:12',
        content:
          '<p>好的，继续。关于 <code>--seam</code> 色阶的实现：</p><p>需要在 <code>tokens.css</code> 里新增一个 seam 色阶，位于 accent 和 status 之间，专门用于时间边界线。</p>',
      },
      {
        role: 'tool',
        author: 'claude',
        time: '15:14',
        summary: 'edit tokens.css',
        content:
          '<div class="tool-call"><span class="file">▸ edit  tokens.css</span><span style="margin-left: auto; color: var(--sebas-text-faint);">+6 −0</span></div>',
      },
    ],
    'sess-2': [
      { role: 'user', author: 'you', time: '12:30', content: '检查一下 beads 的 issue 列表' },
      { role: 'assistant', author: 'claude', time: '12:30', content: '<p>正在查询 beads 的 issue 列表...</p>' },
    ],
    'sess-b1': [
      { role: 'user', author: 'you', time: '11:20', content: 'beads 项目有什么待办事项？' },
      { role: 'assistant', author: 'claude', time: '11:20', content: '<p>Beans 项目当前有 3 个开放 issue，我来列出...</p>' },
    ],
  }

  // Combined provider/model options
  private modelOptions = [
    { value: 'direct/claude-sonnet-4', label: 'direct / claude-sonnet-4' },
    { value: 'direct/claude-opus-4', label: 'direct / claude-opus-4' },
    { value: 'direct/deepseek-reasoner', label: 'direct / deepseek-reasoner' },
    { value: 'direct/deepseek-chat', label: 'direct / deepseek-chat' },
    { value: 'gateway/claude-sonnet-4', label: 'gateway / claude-sonnet-4' },
    { value: 'gateway/claude-opus-4', label: 'gateway / claude-opus-4' },
    { value: 'off/—', label: 'off / —' },
  ]

  private unsub?: () => void

  connectedCallback(): void {
    super.connectedCallback()
    this.unsub = subscribe(() => {
      this.activeProjectId = getActiveProjectId()
      this.activeProject = getProjects().find((p) => p.id === this.activeProjectId)
      this.activeSession = getActiveSessionId()
    })
    this.activeProject = getProjects().find((p) => p.id === this.activeProjectId)
    document.addEventListener('mousedown', this.handleDocClick)
    document.addEventListener('keydown', this.handleDocKey)
  }

  disconnectedCallback(): void {
    this.unsub?.()
    document.removeEventListener('mousedown', this.handleDocClick)
    document.removeEventListener('keydown', this.handleDocKey)
    super.disconnectedCallback()
  }

  get activeTurns(): TurnItem[] {
    return this.turnsBySession[this.activeSession] ?? []
  }

  static styles = [
    viewStyles,
    css`
      :host { display: flex; flex: 1; flex-direction: column; min-height: 0; min-width: 0; }

      .project-header {
        display: flex; align-items: center; gap: var(--sebas-space-3);
        padding: var(--sebas-space-3) var(--sebas-space-5);
        border-bottom: 1px solid var(--sebas-border);
        background: var(--sebas-surface); flex-wrap: wrap;
      }
      .project-header .path { font-weight: 600; font-size: 0.95rem; color: var(--sebas-text-bright); }
      .project-header .branch { font-family: var(--sebas-font-mono); font-size: 0.75rem; color: var(--sebas-accent); background: var(--sebas-accent-soft); border-radius: var(--sebas-radius-full); padding: 1px 10px; }
      .project-header .project-meta { margin-left: auto; display: flex; align-items: center; gap: var(--sebas-space-3); font-size: 0.8rem; color: var(--sebas-text-dim); font-variant-numeric: tabular-nums; }
      .project-header .project-meta .meta-item { display: flex; align-items: center; gap: 5px; }

      /* Chat bubbles: agent bubbles left, user bubbles right */
      .turn-stream-area { flex: 1; min-height: 0; overflow-y: auto; display: flex; flex-direction: column; }
      .turn-stream { padding: var(--sebas-space-5); display: flex; flex-direction: column; gap: var(--sebas-space-3); flex: 1; }
      .turn-block {
        display: flex;
        gap: 10px;
        align-items: flex-start;
        max-width: 100%;
      }
      .turn-block.is-user { flex-direction: row-reverse; }
      .turn-block .avatar {
        width: 26px;
        height: 26px;
        flex: 0 0 26px;
        border-radius: 50%;
        display: grid;
        place-items: center;
        font-size: 0.72rem;
        font-weight: 700;
        background: var(--sebas-surface-2);
        border: 1px solid var(--sebas-border);
        color: var(--sebas-text-dim);
        margin-top: 2px;
      }
      .turn-block .avatar.assistant {
        background: linear-gradient(135deg, var(--sebas-accent-strong), #4338ca);
        color: var(--sebas-accent-ink);
        border-color: transparent;
      }
      .turn-block .avatar.user {
        background: var(--sebas-accent-soft);
        color: var(--sebas-accent);
        border-color: transparent;
      }
      .turn-block .bubble {
        flex: 1;
        min-width: 0;
        max-width: min(680px, calc(100% - 60px));
        padding: 9px 14px;
        background: var(--sebas-surface);
        border: 1px solid var(--sebas-border);
        border-radius: 14px;
        border-top-left-radius: 4px;
        position: relative;
      }
      .turn-block.is-user .bubble {
        background: var(--sebas-accent-soft);
        border-color: var(--sebas-accent-border);
        color: var(--sebas-text);
        border-top-left-radius: 14px;
        border-top-right-radius: 4px;
      }
      .turn-block .meta {
        display: flex;
        align-items: center;
        gap: var(--sebas-space-2);
        font-size: 0.7rem;
        color: var(--sebas-text-faint);
        margin-bottom: 4px;
      }
      .turn-block .meta .author { font-weight: 600; color: var(--sebas-text-dim); }
      .turn-block .meta .author.you { color: var(--sebas-accent); }
      .turn-block .meta .time { margin-left: auto; font-variant-numeric: tabular-nums; }
      .turn-block .body {
        font-size: 0.875rem;
        line-height: 1.65;
        color: var(--sebas-text);
      }
      .turn-block .body p { margin: 0 0 0.5em; }
      .turn-block .body p:last-child { margin-bottom: 0; }
      .turn-block .body .tool-call {
        display: flex; align-items: center; gap: var(--sebas-space-2);
        padding: var(--sebas-space-2) var(--sebas-space-3);
        background: var(--sebas-surface-2);
        border-radius: var(--sebas-radius-md);
        font-family: var(--sebas-font-mono);
        font-size: 0.78rem;
        color: var(--sebas-text-dim);
        margin: var(--sebas-space-2) 0;
      }
      .turn-block .body .tool-call .file { color: var(--sebas-accent); }

      .seam {
        display: flex;
        align-items: center;
        gap: var(--sebas-space-3);
        color: var(--sebas-text-faint);
        font-size: 0.72rem;
        text-transform: uppercase;
        letter-spacing: 0.06em;
        margin: var(--sebas-space-2) 0;
      }
      .seam::before, .seam::after { content: ''; flex: 1; height: 1px; background: var(--sebas-border); }
      .seam .count { color: var(--sebas-accent); font-weight: 600; }

      /* ─── Composer: floating model selector at textarea bottom-right ─── */
      /* ─── Composer (Cursor-style) ─── */
      .composer-area {
        border-top: 1px solid var(--sebas-border);
        background: var(--sebas-bg);
        flex-shrink: 0;
        padding: 0 var(--sebas-space-5) var(--sebas-space-4);
      }
      .composer-shell {
        background: var(--sebas-surface);
        border: 1px solid var(--sebas-border);
        border-radius: 18px;
        padding: var(--sebas-space-3);
        display: flex;
        flex-direction: column;
        gap: var(--sebas-space-2);
      }
      .composer-hint {
        font-size: 0.78rem;
        color: var(--sebas-text-faint);
        padding: 0 4px;
        user-select: none;
      }
      .composer-input wa-textarea {
        width: 100%;
        --wa-textarea-height: auto;
      }
      .composer-input wa-textarea::part(form-control) {
        gap: 0;
      }
      .composer-input wa-textarea::part(form-control-label) {
        display: none;
      }
      .composer-input wa-textarea::part(base) {
        background: transparent;
        border: none;
        min-height: 36px;
        max-height: 200px;
        padding: 4px 8px;
      }
      .composer-bottom {
        display: flex;
        align-items: center;
        gap: var(--sebas-space-2);
        flex-wrap: wrap;
      }
      .composer-bottom .left-tools {
        display: flex;
        align-items: center;
        gap: var(--sebas-space-1);
      }
      .composer-bottom .right-tools {
        display: flex;
        align-items: center;
        gap: var(--sebas-space-2);
        margin-left: auto;
      }
      .icon-btn {
        width: 28px;
        height: 28px;
        display: grid;
        place-items: center;
        background: none;
        border: none;
        color: var(--sebas-text-dim);
        cursor: pointer;
        border-radius: var(--sebas-radius-md);
        transition: background var(--sebas-dur) var(--sebas-ease), color var(--sebas-dur) var(--sebas-ease);
      }
      .icon-btn:hover {
        background: var(--sebas-surface-2);
        color: var(--sebas-text-bright);
      }
      .attachment-chip {
        display: inline-flex;
        align-items: center;
        gap: 4px;
        padding: 0 8px;
        height: 28px;
        border-radius: var(--sebas-radius-md);
        background: var(--sebas-surface-2);
        color: var(--sebas-text-dim);
        font-size: 0.75rem;
        font-variant-numeric: tabular-nums;
      }
      .attachment-chip svg { opacity: 0.7; }

      /* Mode trigger + menu popover */
      .mode-trigger {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        padding: 0 10px;
        height: 28px;
        background: none;
        border: none;
        color: var(--sebas-accent);
        font-size: 0.78rem;
        font-weight: 500;
        cursor: pointer;
        border-radius: var(--sebas-radius-md);
        transition: background var(--sebas-dur) var(--sebas-ease);
      }
      .mode-trigger:hover { background: var(--sebas-surface-2); }
      .mode-trigger .chev {
        font-size: 9px;
        transition: transform var(--sebas-dur) var(--sebas-ease);
      }
      .mode-trigger[aria-expanded='true'] .chev { transform: rotate(180deg); }
      .mode-menu-pop {
        position: absolute;
        bottom: calc(100% + 6px);
        left: 0;
        z-index: 20;
        min-width: 280px;
        background: var(--sebas-surface);
        border: 1px solid var(--sebas-border);
        border-radius: var(--sebas-radius-lg);
        box-shadow: var(--sebas-shadow-l);
        padding: 6px;
        display: flex;
        flex-direction: column;
        gap: 2px;
        animation: sebas-menu-in 0.16s var(--sebas-ease);
      }
      @keyframes sebas-menu-in {
        from { opacity: 0; transform: translateY(4px); }
        to { opacity: 1; transform: none; }
      }
      .mode-item {
        display: flex;
        align-items: flex-start;
        gap: 10px;
        padding: 8px 10px;
        background: none;
        border: none;
        text-align: left;
        cursor: pointer;
        border-radius: var(--sebas-radius-md);
        color: var(--sebas-text);
        transition: background var(--sebas-dur) var(--sebas-ease);
      }
      .mode-item:hover { background: var(--sebas-surface-2); }
      .mode-item.active { background: var(--sebas-surface-2); }
      .mode-item .icon {
        display: grid;
        place-items: center;
        width: 28px;
        height: 28px;
        flex: 0 0 auto;
        border-radius: var(--sebas-radius-md);
        background: var(--sebas-surface-2);
        color: var(--sebas-accent);
      }
      .mode-item .body { flex: 1; min-width: 0; }
      .mode-item .body .label {
        font-size: 0.85rem;
        font-weight: 500;
        color: var(--sebas-text-bright);
      }
      .mode-item .body .desc {
        font-size: 0.72rem;
        color: var(--sebas-text-dim);
        margin-top: 1px;
      }
      .mode-item .check {
        display: grid;
        place-items: center;
        width: 18px;
        height: 18px;
        flex: 0 0 auto;
        color: var(--sebas-text-dim);
      }

      /* Right-side model picker (pill) */
      .model-pill {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        padding: 0 10px;
        height: 28px;
        background: none;
        border: none;
        color: var(--sebas-text-dim);
        font-size: 0.78rem;
        cursor: pointer;
        border-radius: var(--sebas-radius-md);
      }
      .model-pill:hover { background: var(--sebas-surface-2); color: var(--sebas-text-bright); }
      .model-pill .chev { font-size: 9px; }
      .send-button {
        width: 28px;
        height: 28px;
        display: grid;
        place-items: center;
        background: var(--sebas-accent);
        color: var(--sebas-accent-ink);
        border: none;
        border-radius: var(--sebas-radius-md);
        cursor: pointer;
        transition: opacity var(--sebas-dur) var(--sebas-ease);
      }
      .send-button:disabled { opacity: 0.35; cursor: not-allowed; }
      .send-button:hover:enabled { filter: brightness(1.05); }

      /* Mode trigger wrapper (positioning context for popover) */
      .mode-trigger-wrap { position: relative; }

      .empty-stream { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: var(--sebas-space-3); padding: var(--sebas-space-10); color: var(--sebas-text-dim); text-align: center; flex: 1; }
      .empty-stream .glyph { display: grid; place-items: center; width: 48px; height: 48px; border-radius: var(--sebas-radius-full); background: var(--sebas-surface-2); border: 1px solid var(--sebas-border); color: var(--sebas-text-faint); }
      .empty-stream .title { font-weight: 600; font-size: 1rem; color: var(--sebas-text-bright); }
      .empty-stream .hint { font-size: 0.85rem; max-width: 36ch; margin: 0; }

      /* ─── Turn folding (two levels) ─── */
      /* "折腾中" group, attached to the bottom of the agent bubble it
         belongs to (bleeds to the bubble edges, separated by a top rule). */
      .work-group {
        margin: var(--sebas-space-3) -14px -9px; /* bleed to the bubble edges */
        border-left: none;
        border-right: none;
        border-bottom: none;
        border-top: 1px solid var(--sebas-border);
        border-radius: 0;
        overflow: hidden;
        background: var(--sebas-surface-2);
      }
      .work-group .work-group-head {
        background: var(--sebas-surface-2);
        padding: 7px 14px;
      }
      .work-group-head {
        display: flex;
        align-items: center;
        gap: var(--sebas-space-2);
        padding: 8px 12px;
        background: var(--sebas-surface-2);
        cursor: pointer;
        font-size: 0.78rem;
        color: var(--sebas-text-dim);
        user-select: none;
        border: none;
        width: 100%;
        text-align: left;
        transition: background var(--sebas-dur) var(--sebas-ease);
      }
      .work-group-head:hover { background: var(--sebas-surface-3); }
      .work-group-head .chev {
        font-size: 9px;
        transition: transform var(--sebas-dur) var(--sebas-ease);
        color: var(--sebas-text-faint);
      }
      .work-group-head[aria-expanded='true'] .chev { transform: rotate(90deg); }
      .work-group-head .spinner-dot {
        width: 6px; height: 6px; border-radius: 50%;
        background: var(--sebas-status-working);
        animation: sebas-pulse 1.4s ease-in-out infinite;
      }
      @keyframes sebas-pulse {
        0%, 100% { opacity: 0.4; transform: scale(0.85); }
        50% { opacity: 1; transform: scale(1.15); }
      }
      .work-group-head .group-label {
        font-weight: 600;
        color: var(--sebas-text-bright);
        flex: 1;
      }
      .work-group-head .group-count {
        font-variant-numeric: tabular-nums;
        background: var(--sebas-surface-3);
        border-radius: var(--sebas-radius-full);
        padding: 0 7px;
        font-size: 0.65rem;
        color: var(--sebas-text-faint);
      }
      .work-group-body {
        display: flex;
        flex-direction: column;
        gap: 1px;
        background: var(--sebas-surface);
      }
      .work-group-body.collapsed { display: none; }

      /* Inner blocks: each tool/thought individually foldable */
      .work-block {
        border-bottom: 1px solid var(--sebas-border);
      }
      .work-block:last-child { border-bottom: none; }
      .work-block-head {
        display: flex;
        align-items: center;
        gap: 8px;
        padding: 6px 12px;
        background: var(--sebas-surface);
        cursor: pointer;
        font-size: 0.74rem;
        color: var(--sebas-text-dim);
        user-select: none;
        border: none;
        width: 100%;
        text-align: left;
        font-family: var(--sebas-font-mono);
        transition: background var(--sebas-dur) var(--sebas-ease);
      }
      .work-block-head:hover { background: var(--sebas-surface-2); }
      .work-block-head .chev {
        font-size: 8px;
        transition: transform var(--sebas-dur) var(--sebas-ease);
        color: var(--sebas-text-faint);
      }
      .work-block-head[aria-expanded='true'] .chev { transform: rotate(90deg); }
      .work-block-head .kind-icon {
        display: grid;
        place-items: center;
        width: 18px;
        height: 18px;
        border-radius: var(--sebas-radius-sm);
        background: var(--sebas-surface-2);
        color: var(--sebas-text-faint);
      }
      .work-block-head .kind-icon.thought {
        background: var(--sebas-accent-soft);
        color: var(--sebas-accent);
      }
      .work-block-head .kind-icon.tool {
        background: var(--sebas-status-queued-bg);
        color: var(--sebas-status-queued);
      }
      .work-block-head .kind {
        font-size: 0.65rem;
        text-transform: uppercase;
        letter-spacing: 0.06em;
        color: var(--sebas-text-faint);
      }
      .work-block-head .summary {
        flex: 1;
        color: var(--sebas-text);
      }
      .work-block-body {
        padding: 8px 12px 12px;
        font-size: 0.82rem;
        line-height: 1.6;
        color: var(--sebas-text);
        border-top: 1px dashed var(--sebas-border);
      }
      .work-block-body.collapsed { display: none; }
      .work-block-body .tool-call { display: flex; align-items: center; gap: var(--sebas-space-2); padding: var(--sebas-space-2) var(--sebas-space-3); background: var(--sebas-surface-2); border-radius: var(--sebas-radius-md); font-family: var(--sebas-font-mono); font-size: 0.78rem; color: var(--sebas-text-dim); margin: var(--sebas-space-2) 0; }
      .work-block-body .tool-call .file { color: var(--sebas-accent); }
    `,
  ]

  // ─── Composer ───
  private handleKeyDown(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      this.sendMessage()
    }
  }

  private sendMessage() {
    if (!this.messageText.trim()) return
    const sessionId = this.activeSession
    const turns = this.turnsBySession[sessionId] ?? []
    this.turnsBySession = {
      ...this.turnsBySession,
      [sessionId]: [
        ...turns,
        { role: 'user' as const, author: 'you', time: new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }), content: this.messageText },
      ],
    }
    this.messageText = ''
    this.requestUpdate()
    // Simulate agent response after a delay
    setTimeout(() => {
      const currentTurns = this.turnsBySession[sessionId] ?? []
      this.turnsBySession = {
        ...this.turnsBySession,
        [sessionId]: [
          ...currentTurns,
          { role: 'assistant' as const, author: 'claude', time: new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }), content: `<p>好的，收到。我来处理...</p>` },
        ],
      }
      this.requestUpdate()
    }, 800)
  }

  // ─── Turn folding: render "折腾中" as part of the agent bubble ───
  /**
   * Walk the turn stream and render each message. A run of tool/thought
   * turns that is immediately followed by an assistant text is treated as
   * that assistant's "thinking + acting" span: the collapsible 折腾中 group
   * is attached to the BOTTOM of the agent bubble.
   *
   * A tool/thought run with no following assistant is not rendered at all —
   * the 折腾中 group only ever appears inside an agent bubble.
   */
  private renderTurns(turns: TurnItem[]) {
    const out: ReturnType<typeof html>[] = []
    let i = 0
    while (i < turns.length) {
      const t = turns[i]!
      if (t.role === 'seam') {
        out.push(html`<div class="seam"><span class="count">${t.count} new</span><span>· ${t.label}</span></div>`)
        i++
        continue
      }
      if (t.role === 'tool' || t.role === 'thought') {
        // Collect a run of consecutive tool/thought
        const items: TurnItem[] = []
        let j = i
        while (j < turns.length && (turns[j]!.role === 'tool' || turns[j]!.role === 'thought')) {
          items.push(turns[j]!)
          j++
        }
        // Only render when an assistant text turn follows this run; otherwise
        // the work has no bubble to attach to and is skipped.
        const next = turns[j]
        if (next?.role === 'assistant') {
          out.push(html`
            <div class="turn-block is-assistant">
              <div class="avatar assistant">AI</div>
              <div class="bubble">
                <div class="meta">
                  <span class="author">assistant</span>
                  <span class="time">${next.time ?? ''}</span>
                </div>
                <div class="body">${unsafeHTML(next.content ?? '')}</div>
                ${this.renderWorkGroup(items, i)}
              </div>
            </div>
          `)
          i = j + 1
          continue
        }
        i = j
        continue
      }
      // Plain text turn (user, or assistant not preceded by work)
      out.push(html`
        <div class="turn-block ${t.role === 'user' ? 'is-user' : 'is-assistant'}">
          <div class="avatar ${t.author === 'you' ? 'user' : 'assistant'}">${t.author === 'you' ? '你' : 'AI'}</div>
          <div class="bubble">
            <div class="meta">
              <span class="author ${t.author === 'you' ? 'you' : ''}">${t.author === 'you' ? 'you' : 'assistant'}</span>
              <span class="time">${t.time}</span>
            </div>
            <div class="body">${t.role === 'assistant' ? unsafeHTML(t.content ?? '') : html`<p>${t.content}</p>`}</div>
          </div>
        </div>
      `)
      i++
    }
    return out
  }

  /** Build a single "折腾中" foldable group rendered inside an agent bubble. */
  private renderWorkGroup(items: TurnItem[], startIndex: number) {
    const groupId = `g-${this.activeSession}-${startIndex}`
    const groupExpanded = !!this.expandedGroups[groupId]
    return html`
      <div class="work-group">
        <button
          class="work-group-head"
          aria-expanded=${groupExpanded ? 'true' : 'false'}
          @click=${() => (this.expandedGroups = { ...this.expandedGroups, [groupId]: !groupExpanded })}
        >
          <span class="spinner-dot" aria-hidden="true"></span>
          <span class="group-label">折腾中</span>
          <span class="group-count">${items.length} ${items.length === 1 ? 'step' : 'steps'}</span>
          <span class="chev">▶</span>
        </button>
        <div class="work-group-body ${groupExpanded ? '' : 'collapsed'}">
          ${items.map((it, idx) => {
            const blockId = `b-${this.activeSession}-${startIndex}-${idx}`
            const blockExpanded = !!this.expandedBlocks[blockId]
            const kind = it.role === 'thought' ? 'thought' : 'tool'
            return html`
              <div class="work-block">
                <button
                  class="work-block-head"
                  aria-expanded=${blockExpanded ? 'true' : 'false'}
                  @click=${() => (this.expandedBlocks = { ...this.expandedBlocks, [blockId]: !blockExpanded })}
                >
                  <span class="chev">▶</span>
                  <span class="kind-icon ${kind}">${icon(kind === 'thought' ? 'shield' : 'alert', 11)}</span>
                  <span class="kind">${kind}</span>
                  <span class="summary">${it.summary ?? (kind === 'thought' ? 'Thinking…' : 'Tool call')}</span>
                </button>
                <div class="work-block-body ${blockExpanded ? '' : 'collapsed'}">
                  ${unsafeHTML(it.content ?? '')}
                </div>
              </div>
            `
          })}
        </div>
      </div>
    `
  }

  render() {
    const project = this.activeProject
    const turns = this.activeTurns

    return html`
      <div style="display:flex;flex:1;flex-direction:column;min-height:0;min-width:0;">
        <!-- Project header -->
        <div class="project-header">
          ${project ? html`
            <span class="path">${project.name}</span>
            ${project.gitBranch ? html`<span class="branch">${project.gitBranch}</span>` : nothing}
            <span class="project-meta">
              <span class="meta-item"><wa-icon name="layer-group" style="font-size:12px;" aria-hidden="true"></wa-icon>${project.sessions} sessions</span>
              <span class="meta-item" style="color:var(--sebas-status-working);"><span style="width:6px;height:6px;border-radius:50%;background:var(--sebas-status-working);display:inline-block;"></span>${project.hasActive ? '1 active' : 'idle'}</span>
            </span>
          ` : html`
            <span class="path" style="color:var(--sebas-text-faint);">No project selected</span>
          `}
        </div>

        <!-- Turn stream -->
        <div class="turn-stream-area" style="flex:1;overflow-y:auto;">
          ${turns.length > 0 ? html`
            <div class="turn-stream">
              ${this.renderTurns(turns)}
            </div>
          ` : html`
            <div class="empty-stream">
              <div class="glyph"><wa-icon name="comment" style="font-size:20px;"></wa-icon></div>
              <span class="title">${this.activeSession ? 'Start a conversation' : 'No session selected'}</span>
              <p class="hint">Create a new session in the sidebar under this project.</p>
            </div>
          `}
        </div>

        <!-- Composer -->
        <div class="composer-area">
          <div class="composer-shell">
            ${this.activeSession
              ? html`<div class="composer-hint">Ask for follow-up changes</div>`
              : html`<div class="composer-hint">Pick or create a session in the sidebar to start a conversation</div>`}
            <div class="composer-input">
              <wa-textarea
                placeholder="Ask Claude…"
                rows="2"
                .value=${this.messageText}
                @wa-input=${(e: any) => (this.messageText = e.target.value)}
                @keydown=${this.handleKeyDown}
              ></wa-textarea>
            </div>
            <div class="composer-bottom">
              <div class="left-tools">
                <button class="icon-btn" aria-label="Attach file" title="Attach files">
                  ${icon('plus', 16)}
                </button>
                <div class="mode-trigger-wrap">
                  <button
                    class="mode-trigger"
                    aria-label="Switch mode"
                    aria-expanded=${this.modeMenuOpen ? 'true' : 'false'}
                    @click=${(e: Event) => {
                      e.stopPropagation()
                      this.modeMenuOpen = !this.modeMenuOpen
                    }}
                  >
                    ${this.mode === 'full' ? icon('unlock', 13) : icon('shield', 13)}
                    ${this.modeLabels[this.mode].short}
                    <span class="chev">▼</span>
                  </button>
                  ${this.modeMenuOpen
                    ? html`
                        <div class="mode-menu-pop" role="menu">
                          ${(['ask', 'auto', 'plan', 'full'] as const).map(
                            (m) => html`
                              <button
                                class="mode-item ${this.mode === m ? 'active' : ''}"
                                role="menuitemradio"
                                aria-checked=${this.mode === m ? 'true' : 'false'}
                                @click=${(e: Event) => {
                                  e.stopPropagation()
                                  this.mode = m
                                  this.modeMenuOpen = false
                                }}
                              >
                                <span class="icon">${
                                  m === 'ask' ? icon('hand', 14)
                                    : m === 'auto' ? icon('wand', 14)
                                    : m === 'plan' ? icon('plan', 14)
                                    : icon('unlock', 14)
                                }</span>
                                <span class="body">
                                  <span class="label">${this.modeLabels[m].label}</span>
                                  <span class="desc">${this.modeLabels[m].title}</span>
                                </span>
                                ${this.mode === m
                                  ? html`<span class="check">✓</span>`
                                  : nothing}
                              </button>
                            `
                          )}
                        </div>
                      `
                    : nothing}
                </div>
                ${this.attachmentCount > 0
                  ? html`<span class="attachment-chip" title="${this.attachmentCount} attachments">${icon('paperclip', 11)} ${this.attachmentCount}</span>`
                  : nothing}
              </div>
              <div class="right-tools">
                <wa-select
                  class="model-pill-select"
                  size="xs"
                  hoist
                  value=${this.selectedModelKey}
                  @wa-change=${(e: any) => (this.selectedModelKey = e.target.value)}
                  style="display:none;"
                >
                  ${this.modelOptions.map((o) => html`<wa-option value=${o.value}>${o.label}</wa-option>`)}
                </wa-select>
                <button class="model-pill" @click=${(e: Event) => {
                  const next = (e.currentTarget as HTMLElement).nextElementSibling as any
                  next?.show?.()
                }}>
                  <span class="model-pill-dot"></span>
                  ${this.modelOptions.find((o) => o.value === this.selectedModelKey)?.label ?? this.selectedModelKey}
                  <span class="chev">▼</span>
                </button>
                <button class="send-button" aria-label="Send" @click=${this.sendMessage} ?disabled=${!this.messageText.trim() || !this.activeSession}>
                  ${icon('arrowUp', 14)}
                </button>
              </div>
            </div>
          </div>
        </div>
      </div>
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-preview-workbench': SebasPreviewWorkbench
  }
}