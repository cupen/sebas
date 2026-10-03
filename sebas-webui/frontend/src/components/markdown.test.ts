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

  // （fix-webui-qa-round11 4.1，B-4）单换行在段内逐字保留（breaks:false）：
  // wire 往返的 `\n` 不被管线吃掉，pre-wrap 容器（transcript .body）据此
  // 渲染出真实的行结构——三行提交渲染为三行的管线半边合同。
  it('keeps single newlines inside a paragraph verbatim for pre-wrap containers (round11 4.1)', () => {
    const html = renderMarkdown('键盘行甲\n键盘行乙\n键盘行丙')
    // 段内单换行逐字保留（块间的尾随换行不产生可见空行，无碍）。
    expect(html).toContain('<p>键盘行甲\n键盘行乙\n键盘行丙</p>')
  })
})
