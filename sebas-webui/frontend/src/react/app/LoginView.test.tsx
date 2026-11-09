import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { ApiError } from '../../api/client.js'
import { LoginView, loginErrorMessage, validateLogin } from './LoginView.js'

afterEach(cleanup)

describe('validateLogin', () => {
  it('rejects missing fields without calling the API', () => {
    expect(validateLogin('', 'x')).toBe('请输入用户名')
    expect(validateLogin('op', '')).toBe('请输入密码')
    expect(validateLogin('op', 'x')).toBeNull()
  })
})

describe('loginErrorMessage', () => {
  it('maps 401/429 and network failures to Chinese copy', () => {
    expect(loginErrorMessage(new ApiError(401, 'unauthorized'))).toBe('用户名或密码错误')
    expect(loginErrorMessage(new ApiError(429, 'slow down'))).toBe('尝试次数过多，请稍后再试')
    expect(loginErrorMessage(new ApiError(500, 'boom'))).toContain('HTTP 500')
    expect(loginErrorMessage(new Error('offline'))).toBe('网络连接失败，请检查服务是否可用')
  })
})

describe('LoginView', () => {
  it('submits credentials with password-manager hints', async () => {
    const login = vi.fn(async () => ({ username: 'op' }))
    const onSuccess = vi.fn(async () => {})
    render(<LoginView login={login} onSuccess={onSuccess} />)

    expect(screen.getByLabelText('用户名').getAttribute('autocomplete')).toBe('username')
    expect(screen.getByLabelText('密码').getAttribute('autocomplete')).toBe('current-password')

    fireEvent.change(screen.getByLabelText('用户名'), { target: { value: ' op ' } })
    fireEvent.change(screen.getByLabelText('密码'), { target: { value: 'secret' } })
    fireEvent.click(screen.getByRole('button', { name: /登\s*录/ }))

    await waitFor(() => expect(login).toHaveBeenCalledWith('op', 'secret'))
    expect(onSuccess).toHaveBeenCalled()
  })

  it('shows inline validation and the mapped API error', async () => {
    const login = vi.fn(async () => {
      throw new ApiError(401, 'unauthorized')
    })
    render(<LoginView login={login} />)

    fireEvent.click(screen.getByRole('button', { name: /登\s*录/ }))
    expect((await screen.findByRole('alert')).textContent).toContain('请输入用户名')
    expect(login).not.toHaveBeenCalled()

    fireEvent.change(screen.getByLabelText('用户名'), { target: { value: 'op' } })
    fireEvent.change(screen.getByLabelText('密码'), { target: { value: 'bad' } })
    fireEvent.click(screen.getByRole('button', { name: /登\s*录/ }))

    await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('用户名或密码错误'))
  })
})
