// @vitest-environment jsdom
/**
 * 首启设置视图（add-webui-multiuser-rbac 5.2）：零用户首启的 root 自定义
 * 引导页。三个字段（用户名/密码/确认密码），就地校验优先——缺名、弱密码
 * （<8，与服务端 400 同一阈值）、两次输入不一致都不发请求；服务端 400
 * （弱密码/鉴权关闭）与 409（已有用户）取响应 error 字段就地展示；成功
 * 冒泡 `setup-success`（shell 据此重探身份进入工作台）。
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const apiMocks = vi.hoisted(() => ({
  authSetup: vi.fn(),
}))

vi.mock('../api/client.js', () => ({
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
    authSetup: apiMocks.authSetup,
  },
}))

import './setup-view.js'
import type { SebasSetup } from './setup-view.js'
// mocked 模块里的 ApiError 类——与组件内 instanceof 同一构造器。
import { ApiError } from '../api/client.js'

async function mount(): Promise<SebasSetup> {
  const el = document.createElement('sebas-setup') as SebasSetup
  document.body.appendChild(el)
  await el.updateComplete
  return el
}

function field(el: SebasSetup, name: string): HTMLInputElement {
  return el.shadowRoot!.querySelector<HTMLInputElement>(`input[name="${name}"]`)!
}

function errorText(el: SebasSetup): string {
  return el.shadowRoot!.querySelector<HTMLElement>('.error')?.textContent?.trim() ?? ''
}

/** 填三字段并提交；返回是否发出了请求（供「不发请求」断言）。 */
async function submitAs(
  el: SebasSetup,
  values: { username: string; password: string; confirm: string },
): Promise<void> {
  for (const [name, value] of Object.entries(values)) {
    const input = field(el, name)
    input.value = value
    input.dispatchEvent(new Event('input', { bubbles: true, composed: true }))
  }
  ;(el.shadowRoot!.querySelector('button[type="submit"]') as HTMLButtonElement).click()
  await el.updateComplete
  await new Promise((r) => setTimeout(r, 0))
  await el.updateComplete
}

beforeEach(() => {
  vi.clearAllMocks()
  apiMocks.authSetup.mockResolvedValue({ status: 'ok', username: 'cupen' })
})

afterEach(() => {
  document.body.innerHTML = ''
})

describe('sebas-setup first-run root bootstrap (add-webui-multiuser-rbac 5.2)', () => {
  it('renders username / password / confirm fields', async () => {
    const el = await mount()
    expect(field(el, 'username')).toBeTruthy()
    expect(field(el, 'password')).toBeTruthy()
    expect(field(el, 'confirm')).toBeTruthy()
    el.remove()
  })

  it('submits {username, password} to /api/auth/setup and bubbles setup-success on completion', async () => {
    const el = await mount()
    const success = vi.fn()
    el.addEventListener('setup-success', success)
    await submitAs(el, { username: 'cupen', password: 'long-enough', confirm: 'long-enough' })
    expect(apiMocks.authSetup).toHaveBeenCalledTimes(1)
    expect(apiMocks.authSetup).toHaveBeenCalledWith('cupen', 'long-enough')
    expect(success).toHaveBeenCalledTimes(1)
    expect((success.mock.calls[0][0] as CustomEvent).detail).toEqual({ username: 'cupen' })
    el.remove()
  })

  it('rejects a weak password locally without any request', async () => {
    const el = await mount()
    await submitAs(el, { username: 'cupen', password: 'short', confirm: 'short' })
    expect(apiMocks.authSetup).not.toHaveBeenCalled()
    expect(errorText(el)).toContain('密码至少需要 8 个字符')
    el.remove()
  })

  it('rejects mismatched confirmation locally without any request', async () => {
    const el = await mount()
    await submitAs(el, { username: 'cupen', password: 'long-enough', confirm: 'long-enough-2' })
    expect(apiMocks.authSetup).not.toHaveBeenCalled()
    expect(errorText(el)).toContain('两次输入的密码不一致')
    el.remove()
  })

  it('rejects a missing username locally without any request', async () => {
    const el = await mount()
    await submitAs(el, { username: '   ', password: 'long-enough', confirm: 'long-enough' })
    expect(apiMocks.authSetup).not.toHaveBeenCalled()
    expect(errorText(el)).toContain('请输入用户名')
    el.remove()
  })

  it('shows the server 400 message in place (weak password raced past the client check)', async () => {
    apiMocks.authSetup.mockRejectedValue(new ApiError(400, '密码过短（最少 8 字符）'))
    const el = await mount()
    await submitAs(el, { username: 'cupen', password: 'long-enough', confirm: 'long-enough' })
    expect(errorText(el)).toContain('密码过短')
    el.remove()
  })

  it('shows the 409 conflict in place when users already exist (setup is zero-user only)', async () => {
    apiMocks.authSetup.mockRejectedValue(new ApiError(409, 'users already exist'))
    const el = await mount()
    await submitAs(el, { username: 'cupen', password: 'long-enough', confirm: 'long-enough' })
    expect(errorText(el)).toContain('初始化被拒绝')
    expect(errorText(el)).toContain('users already exist')
    el.remove()
  })
})
