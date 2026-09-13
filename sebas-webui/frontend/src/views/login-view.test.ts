// @vitest-environment jsdom
/**
 * 登录视图（add-webui-multiuser-rbac 5.1）：用户名+密码双字段形态——
 * 旧 `{secret}` 单字段（token 或密码二义）链路已整体移除：
 *   - 提交载荷只含 `{username, password}`（authLogin 双参，绝无 secret 字段）
 *   - 成功冒泡 `login-success`（携带登录响应的 username）
 *   - 401 凭据错 / 429 限速 / 其他 HTTP 失败 / 网络失败，文案就地展示
 * api client 全量 mock（与 settings-modal.test.ts 同款 ApiError 同形类）。
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const apiMocks = vi.hoisted(() => ({
  authLogin: vi.fn(),
}))

vi.mock('../api/client.js', () => ({
  // 与真 ApiError 同形（client.ts）：status + code + count，视图按 status 分支。
  ApiError: class ApiError extends Error {
    readonly status: number
    readonly code: string | null
    readonly count: number | null
    constructor(
      status: number,
      message: string,
      code: string | null = null,
      count: number | null = null,
    ) {
      super(message)
      this.status = status
      this.code = code
      this.count = count
    }
  },
  api: {
    authLogin: apiMocks.authLogin,
  },
}))

import './login-view.js'
import type { SebasLogin } from './login-view.js'
// mocked 模块里的 ApiError 类——与组件内 instanceof 同一构造器。
import { ApiError } from '../api/client.js'

async function mount(): Promise<SebasLogin> {
  const el = document.createElement('sebas-login') as SebasLogin
  document.body.appendChild(el)
  await el.updateComplete
  return el
}

function field(el: SebasLogin, name: string): HTMLInputElement {
  return el.shadowRoot!.querySelector<HTMLInputElement>(`input[name="${name}"]`)!
}

function errorText(el: SebasLogin): string {
  return el.shadowRoot!.querySelector<HTMLElement>('.error')?.textContent?.trim() ?? ''
}

/** 填两字段并提交（jsdom 不做约束校验，required 不拦截显式 submit）。 */
async function submitAs(el: SebasLogin, username: string, password: string): Promise<void> {
  field(el, 'username').value = username
  field(el, 'username').dispatchEvent(new Event('input', { bubbles: true, composed: true }))
  field(el, 'password').value = password
  field(el, 'password').dispatchEvent(new Event('input', { bubbles: true, composed: true }))
  ;(el.shadowRoot!.querySelector('button[type="submit"]') as HTMLButtonElement).click()
  await el.updateComplete
  await new Promise((r) => setTimeout(r, 0))
  await el.updateComplete
}

beforeEach(() => {
  vi.clearAllMocks()
  apiMocks.authLogin.mockResolvedValue({ status: 'ok', username: 'cupen' })
})

afterEach(() => {
  document.body.innerHTML = ''
})

describe('sebas-login two-field form (add-webui-multiuser-rbac 5.1)', () => {
  it('renders separate username and password fields with no single-field secret path', async () => {
    const el = await mount()
    expect(field(el, 'username')).toBeTruthy()
    expect(field(el, 'password')).toBeTruthy()
    expect(field(el, 'password').getAttribute('autocomplete')).toBe('current-password')
    // hintUsername 单字段链路已删：无任何 secret 输入与 hint 绑定。
    expect(el.shadowRoot!.querySelector('input[name="secret"]')).toBeNull()
    expect(el.shadowRoot!.textContent ?? '').not.toContain('Token')
    el.remove()
  })

  it('submits the {username, password} pair and bubbles login-success with the username', async () => {
    const el = await mount()
    const success = vi.fn()
    el.addEventListener('login-success', success)
    await submitAs(el, '  cupen  ', 'long-enough')
    expect(apiMocks.authLogin).toHaveBeenCalledTimes(1)
    expect(apiMocks.authLogin).toHaveBeenCalledWith('cupen', 'long-enough')
    // 载荷是双参（组件内部才拼 {username,password}）；绝无 secret 形态。
    expect(apiMocks.authLogin.mock.calls[0][0]).not.toContain('secret')
    expect(success).toHaveBeenCalledTimes(1)
    expect((success.mock.calls[0][0] as CustomEvent).detail).toEqual({ username: 'cupen' })
    el.remove()
  })

  it('shows the credential error in place on 401', async () => {
    apiMocks.authLogin.mockRejectedValue(new ApiError(401, 'credentials rejected'))
    const el = await mount()
    await submitAs(el, 'cupen', 'wrong')
    expect(errorText(el)).toContain('用户名或密码错误')
    el.remove()
  })

  it('shows the rate-limit hint on 429 without leaking the raw message', async () => {
    apiMocks.authLogin.mockRejectedValue(new ApiError(429, 'too many attempts'))
    const el = await mount()
    await submitAs(el, 'cupen', 'pw')
    expect(errorText(el)).toContain('尝试次数过多')
    expect(errorText(el)).not.toContain('too many attempts')
    el.remove()
  })

  it('shows a generic inline error for other HTTP failures and network errors', async () => {
    apiMocks.authLogin.mockRejectedValue(new ApiError(500, 'boom'))
    const el = await mount()
    await submitAs(el, 'cupen', 'pw')
    expect(errorText(el)).toContain('HTTP 500')
    el.remove()

    apiMocks.authLogin.mockRejectedValue(new TypeError('fetch failed'))
    const el2 = await mount()
    await submitAs(el2, 'cupen', 'pw')
    expect(errorText(el2)).toContain('网络连接失败')
    el2.remove()
  })
})
