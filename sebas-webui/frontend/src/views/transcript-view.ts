/**
 * Streaming transcript view with a per-session "seen" boundary.
 *
 * Renders each entry in `entries` as a left-rail timestamp + content body.
 * Above the entries sits a thin "seam" strip that highlights anything
 * appended since the reader last looked at the session; the seam is
 * anchored to the largest `created_at_unix` the client has actually
 * scrolled past (or clicked through). Anchoring by timestamp — not by
 * array index — keeps the seam pinned to the same logical entry even
 * when an older card refreshes in place (spec 4.4: the router stamps
 * `created_at_unix` at push time, never on refresh).
 *
 * Scroll behaviour:
 *   - while `sticky` is true, the view auto-scrolls to the seam (when
 *     there are unseen entries) or to the bottom (when everything is
 *     already seen)
 *   - a near-bottom scroll (within 80px) marks entries as seen (250ms
 *     debounce, monotonic — only ever advances the boundary)
 *   - if the reader scrolls up past the seam, sticky flips off so we
 *     don't fight them; scrolling back down to the seam re-engages it
 */

import { LitElement, css, html, nothing } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { unsafeHTML } from 'lit/directives/unsafe-html.js'
import type { CardElementView } from '../api/client.js'
import { renderMarkdown } from '../components/markdown.js'

/** Bottom-scroll threshold for "mark-as-seen" detection. */
const NEAR_BOTTOM_PX = 80
/** Debounce window for mark-as-seen writes. */
const MARK_SEEN_DEBOUNCE_MS = 250

/**
 * Render a unix-seconds timestamp into the format dictated by the spec:
 *   - today          → HH:MM:SS
 *   - this year      → MM-DD HH:MM
 *   - older          → YYYY-MM-DD
 * Returns an empty string for falsy timestamps (legacy entries).
 */
function formatTime(unixSecs: number): string {
  if (!unixSecs) return ''
  const d = new Date(unixSecs * 1000)
  const now = new Date()
  const sameYear = d.getFullYear() === now.getFullYear()
  const sameDay =
    d.getFullYear() === now.getFullYear() &&
    d.getMonth() === now.getMonth() &&
    d.getDate() === now.getDate()
  const pad = (n: number) => String(n).padStart(2, '0')
  if (sameDay) {
    return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`
  }
  if (sameYear) {
    return `${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`
  }
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`
}

/** ISO 8601 string for `<time datetime=...>`. Empty string if no timestamp. */
function isoTime(unixSecs: number): string {
  if (!unixSecs) return ''
  return new Date(unixSecs * 1000).toISOString()
}

@customElement('sebas-transcript-view')
export class SebasTranscriptView extends LitElement {
  /** The transcript blocks from `SessionDetail.body`. */
  @property({ attribute: false }) entries: CardElementView[] = []
  /** Encoded session key; namespaces the seen-boundary in localStorage. */
  @property() sessionKey = ''
  /**
   * When true (default), auto-scroll on new entries. Flipped to false
   * internally when the reader scrolls up past the seam so we don't
   * fight deliberate scroll-up.
   */
  @property({ type: Boolean }) sticky = true

  /** Number of entries strictly after the seam. */
  @state() private unseenCount = 0
  /** Index of the first unseen entry; null when everything is seen. */
  @state() private seamIndex: number | null = 0

  /** Debounce timer for mark-as-seen writes. */
  private markSeenTimer: number | null = null
  /** Bound scroll handler so we can detach on disconnect. */
  private boundOnScroll = (): void => this.onScroll()
  /** Bound resize handler — scrollIntoView needs a layout flush. */
  private boundOnResize = (): void => {
    if (this.sticky) this.applyAutoScroll()
  }

  static styles = css`
    :host {
      display: block;
      --ts-rail: 64px;
    }
    .scroll {
      max-height: 58vh;
      overflow-y: auto;
      padding: var(--sebas-space-4) var(--sebas-space-5);
      scroll-behavior: smooth;
    }
    .seam {
      display: flex;
      align-items: center;
      justify-content: center;
      gap: var(--sebas-space-2);
      padding: var(--sebas-space-3) var(--sebas-space-4);
      margin: 0 calc(-1 * var(--sebas-space-4)) var(--sebas-space-4);
      background: var(--sebas-status-working-bg);
      border-top: 1px solid var(--sebas-status-working-border);
      border-bottom: 1px solid var(--sebas-status-working-border);
      color: var(--sebas-text-bright);
      font-size: 0.85rem;
    }
    .seam[hidden] {
      display: none;
    }
    .seam .pill {
      display: inline-flex;
      align-items: center;
      gap: var(--sebas-space-2);
      padding: 4px 10px;
      border-radius: var(--sebas-radius-full);
      background: var(--sebas-surface-2);
      border: 1px solid var(--sebas-status-working-border);
      font-variant-numeric: tabular-nums;
    }
    .seam .link {
      color: var(--sebas-accent);
      text-decoration: underline;
      text-underline-offset: 3px;
      cursor: pointer;
      background: none;
      border: none;
      padding: 0;
      font: inherit;
    }
    .seam .link:hover {
      color: var(--sebas-accent-hover);
    }
    .entry {
      display: grid;
      grid-template-columns: var(--ts-rail) 1fr;
      column-gap: var(--sebas-space-4);
      padding: var(--sebas-space-3) 0;
      border-bottom: 1px solid var(--sebas-border);
    }
    .entry:last-child {
      border-bottom: none;
    }
    .entry .ts {
      grid-column: 1;
      align-self: start;
      color: var(--sebas-text-faint);
      font-family: var(--sebas-font-mono);
      font-size: 0.75rem;
      line-height: 1.5;
      white-space: nowrap;
      font-variant-numeric: tabular-nums;
      padding-top: 2px;
    }
    .entry .body {
      grid-column: 2;
      min-width: 0;
      color: var(--sebas-text);
    }
    .entry .body :is(p, pre, ul, ol, h1, h2, h3, h4) {
      overflow-wrap: break-word;
    }
    .entry .body :first-child {
      margin-top: 0;
    }
    .entry .body :last-child {
      margin-bottom: 0;
    }
    .entry .body h1,
    .entry .body h2,
    .entry .body h3 {
      color: var(--sebas-text-bright);
      letter-spacing: -0.01em;
    }
    .entry .body a {
      color: var(--sebas-accent);
      text-decoration: underline;
      text-underline-offset: 3px;
    }
    .entry .body pre {
      background: var(--sebas-well);
      border: 1px solid var(--sebas-border);
      border-radius: var(--sebas-radius-md);
      padding: var(--sebas-space-3);
      overflow-x: auto;
      font-family: var(--sebas-font-mono);
      font-size: 0.82rem;
      line-height: 1.55;
    }
    .entry .body code {
      font-family: var(--sebas-font-mono);
      font-size: 0.88em;
    }
    .entry .body :not(pre) > code {
      background: var(--sebas-surface-3);
      border-radius: var(--sebas-radius-sm);
      padding: 1px 5px;
    }
    .entry .body blockquote {
      margin: 0.5em 0;
      padding: 0.1em 1em;
      border-left: 3px solid var(--sebas-border-strong);
      color: var(--sebas-text-dim);
    }
    .entry.thinking {
      background: var(--sebas-surface-3);
      border-radius: var(--sebas-radius-md);
      padding: var(--sebas-space-2) var(--sebas-space-3);
      margin-bottom: var(--sebas-space-2);
    }
    .entry.thinking .ts {
      color: var(--sebas-text-faint);
      opacity: 0.8;
    }
    .entry.thinking summary {
      cursor: pointer;
      color: var(--sebas-text-dim);
      font-size: 0.78rem;
      text-transform: uppercase;
      letter-spacing: 0.08em;
      user-select: none;
    }
    .entry.thinking summary:hover {
      color: var(--sebas-text-bright);
    }
    .entry.thinking .thinking-body {
      margin-top: var(--sebas-space-2);
      padding-top: var(--sebas-space-2);
      border-top: 1px dashed var(--sebas-border);
    }
  `

  connectedCallback(): void {
    super.connectedCallback()
    this.recomputeSeam()
    window.addEventListener('resize', this.boundOnResize)
  }

  disconnectedCallback(): void {
    super.disconnectedCallback()
    window.removeEventListener('resize', this.boundOnResize)
    if (this.markSeenTimer !== null) {
      clearTimeout(this.markSeenTimer)
      this.markSeenTimer = null
    }
  }

  protected willUpdate(changed: Map<string, unknown>): void {
    if (changed.has('entries') || changed.has('sessionKey')) this.recomputeSeam()
  }

  protected updated(changed: Map<string, unknown>): void {
    super.updated(changed)
    // The scroll listener must be re-attached whenever the scroll
    // container is replaced in the DOM; this happens every render. We
    // diff against a private field so we don't pile up duplicate
    // listeners on the same node.
    const el = this.renderRoot.querySelector<HTMLElement>('.scroll')
    if (el && el !== this.scrollEl) {
      this.scrollEl?.removeEventListener('scroll', this.boundOnScroll)
      this.scrollEl = el
      el.addEventListener('scroll', this.boundOnScroll, { passive: true })
    }
    if (changed.has('entries') || changed.has('sessionKey') || changed.has('seamIndex')) {
      // Wait one frame for layout to settle, then scroll. Without the
      // rAF, scrollHeight can lag the freshly-inserted entries.
      requestAnimationFrame(() => this.applyAutoScroll())
    }
  }

  /** The current scroll container, if any. */
  private scrollEl: HTMLElement | null = null

  // ---- localStorage helpers --------------------------------------------

  private static storageKey(sessionKey: string): string {
    return `sebas:seen:${sessionKey}`
  }

  private readSeen(): number {
    try {
      const raw = localStorage.getItem(SebasTranscriptView.storageKey(this.sessionKey))
      if (raw === null) return 0
      const n = Number(raw)
      return Number.isFinite(n) ? n : 0
    } catch {
      return 0
    }
  }

  private writeSeen(value: number): void {
    try {
      localStorage.setItem(SebasTranscriptView.storageKey(this.sessionKey), String(value))
    } catch {
      /* storage may be disabled; degrade silently */
    }
  }

  // ---- seam logic -------------------------------------------------------

  /**
   * Recompute `seamIndex` and `unseenCount` from the current entries
   * and the stored seen-boundary. The seam is the first index whose
   * `created_at_unix` is strictly greater than the stored value; entries
   * without a timestamp (legacy, value 0) are ignored — they cannot
   * advance the seam, because anchoring on them would let an
   * unknown-time entry push the seam onto a clearly-seen neighbour.
   */
  private recomputeSeam(): void {
    const seen = this.readSeen()
    const ts = this.entries.map((e) => e.created_at_unix || 0)
    if (ts.length === 0) {
      this.seamIndex = null
      this.unseenCount = 0
      return
    }
    const maxTs = ts.reduce((a, b) => (b > a ? b : a), 0)
    if (seen > 0 && seen >= maxTs) {
      // Everything is at or below the stored boundary.
      this.seamIndex = null
      this.unseenCount = 0
      return
    }
    const idx = this.entries.findIndex((e) => (e.created_at_unix || 0) > seen)
    if (idx === -1) {
      this.seamIndex = null
      this.unseenCount = 0
    } else {
      this.seamIndex = idx
      this.unseenCount = this.entries.length - idx
    }
  }

  // ---- mark-all-seen ----------------------------------------------------

  private markAllSeen = (): void => {
    const ts = this.entries.map((e) => e.created_at_unix || 0)
    const max = ts.length === 0 ? 0 : ts.reduce((a, b) => (b > a ? b : a), 0)
    this.writeSeen(max)
    this.seamIndex = null
    this.unseenCount = 0
  }

  // ---- scroll handling --------------------------------------------------

  private onScroll(): void {
    const el = this.scrollEl
    if (!el) return
    const distanceFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight
    const nearBottom = distanceFromBottom <= NEAR_BOTTOM_PX
    const seam = this.renderRoot.querySelector<HTMLElement>('.seam')
    const seamTop = seam ? seam.offsetTop : Number.POSITIVE_INFINITY
    // Distance the reader has scrolled relative to the seam. Positive
    // when they're at or below the seam; negative when above it.
    const relativeToSeam = el.scrollTop - seamTop
    if (nearBottom || relativeToSeam >= 0) {
      // The reader is at-or-past the seam — re-engage sticky if we'd
      // disengaged it on a deliberate scroll-up.
      if (!this.sticky) this.sticky = true
      this.scheduleMarkSeen()
    } else {
      // The reader has scrolled above the seam — let them read old
      // content without us yanking them back.
      if (this.sticky) this.sticky = false
    }
  }

  private scheduleMarkSeen(): void {
    if (this.markSeenTimer !== null) return
    this.markSeenTimer = window.setTimeout(() => {
      this.markSeenTimer = null
      this.commitMarkSeen()
    }, MARK_SEEN_DEBOUNCE_MS)
  }

  /** Push the seen-boundary forward to the newest rendered entry. */
  private commitMarkSeen(): void {
    const ts = this.entries.map((e) => e.created_at_unix || 0)
    if (ts.length === 0) return
    const max = ts.reduce((a, b) => (b > a ? b : a), 0)
    if (max > this.readSeen()) {
      this.writeSeen(max)
      // The seam may have moved or disappeared; update internal state
      // and re-render without scheduling another auto-scroll — the user
      // is already where they want to be.
      const prev = this.seamIndex
      this.recomputeSeam()
      if (prev !== this.seamIndex) this.requestUpdate('seamIndex')
    }
  }

  /** Apply the spec-mandated scroll behaviour for the current frame. */
  private applyAutoScroll(): void {
    const el = this.scrollEl
    if (!el || !this.sticky) return
    if (this.seamIndex !== null && this.unseenCount > 0) {
      const seam = this.renderRoot.querySelector<HTMLElement>('.seam')
      if (seam && typeof seam.scrollIntoView === 'function') {
        seam.scrollIntoView({ block: 'center' })
      }
    } else {
      // Already-seen case: stick to the newest entry.
      el.scrollTop = el.scrollHeight
    }
  }

  // ---- render -----------------------------------------------------------

  render() {
    const showSeam = this.unseenCount > 0
    return html`
      <div class="scroll" role="log" aria-label="Session transcript">
        ${showSeam
          ? html`
              <div class="seam" data-count=${this.unseenCount} role="status">
                <span class="pill">~${this.unseenCount} new since you last viewed</span>
                <button type="button" class="link" @click=${this.markAllSeen}>
                  mark all seen
                </button>
              </div>
            `
          : html`<div class="seam" hidden></div>`}
        ${this.entries.map((e) => this.renderEntry(e))}
      </div>
    `
  }

  private renderEntry(e: CardElementView) {
    if (!e.content) return nothing
    const iso = isoTime(e.created_at_unix)
    const ts = formatTime(e.created_at_unix)
    if (e.element_type === 'thinking') {
      return html`
        <section class="entry thinking">
          <time class="ts" datetime=${iso || nothing}>${ts}</time>
          <div class="body">
            <details>
              <summary>thinking</summary>
              <div class="thinking-body">${unsafeHTML(renderMarkdown(e.content))}</div>
            </details>
          </div>
        </section>
      `
    }
    // Default to markdown for unknown element_types so the user still
    // sees the content (legacy `collapsible`/`div` shapes fall through).
    return html`
      <section class="entry">
        <time class="ts" datetime=${iso || nothing}>${ts}</time>
        <div class="body">${unsafeHTML(renderMarkdown(e.content))}</div>
      </section>
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-transcript-view': SebasTranscriptView
  }
}
