import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { ApiError } from '../../api/client.js'
import { MIN_PASSWORD_LENGTH, SetupView, setupErrorMessage, validateSetup } from './SetupView.js'

afterEach(cleanup)

describe('validateSetup', () => {
  it('checks username, minimum length and confirmation', () => {
    expect(validateSetup('', 'x'.repeat(8), 'x'.repeat(8))).toBe('请输入用户名')
    expect(validateSetup('root', 'short', 'short')).toBe(`密码至少需要 ${MIN_PASSWORD_LENGTH} 个字符`)
    expect(validateSetup('root', 'long-enough', 'other')).toBe('两次输入的密码不一致')
    expect(validateSetup('root', 'long-enough', 'long-enough')).toBeNull()
  })
})

describe('setupErrorMessage', () => {
  it('prefixes 409 conflicts and passes other messages through', () => {
    expect(setupErrorMessage(new ApiError(409, 'user exists'))).toBe('初始化被拒绝：user exists')
    expect(setupErrorMessage(new ApiError(400, 'weak password'))).toBe('weak password')
    expect(setupErrorMessage(new Error('offline'))).toContain('网络连接失败')
  })
})

describe('SetupView', () => {
  it('creates the root account with new-password autocomplete', async () => {
    const setup = vi.fn(async () => ({ username: 'root' }))
    const onSuccess = vi.fn(async () => {})
    render(<SetupView setup={setup} onSuccess={onSuccess} />)

    expect(screen.getByLabelText('密码（至少 8 个字符）').getAttribute('autocomplete')).toBe(
      'new-password',
    )
    fireEvent.change(screen.getByLabelText('用户名'), { target: { value: 'root' } })
    fireEvent.change(screen.getByLabelText('密码（至少 8 个字符）'), {
      target: { value: 'long-enough' },
    })
    fireEvent.change(screen.getByLabelText('确认密码'), { target: { value: 'long-enough' } })
    fireEvent.click(screen.getByRole('button', { name: '创建并进入' }))

    await waitFor(() => expect(setup).toHaveBeenCalledWith('root', 'long-enough'))
    expect(onSuccess).toHaveBeenCalled()
  })

  it('blocks a mismatched confirmation before any request', async () => {
    const setup = vi.fn()
    render(<SetupView setup={setup} />)
    fireEvent.change(screen.getByLabelText('用户名'), { target: { value: 'root' } })
    fireEvent.change(screen.getByLabelText('密码（至少 8 个字符）'), {
      target: { value: 'long-enough' },
    })
    fireEvent.change(screen.getByLabelText('确认密码'), { target: { value: 'different' } })
    fireEvent.click(screen.getByRole('button', { name: '创建并进入' }))
    expect((await screen.findByRole('alert')).textContent).toContain('两次输入的密码不一致')
    expect(setup).not.toHaveBeenCalled()
  })
})
