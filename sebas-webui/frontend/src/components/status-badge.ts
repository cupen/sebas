/**
 * Operator-facing status badge. Renders the backend-owned status projection
 * (label + slug + glyph) as a tinted pill: glyph shape, colored dot, and the
 * word — so state survives greyscale and colour-blindness (shape + word +
 * colour). Pill background/border/text come from the per-slug status tokens
 * via the data-status attribute; the dot keeps an inline `--sebas-status-*`
 * reference (stylesheet hook, mirrors the SSR projection).
 *
 * The mapping lives server-side (single source); this component only
 * renders what the API sends.
 */

import { LitElement, css, html, nothing } from 'lit'
import { customElement, property } from 'lit/decorators.js'

export type StatusSlug = 'starting' | 'queued' | 'working' | 'done' | 'failed' | 'dormant'

@customElement('sebas-status-badge')
export class SebasStatusBadge extends LitElement {
  @property() slug: StatusSlug | string = 'queued'
  @property() label = ''
  @property() glyph = ''

  static styles = css`
    .badge {
      display: inline-flex;
      align-items: center;
      gap: 7px;
      padding: 3px 10px 3px 9px;
      border-radius: var(--sebas-radius-full, 999px);
      border: 1px solid var(--sebas-status-queued-border, rgba(56, 209, 221, 0.3));
      background: var(--sebas-status-queued-bg, rgba(56, 209, 221, 0.1));
      color: var(--sebas-status-queued, #38d1dd);
      font-size: 0.78rem;
      font-weight: 550;
      letter-spacing: 0.01em;
      line-height: 1.4;
      font-variant-numeric: tabular-nums;
      white-space: nowrap;
      transition:
        background var(--sebas-dur, 150ms) var(--sebas-ease, ease),
        border-color var(--sebas-dur, 150ms) var(--sebas-ease, ease);
    }
    .badge[data-status='starting'] {
      background: var(--sebas-status-starting-bg);
      border-color: var(--sebas-status-starting-border);
      color: var(--sebas-status-starting);
    }
    .badge[data-status='queued'] {
      background: var(--sebas-status-queued-bg);
      border-color: var(--sebas-status-queued-border);
      color: var(--sebas-status-queued);
    }
    .badge[data-status='working'] {
      background: var(--sebas-status-working-bg);
      border-color: var(--sebas-status-working-border);
      color: var(--sebas-status-working);
    }
    .badge[data-status='done'] {
      background: var(--sebas-status-done-bg);
      border-color: var(--sebas-status-done-border);
      color: var(--sebas-status-done);
    }
    .badge[data-status='failed'] {
      background: var(--sebas-status-failed-bg);
      border-color: var(--sebas-status-failed-border);
      color: var(--sebas-status-failed);
    }
    .badge[data-status='dormant'] {
      background: var(--sebas-status-dormant-bg);
      border-color: var(--sebas-status-dormant-border);
      color: var(--sebas-status-dormant);
    }
    .glyph {
      font-size: 0.9em;
      line-height: 1;
    }
    .dot {
      position: relative;
      width: 7px;
      height: 7px;
      border-radius: 50%;
      background: var(--sebas-status-queued, #38d1dd);
      flex: 0 0 auto;
    }
    /* Live pulse on the working status — CSS only, honours reduced motion. */
    .badge[data-status='working'] .dot::before {
      content: '';
      position: absolute;
      inset: -3px;
      border-radius: 50%;
      border: 1px solid var(--sebas-status-working, currentColor);
      animation: sebas-ping 1.6s var(--sebas-ease, ease) infinite;
    }
    @keyframes sebas-ping {
      0% {
        transform: scale(0.55);
        opacity: 0.9;
      }
      80%,
      100% {
        transform: scale(1.7);
        opacity: 0;
      }
    }
    @media (prefers-reduced-motion: reduce) {
      .badge[data-status='working'] .dot::before {
        animation: none;
        opacity: 0;
      }
    }
    .label {
      font-size: 0.92em;
    }
  `

  private get statusColor(): string {
    const slug = this.slug.replace(/[^a-z]/g, '')
    return `var(--sebas-status-${slug}, var(--sebas-status-queued, #b8860b))`
  }

  render() {
    // Glyph and label are always rendered: colour is never the only channel.
    return html`<span class="badge" data-status=${this.slug}>
      <span class="dot" style=${`background: ${this.statusColor}`} aria-hidden="true"></span>
      <span class="glyph" aria-hidden="true">${this.glyph || nothing}</span>
      <span class="label">${this.label || this.slug}</span>
    </span>`
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-status-badge': SebasStatusBadge
  }
}
