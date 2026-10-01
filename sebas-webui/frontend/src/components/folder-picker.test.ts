/**
 * joinChildPath unit tests (add-webui-picker-workdir-start): the picker's
 * child-path join must strip trailing separators of either flavor before
 * appending `/` — the Windows-echoed form ends with `\`, and the old
 * `replace(/\/$/, '')`-then-join produced `\\?\D:\dir\/sub`, which the
 * backend could not resolve (400 on every expand).
 *
 * （add-webui-round7-gaps 3.2）「新建文件夹」的组件级单测：内联命名 →
 * `POST /api/fs/mkdir` → 当前节点局部刷新 → 新目录在树内可见并选中
 * （folder-selected 照发，复用点零改动）；拒绝路径内联呈现且不发必败请求。
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { api, type FsBrowseResponse } from '../api/client.js'
import { joinChildPath, validateFolderName } from './folder-picker.js'
import './folder-picker.js'
import type { SebasFolderPicker } from './folder-picker.js'

// ---- api 局部 mock（保留真实 errorText——错误呈现本身是被测行为之一）----

vi.mock('../api/client.js', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/client.js')>()
  return {
    ...actual,
    api: {
      ...actual.api,
      fsBrowseDirs: vi.fn(),
      fsMkdir: vi.fn(),
    },
  }
})

const mockOf = <F>(fn: F): ReturnType<typeof vi.fn> & F =>
  fn as unknown as ReturnType<typeof vi.fn> & F

const apiMock = vi.mocked(api)

function listing(path: string, names: string[]): FsBrowseResponse {
  return { path, entries: names.map((name) => ({ name, is_dir: true, has_subdirs: false })) }
}

describe('joinChildPath', () => {
  it('strips a trailing backslash before joining (Windows verbatim echo)', () => {
    expect(joinChildPath('C:\\Users\\dev', 'repo')).toBe('C:\\Users\\dev/repo')
  })

  it('strips a trailing slash', () => {
    expect(joinChildPath('/home/dev/', 'repo')).toBe('/home/dev/repo')
  })

  it('strips runs of trailing separators', () => {
    expect(joinChildPath('D:\\a\\\\', 'x')).toBe('D:\\a/x')
  })

  it('keeps interior separators and the verbatim prefix untouched', () => {
    expect(joinChildPath('\\\\?\\D:\\root', 'a b')).toBe('\\\\?\\D:\\root/a b')
  })
})

describe('validateFolderName（与服务端 mkdir 同一规则的本地预检）', () => {
  it('legal single-segment names pass (trim included)', () => {
    expect(validateFolderName('projects')).toBeNull()
    expect(validateFolderName('  我的项目  ')).toBeNull()
  })

  it('empty / dot forms / separators / NUL are rejected with Chinese copy', () => {
    expect(validateFolderName('')).toContain('不能为空')
    expect(validateFolderName('   ')).toContain('不能为空')
    expect(validateFolderName('.')).toContain('「.」')
    expect(validateFolderName('..')).toContain('「..」')
    expect(validateFolderName('a/b')).toContain('分隔符')
    expect(validateFolderName('a\\b')).toContain('分隔符')
    expect(validateFolderName('a\0b')).toContain('非法字符')
  })
})

describe('新建文件夹（add-webui-round7-gaps 3.2，webui/projects spec）', () => {
  async function mountPicker(): Promise<SebasFolderPicker> {
    const el = document.createElement('sebas-folder-picker') as SebasFolderPicker
    document.body.appendChild(el)
    await el.updateComplete
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    return el
  }

  function q(el: SebasFolderPicker, sel: string): HTMLElement | null {
    return el.shadowRoot?.querySelector(sel) ?? null
  }

  function treePaths(el: SebasFolderPicker): string[] {
    return [...(el.shadowRoot?.querySelectorAll('wa-tree-item') ?? [])].map(
      (n) => (n as HTMLElement).dataset.path ?? '',
    )
  }

  beforeEach(() => {
    vi.clearAllMocks()
  })

  afterEach(() => {
    document.body.querySelectorAll('sebas-folder-picker').forEach((n) => n.remove())
  })

  it('loaded 后出现「新建文件夹」入口；点击展开内联命名行', async () => {
    mockOf(apiMock.fsBrowseDirs).mockResolvedValue(listing('/ws', ['alpha']))
    const el = await mountPicker()
    const toggle = q(el, '[data-testid="mkdir-toggle"]')
    expect(toggle).toBeTruthy()
    expect(toggle!.textContent).toContain('新建文件夹')
    expect(q(el, '[data-testid="mkdir-name"]')).toBeNull()
    toggle!.click()
    await el.updateComplete
    expect(q(el, '[data-testid="mkdir-name"]')).toBeTruthy()
    el.remove()
  })

  it('成功路径：mkdir 落到当前根目录，局部刷新后新目录在树内可见并选中', async () => {
    // 首拉无新目录；创建后的重拉（局部刷新）带上新条目。
    mockOf(apiMock.fsBrowseDirs)
      .mockResolvedValueOnce(listing('/ws', ['alpha']))
      .mockResolvedValueOnce(listing('/ws', ['alpha', 'NewDir']))
    mockOf(apiMock.fsMkdir).mockResolvedValue({ path: '/ws', name: 'NewDir', created: true })
    const el = await mountPicker()
    expect(treePaths(el)).toEqual(['/ws/alpha'])

    q(el, '[data-testid="mkdir-toggle"]')!.click()
    await el.updateComplete
    const input = q(el, '[data-testid="mkdir-name"]') as HTMLInputElement
    input.value = 'NewDir'
    input.dispatchEvent(new Event('input', { bubbles: true }))
    await el.updateComplete

    const selected: string[] = []
    el.addEventListener('folder-selected', (e) => selected.push((e as CustomEvent).detail.path))
    q(el, '[data-testid="mkdir-confirm"]')!.click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete

    expect(mockOf(apiMock.fsMkdir).mock.calls).toEqual([['/ws', 'NewDir']])
    // 局部刷新发生了（重拉根列表），新目录在树内可见。
    expect(mockOf(apiMock.fsBrowseDirs).mock.calls.length).toBeGreaterThanOrEqual(2)
    expect(treePaths(el)).toContain('/ws/NewDir')
    // 选中新目录并照发 folder-selected（复用点免手工刷新即可继续）。
    expect(el.selectedPath).toBe('/ws/NewDir')
    expect(selected).toEqual(['/ws/NewDir'])
    // 内联行收起。
    expect(q(el, '[data-testid="mkdir-name"]')).toBeNull()
    el.remove()
  })

  it('空名本地拒绝：不发请求、内联中文错误', async () => {
    mockOf(apiMock.fsBrowseDirs).mockResolvedValue(listing('/ws', []))
    const el = await mountPicker()
    q(el, '[data-testid="mkdir-toggle"]')!.click()
    await el.updateComplete
    q(el, '[data-testid="mkdir-confirm"]')!.click()
    await el.updateComplete
    expect(mockOf(apiMock.fsMkdir).mock.calls).toHaveLength(0)
    const err = q(el, '[data-testid="mkdir-error"]')
    expect(err?.textContent).toContain('目录名不能为空')
    el.remove()
  })

  it('服务端类型化拒绝（越界/同名等 400）内联呈现，输入行不收起', async () => {
    mockOf(apiMock.fsBrowseDirs).mockResolvedValue(listing('/ws', ['alpha']))
    mockOf(apiMock.fsMkdir).mockRejectedValue(new Error('同名目录已存在: alpha'))
    const el = await mountPicker()
    q(el, '[data-testid="mkdir-toggle"]')!.click()
    await el.updateComplete
    const input = q(el, '[data-testid="mkdir-name"]') as HTMLInputElement
    input.value = 'alpha'
    input.dispatchEvent(new Event('input', { bubbles: true }))
    await el.updateComplete
    q(el, '[data-testid="mkdir-confirm"]')!.click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    const err = q(el, '[data-testid="mkdir-error"]')
    expect(err?.textContent).toContain('同名目录已存在: alpha')
    // 输入行保持展开，操作者可直接改名重试。
    expect(q(el, '[data-testid="mkdir-name"]')).toBeTruthy()
    expect(el.selectedPath).toBe('')
    el.remove()
  })

  it('取消收起内联行并清空错误', async () => {
    mockOf(apiMock.fsBrowseDirs).mockResolvedValue(listing('/ws', []))
    const el = await mountPicker()
    q(el, '[data-testid="mkdir-toggle"]')!.click()
    await el.updateComplete
    q(el, '[data-testid="mkdir-confirm"]')!.click()
    await el.updateComplete
    expect(q(el, '[data-testid="mkdir-error"]')).toBeTruthy()
    q(el, '[data-testid="mkdir-cancel"]')!.click()
    await el.updateComplete
    expect(q(el, '[data-testid="mkdir-name"]')).toBeNull()
    // 重新打开：错误已清空。
    q(el, '[data-testid="mkdir-toggle"]')!.click()
    await el.updateComplete
    expect(q(el, '[data-testid="mkdir-error"]')).toBeNull()
    el.remove()
  })
})
