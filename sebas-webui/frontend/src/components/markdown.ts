/**
 * Markdown rendering pipeline: marked (parse) → DOMPurify (sanitize) →
 * highlight.js (code blocks). SSR used to auto-escape everything; in the
 * SPA, sanitization is mandatory before `unsafeHTML` — untrusted markdown
 * must never yield script or event-handler attributes.
 */

import { marked } from 'marked'
import DOMPurify from 'dompurify'
import hljs from 'highlight.js/lib/common'

marked.setOptions({
  gfm: true,
  breaks: false,
})

// DOMPurify must bind to a live window; importing the default export alone
// leaves it unbundled in environments (happy-dom) where window loads late.
const purify = DOMPurify(window)

/**
 * Optional measurement seam for the deterministic performance harness
 * (4.6). Inert in production: `null` costs one branch per parse.
 */
export interface MarkdownObserver {
  parse(): void
  sanitize(): void
  highlight(): void
}

let observer: MarkdownObserver | null = null

export function setMarkdownObserver(next: MarkdownObserver | null): void {
  observer = next
}

/** Render untrusted markdown to sanitized, highlighted HTML. */
export function renderMarkdown(source: string): string {
  observer?.parse()
  const raw = marked.parse(source, { async: false })
  const clean = purify.sanitize(raw, {
    FORBID_TAGS: ['script', 'style', 'iframe', 'form', 'object', 'embed'],
    FORBID_ATTR: ['onerror', 'onclick', 'onload', 'onmouseover', 'onfocus'],
  })
  observer?.sanitize()
  return wrapTables(highlightIn(clean))
}

/**
 * Give wide GFM tables their own horizontal scroll container. Without this
 * the transcript pane scrolls as a whole (blueprint 4.1 requires overflow
 * inside the entry) and reading one table moves every other row.
 */
function wrapTables(html: string): string {
  const template = document.createElement('template')
  template.innerHTML = html
  template.content.querySelectorAll('table').forEach((table) => {
    if (table.parentElement?.classList.contains('sebas-table-scroll')) return
    const wrapper = document.createElement('div')
    wrapper.className = 'sebas-table-scroll'
    table.replaceWith(wrapper)
    wrapper.appendChild(table)
  })
  return template.innerHTML
}

/** Apply highlight.js to code blocks inside an already-sanitized fragment. */
function highlightIn(html: string): string {
  const template = document.createElement('template')
  template.innerHTML = html
  template.content.querySelectorAll('pre code').forEach((el) => {
    // An existing language class wins; otherwise hljs autodetects.
    const existing = Array.from(el.classList).find((c) => c.startsWith('language-'))
    if (existing) {
      try {
        // biome-ignore lint: hljs may not know every language tag
        observer?.highlight()
        const result = hljs.highlight(el.textContent ?? '', {
          language: existing.slice('language-'.length),
          ignoreIllegals: true,
        })
        el.innerHTML = result.value
        return
      } catch {
        // fall through to autodetection
      }
    }
    observer?.highlight()
    el.innerHTML = hljs.highlightAuto(el.textContent ?? '').value
  })
  return template.innerHTML
}
