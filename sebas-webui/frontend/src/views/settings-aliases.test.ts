// @vitest-environment jsdom
// 模型别名管理分区（fix-webui-qa-round8 6.1，router-model-aliases「模型
// 别名管理有 WebUI 入口」）：列表读形、新建、编辑、删除+确认、错误就地
// 呈现——全部经 mock 的既有 `/api/model-aliases` CRUD。
import { beforeEach, afterEach, describe, expect, it, vi } from 'vitest'
import { installWaDomPolyfills } from '../test-support/wa-polyfills.js'

installWaDomPolyfills()

const apiMocks = vi.hoisted(() => ({
  providers: vi.fn(),
  aliasCreate: vi.fn(),
  aliasUpdate: vi.fn(),
  aliasDelete: vi.fn(),
}))

vi.mock('../api/client.js', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/client.js')>()
  return {
    ...actual,
    ApiError: actual.ApiError,
    api: {
      ...actual.api,
      providers: apiMocks.providers,
      aliasCreate: apiMocks.aliasCreate,
      aliasUpdate: apiMocks.aliasUpdate,
      aliasDelete: apiMocks.aliasDelete,
    },
  }
})

import './settings-aliases.js'
import type { SebasModelAliases } from './settings-aliases.js'

const snapshot = {
  providers: [
    { name: 'deepseek' },
    { name: 'anthropic' },
  ],
  config_providers: [],
  model_aliases: {
    deep: { provider: 'deepseek', upstream_model: 'deepseek-chat' },
    claude: { provider: 'anthropic' },
  },
}

async function mount(): Promise<SebasModelAliases> {
  const el = document.createElement('sebas-model-aliases') as SebasModelAliases
  document.body.appendChild(el)
  await el.updateComplete
  await new Promise((r) => setTimeout(r, 0))
  await el.updateComplete
  return el
}

function rows(el: SebasModelAliases): HTMLElement[] {
  return [...el.shadowRoot!.querySelectorAll<HTMLElement>('[data-testid="alias-row"]')]
}

beforeEach(() => {
  vi.clearAllMocks()
  apiMocks.providers.mockResolvedValue(JSON.parse(JSON.stringify(snapshot)))
  apiMocks.aliasCreate.mockResolvedValue({ created: 'x' })
  apiMocks.aliasUpdate.mockResolvedValue({ updated: 'x' })
  apiMocks.aliasDelete.mockResolvedValue({ deleted: 'x' })
})

afterEach(() => {
  document.body.innerHTML = ''
})

describe('sebas-model-aliases (router-model-aliases 别名管理 WebUI 入口)', () => {
  it('renders the alias list from the providers read model, sorted by name', async () => {
    const el = await mount()
    const list = el.shadowRoot!.querySelector('[data-testid="alias-list"]')!
    expect(list).toBeTruthy()
    const names = rows(el).map((r) => r.dataset.alias)
    expect(names).toEqual(['claude', 'deep'])
    const deep = rows(el).find((r) => r.dataset.alias === 'deep')!
    expect(deep.textContent).toContain('deepseek')
    expect(deep.textContent).toContain('deepseek-chat')
    // 无上游模型覆写的条目不显示上游段。
    expect(rows(el).find((r) => r.dataset.alias === 'claude')!.textContent).not.toContain('·')
    el.remove()
  })

  it('creates an alias through the existing /api/model-aliases CRUD and refreshes', async () => {
    const el = await mount()
    el.shadowRoot!.querySelector<HTMLElement>('[data-testid="alias-create"]')!.click()
    await el.updateComplete
    const nameInput = el.shadowRoot!.querySelector<HTMLInputElement>('[data-testid="alias-name-input"]')!
    nameInput.value = 'ds'
    nameInput.dispatchEvent(new Event('input'))
    await el.updateComplete
    el.shadowRoot!.querySelector<HTMLElement>('[data-testid="alias-save"]')!.click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    expect(apiMocks.aliasCreate).toHaveBeenCalledWith('ds', 'deepseek', undefined)
    el.remove()
  })

  it('edits an alias in place (name is the primary key: input disabled)', async () => {
    const el = await mount()
    const deep = rows(el).find((r) => r.dataset.alias === 'deep')!
    deep.querySelector<HTMLElement>('button[aria-label="编辑别名 deep"]')!.click()
    await el.updateComplete
    const nameInput = el.shadowRoot!.querySelector<HTMLInputElement>('[data-testid="alias-name-input"]')!
    expect(nameInput.hasAttribute('disabled')).toBe(true)
    el.shadowRoot!.querySelector<HTMLElement>('[data-testid="alias-save"]')!.click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    expect(apiMocks.aliasUpdate).toHaveBeenCalledWith('deep', 'deepseek', 'deepseek-chat')
    el.remove()
  })

  it('delete requires an explicit confirm step', async () => {
    const el = await mount()
    rows(el)[0]!.querySelector<HTMLElement>('button[aria-label="删除别名 claude"]')!.click()
    await el.updateComplete
    // 确认弹窗在场；未点确认前 API 未被调用。
    expect(apiMocks.aliasDelete).not.toHaveBeenCalled()
    const confirm = el.shadowRoot!.querySelector<HTMLElement>('[data-testid="alias-delete-confirm"]')!
    confirm.click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    expect(apiMocks.aliasDelete).toHaveBeenCalledWith('claude')
    el.remove()
  })

  it('a failed create stays in the editor with the server reason inline', async () => {
    apiMocks.aliasCreate.mockRejectedValue(
      new (await import('../api/client.js')).ApiError(409, "别名 'ds' 已存在"),
    )
    const el = await mount()
    el.shadowRoot!.querySelector<HTMLElement>('[data-testid="alias-create"]')!.click()
    await el.updateComplete
    const nameInput = el.shadowRoot!.querySelector<HTMLInputElement>('[data-testid="alias-name-input"]')!
    nameInput.value = 'ds'
    nameInput.dispatchEvent(new Event('input'))
    await el.updateComplete
    el.shadowRoot!.querySelector<HTMLElement>('[data-testid="alias-save"]')!.click()
    await new Promise((r) => setTimeout(r, 0))
    await el.updateComplete
    const errorBox = el.shadowRoot!.querySelector('[data-testid="alias-editor-error"]')
    expect(errorBox?.textContent).toContain('已存在')
    // 编辑器未关闭（失败不假装成功）。
    expect(el.shadowRoot!.querySelector('[data-testid="alias-save"]')).toBeTruthy()
    el.remove()
  })
})
