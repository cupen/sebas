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

async function mount(role: 'root' | 'admin' | 'member' | 'viewer' | null = null): Promise<SebasModelAliases> {
  const el = document.createElement('sebas-model-aliases') as SebasModelAliases
  if (role !== null) el.role = role
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

  // ── fix-webui-qa-round10 3.2（C-DEF-02）：别名写控件随角色裁剪 ──
  // spec「Member cannot mutate a provider … the settings UI does not offer
  // the mutation controls to that member」对别名面同样成立：settings.manage
  // 档（root/admin，及鉴权关闭宿主）才呈现新建/编辑/删除；member/viewer
  // 只读浏览（列表保留），防线在服务端 403。

  it('member/viewer see no write controls; the list stays browsable', async () => {
    for (const role of ['member', 'viewer'] as const) {
      const el = await mount(role)
      expect(el.shadowRoot!.querySelector('[data-testid="alias-create"]'), `${role} 不得见新建`).toBeNull()
      expect(el.shadowRoot!.querySelector('[data-testid="alias-list"]'), `${role} 列表保留`).toBeTruthy()
      expect(rows(el).length).toBe(2)
      for (const row of rows(el)) {
        expect(row.querySelector('button[title="编辑"]'), `${role} 不得见编辑`).toBeNull()
        expect(row.querySelector('button[title="删除"]'), `${role} 不得见删除`).toBeNull()
      }
      el.remove()
    }
  })

  it('root/admin (and auth-disabled hosts) keep the full write surface', async () => {
    for (const role of ['root', 'admin', null] as const) {
      const el = await mount(role)
      expect(el.shadowRoot!.querySelector('[data-testid="alias-create"]'), `${role} 新建在`).toBeTruthy()
      const row = rows(el)[0]!
      expect(row.querySelector('button[title="编辑"]'), `${role} 编辑在`).toBeTruthy()
      expect(row.querySelector('button[title="删除"]'), `${role} 删除在`).toBeTruthy()
      el.remove()
    }
  })

  // ── fix-webui-qa-round11 4.3（A-2）：零可选 provider 的空态指引 ──
  // spec「Alias creation guides when no provider target exists」：下拉禁用
  // + 指引文案（指向「模型」分区），不呈现静默空下拉；保存键同禁（表单
  // 不允许提交无目标的别名）。
  describe('alias form guides when no provider target exists (round11 4.3)', () => {
    function emptyStoreSnapshot() {
      return { providers: [], config_providers: [], model_aliases: {} }
    }

    async function openCreate(el: SebasModelAliases): Promise<void> {
      el.shadowRoot!.querySelector<HTMLElement>('[data-testid="alias-create"]')!.click()
      await el.updateComplete
    }

    it('empty dropdown renders disabled with guidance pointing to the models section; save is blocked', async () => {
      apiMocks.providers.mockResolvedValue(emptyStoreSnapshot())
      const el = await mount()
      await openCreate(el)
      const select = el.shadowRoot!.querySelector<HTMLInputElement>('[data-testid="alias-provider-select"]')!
      expect(select, '空态下拉在场').toBeTruthy()
      expect(select.hasAttribute('disabled'), '下拉禁用态').toBe(true)
      const hint = el.shadowRoot!.querySelector('[data-testid="alias-provider-empty-hint"]')!
      expect(hint.textContent).toContain('模型')
      expect(hint.textContent).toContain('provider')
      // 保存键禁用：无目标不可提交（CRUD 零调用）。
      const save = el.shadowRoot!.querySelector<HTMLButtonElement>('[data-testid="alias-save"]')!
      expect((save as unknown as { disabled: boolean }).disabled).toBe(true)
      save.click()
      await new Promise((r) => setTimeout(r, 0))
      await el.updateComplete
      expect(apiMocks.aliasCreate).not.toHaveBeenCalled()
      el.remove()
    })

    it('creating a provider unlocks the form: the dropdown offers the new target', async () => {
      apiMocks.providers.mockResolvedValue(emptyStoreSnapshot())
      const el = await mount()
      await openCreate(el)
      expect(el.shadowRoot!.querySelector('[data-testid="alias-provider-empty-hint"]')).toBeTruthy()
      el.remove()

      // 操作者在「模型」分区建了 provider 后重进别名分区 = 新元素重挂
      // （connectedCallback 重读 providers）→ 下拉解锁、可选中新行。
      apiMocks.providers.mockResolvedValue({
        providers: [{ name: 'qa-provider' }],
        config_providers: [],
        model_aliases: {},
      })
      const el2 = await mount()
      await openCreate(el2)
      expect(el2.shadowRoot!.querySelector('[data-testid="alias-provider-empty-hint"]')).toBeNull()
      const select = el2.shadowRoot!.querySelector<HTMLInputElement>('[data-testid="alias-provider-select"]')!
      expect(select.hasAttribute('disabled')).toBe(false)
      // 正常提交走通（目标的解锁面）。
      const nameInput = el2.shadowRoot!.querySelector<HTMLInputElement>('[data-testid="alias-name-input"]')!
      nameInput.value = 'qa'
      nameInput.dispatchEvent(new Event('input'))
      await el2.updateComplete
      el2.shadowRoot!.querySelector<HTMLElement>('[data-testid="alias-save"]')!.click()
      await new Promise((r) => setTimeout(r, 0))
      await el2.updateComplete
      expect(apiMocks.aliasCreate).toHaveBeenCalledWith('qa', 'qa-provider', undefined)
      el2.remove()
    })
  })
})
