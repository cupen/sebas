// @vitest-environment jsdom
// DOMPurify officially supports browser-grade DOM; jsdom matches it far
// more closely than happy-dom, so the security-critical sanitizer tests
// run under jsdom.
import { describe, expect, it } from 'vitest'
import { renderMarkdown } from './markdown.js'

describe('renderMarkdown', () => {
  it('renders ordinary markdown', () => {
    const html = renderMarkdown('# Title\n\nsome **bold** text')
    expect(html).toContain('<h1>Title</h1>')
    expect(html).toContain('<strong>bold</strong>')
  })

  it('strips script tags from untrusted markdown', () => {
    const html = renderMarkdown('hello <script>alert(1)</script> world')
    expect(html).not.toContain('<script')
    expect(html).not.toContain('alert(1)')
  })

  it('strips event handler attributes (the XSS escape hatch)', () => {
    const html = renderMarkdown('<img src=x onerror="alert(1)">')
    expect(html).not.toContain('onerror')
  })

  it('strips javascript: URLs and iframes', () => {
    const html = renderMarkdown('<iframe src="javascript:alert(1)"></iframe>')
    expect(html).not.toContain('iframe')
    expect(html).not.toContain('javascript:')
  })

  it('highlights fenced code blocks', () => {
    const html = renderMarkdown('```rust\nfn main() {}\n```\n')
    expect(html).toContain('<code')
    expect(html).toMatch(/class="[^"]*hljs/)
  })

  it('keeps inline code and links', () => {
    const html = renderMarkdown('run `npm test` and see [docs](https://example.com)')
    expect(html).toContain('<code>npm test</code>')
    expect(html).toContain('<a href="https://example.com"')
  })
})
