/**
 * Shared view style kit. Every routed view composes `viewStyles` with its
 * own `css` fragment so panels, tables, empty states, skeletons, callouts
 * and key/value rows look identical everywhere. These live inside each
 * component's shadow root, so the focus-visible rules here cover anchors
 * and focusable elements that the document-level rule cannot reach.
 */

import { css } from 'lit'

export const viewStyles = css`
  :host {
    display: block;
  }

  /* ---- Page header ------------------------------------------------- */
  .page-head {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: var(--sebas-space-4);
    flex-wrap: wrap;
    margin-bottom: var(--sebas-space-5);
  }
  .page-title {
    margin: 0;
    font-size: 1.25rem;
    font-weight: 650;
    letter-spacing: -0.015em;
    color: var(--sebas-text-bright);
  }
  .page-sub {
    margin: var(--sebas-space-1) 0 0;
    color: var(--sebas-text-dim);
    font-size: 0.875rem;
  }

  /* ---- Panels ------------------------------------------------------- */
  .panel {
    background: var(--sebas-surface);
    border: 1px solid var(--sebas-border);
    border-radius: var(--sebas-radius-lg);
    box-shadow: var(--sebas-shadow-1);
    overflow: hidden;
  }
  .panel-pad {
    padding: var(--sebas-space-4);
  }
  .panel-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sebas-space-3);
    flex-wrap: wrap;
    padding: var(--sebas-space-3) var(--sebas-space-4);
    border-bottom: 1px solid var(--sebas-border);
  }
  .panel-title {
    margin: 0;
    font-size: 0.95rem;
    font-weight: 600;
    color: var(--sebas-text-bright);
  }
  .panel-caption {
    color: var(--sebas-text-faint);
    font-size: 0.8rem;
  }

  /* ---- Tables ------------------------------------------------------- */
  table {
    width: 100%;
    border-collapse: collapse;
  }
  th,
  td {
    text-align: left;
    padding: var(--sebas-space-3) var(--sebas-space-4);
    border-bottom: 1px solid var(--sebas-border);
    vertical-align: middle;
  }
  th {
    color: var(--sebas-text-faint);
    font-weight: 550;
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    background: var(--sebas-surface-2);
    white-space: nowrap;
  }
  tbody tr {
    transition: background var(--sebas-dur) var(--sebas-ease);
  }
  tbody tr:hover {
    background: var(--sebas-surface-2);
  }
  tbody tr:last-child td {
    border-bottom: none;
  }
  /* Status accent on the leading edge of a row (set data-status on the tr). */
  tbody tr[data-status] td:first-child {
    box-shadow: inset 2px 0 0 0 var(--sebas-status-dormant);
  }
  tbody tr[data-status='starting'] td:first-child {
    box-shadow: inset 2px 0 0 0 var(--sebas-status-starting);
  }
  tbody tr[data-status='queued'] td:first-child {
    box-shadow: inset 2px 0 0 0 var(--sebas-status-queued);
  }
  tbody tr[data-status='working'] td:first-child {
    box-shadow: inset 2px 0 0 0 var(--sebas-status-working);
  }
  tbody tr[data-status='done'] td:first-child {
    box-shadow: inset 2px 0 0 0 var(--sebas-status-done);
  }
  tbody tr[data-status='failed'] td:first-child {
    box-shadow: inset 2px 0 0 0 var(--sebas-status-failed);
  }
  tbody tr[data-status='dormant'] td:first-child {
    box-shadow: inset 2px 0 0 0 var(--sebas-status-dormant);
  }

  /* ---- Typography helpers ------------------------------------------ */
  .mono {
    font-family: var(--sebas-font-mono);
    font-size: 0.82rem;
  }
  .tnum {
    font-variant-numeric: tabular-nums;
  }
  .dim {
    color: var(--sebas-text-dim);
  }

  /* ---- Links -------------------------------------------------------- */
  a {
    color: var(--sebas-accent);
    text-decoration: none;
    transition: color var(--sebas-dur) var(--sebas-ease);
  }
  a:hover {
    color: var(--sebas-accent-hover);
    text-decoration: underline;
    text-underline-offset: 3px;
  }
  a:focus-visible {
    outline: var(--sebas-focus-ring);
    outline-offset: 2px;
    border-radius: var(--sebas-radius-sm);
  }

  /* ---- Empty states -------------------------------------------------- */
  .empty {
    padding: var(--sebas-space-10) var(--sebas-space-6);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--sebas-space-2);
    text-align: center;
    color: var(--sebas-text-dim);
  }
  .empty .glyph {
    display: grid;
    place-items: center;
    width: 44px;
    height: 44px;
    border-radius: var(--sebas-radius-full);
    background: var(--sebas-surface-2);
    border: 1px solid var(--sebas-border);
    color: var(--sebas-text-faint);
    margin-bottom: var(--sebas-space-2);
  }
  .empty .title {
    color: var(--sebas-text-bright);
    font-weight: 600;
    font-size: 0.95rem;
  }
  .empty .hint {
    margin: 0;
    font-size: 0.85rem;
    max-width: 42ch;
  }
  .empty .cta {
    margin-top: var(--sebas-space-3);
  }

  /* ---- Loading skeletons --------------------------------------------- */
  .skel {
    border-radius: var(--sebas-radius-sm);
    background: linear-gradient(
      90deg,
      var(--sebas-surface-2) 25%,
      var(--sebas-surface-3) 45%,
      var(--sebas-surface-2) 65%
    );
    background-size: 200% 100%;
    animation: sebas-shimmer 1.4s ease-in-out infinite;
  }
  .skel-row {
    display: flex;
    gap: var(--sebas-space-4);
    padding: var(--sebas-space-4);
    border-bottom: 1px solid var(--sebas-border);
  }
  .skel-row:last-child {
    border-bottom: none;
  }
  .skel-line {
    height: 12px;
  }
  @keyframes sebas-shimmer {
    from {
      background-position: 180% 0;
    }
    to {
      background-position: -80% 0;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .skel {
      animation: none;
    }
  }

  /* ---- Callouts ------------------------------------------------------ */
  .callout {
    display: flex;
    align-items: flex-start;
    gap: var(--sebas-space-2);
    padding: var(--sebas-space-3) var(--sebas-space-4);
    border-radius: var(--sebas-radius-md);
    border: 1px solid;
    font-size: 0.875rem;
    margin: 0 0 var(--sebas-space-4);
  }
  .callout svg {
    flex: 0 0 auto;
    margin-top: 2px;
  }
  .callout-error {
    color: var(--sebas-status-failed);
    background: var(--sebas-status-failed-bg);
    border-color: var(--sebas-status-failed-border);
  }
  .callout-warn {
    color: var(--sebas-status-working);
    background: var(--sebas-status-working-bg);
    border-color: var(--sebas-status-working-border);
  }
  .callout-info {
    color: var(--sebas-status-done);
    background: var(--sebas-status-done-bg);
    border-color: var(--sebas-status-done-border);
  }

  /* ---- Chips ---------------------------------------------------------- */
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 3px 10px;
    border-radius: var(--sebas-radius-full);
    border: 1px solid var(--sebas-border);
    background: var(--sebas-surface-2);
    color: var(--sebas-text-dim);
    font-size: 0.78rem;
    white-space: nowrap;
  }
  .chip .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--sebas-text-faint);
  }
  .chip b {
    color: var(--sebas-text-bright);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }

  /* ---- Key/value rows -------------------------------------------------- */
  .kv {
    display: grid;
    grid-template-columns: minmax(140px, 220px) 1fr;
    gap: var(--sebas-space-2) var(--sebas-space-6);
    padding: var(--sebas-space-3) 0;
    border-bottom: 1px solid var(--sebas-border);
    margin: 0;
  }
  .kv:last-child {
    border-bottom: none;
  }
  .kv dt {
    color: var(--sebas-text-dim);
    font-size: 0.875rem;
  }
  .kv dd {
    margin: 0;
    color: var(--sebas-text);
    font-variant-numeric: tabular-nums;
    overflow-wrap: anywhere;
  }
`
