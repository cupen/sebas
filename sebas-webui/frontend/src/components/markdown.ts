/**
 * Markdown rendering pipeline: marked (parse) → DOMPurify (sanitize) →
 * highlight.js (code blocks). SSR used to auto-escape everything; in the
 * SPA, sanitization is mandatory before `unsafeHTML` — untrusted markdown
 * must never yield script or event-handler attributes.
 */

import { marked } from 'marked'
import DOMPurify from 'dompurify'
import hljs from 'highlight.js'

marked.setOptions({
  gfm: true,
  breaks: false,
})

// DOMPurify must bind to a live window; importing the default export alone
// leaves it unbundled in environments (happy-dom) where window loads late.
const purify = DOMPurify(window)

/** Render untrusted markdown to sanitized, highlighted HTML. */
export function renderMarkdown(source: string): string {
  const raw = marked.parse(source, { async: false })
  const clean = purify.sanitize(raw, {
    FORBID_TAGS: ['script', 'style', 'iframe', 'form', 'object', 'embed'],
    FORBID_ATTR: ['onerror', 'onclick', 'onload', 'onmouseover', 'onfocus'],
  })
  return highlightIn(clean)
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
    el.innerHTML = hljs.highlightAuto(el.textContent ?? '').value
  })
  return template.innerHTML
}
