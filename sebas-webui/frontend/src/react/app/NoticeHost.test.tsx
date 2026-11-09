import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { notify, resetNotices, setFatal, setWsDown } from '../../notify.js'
import { NoticeHost } from './NoticeHost.js'

afterEach(() => {
  cleanup()
  act(() => resetNotices())
})

describe('NoticeHost', () => {
  it('renders global toasts and dismisses them', () => {
    render(<NoticeHost onRetry={() => {}} />)
    act(() => {
      notify({ level: 'warn', message: '保存失败' })
    })
    expect(screen.getByText('保存失败')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: /close/i }))
    expect(screen.queryByText('保存失败')).toBeNull()
  })

  it('shows the reconnect banner for a dropped websocket', () => {
    render(<NoticeHost onRetry={() => {}} />)
    act(() => setWsDown(true))
    expect(screen.getByRole('status').textContent).toContain('服务器连接已断开')
    act(() => setWsDown(false))
    expect(screen.queryByRole('status')).toBeNull()
  })

  it('locks the workbench on fatal and retries explicitly', () => {
    const onRetry = vi.fn()
    render(<NoticeHost onRetry={onRetry} />)
    act(() => setFatal({ kind: 'startup_failed', cause: 'exec failed' }))
    expect(screen.getByRole('alertdialog').textContent).toContain('核心启动失败：exec failed')
    expect(screen.queryByRole('status')).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: /重试连接/ }))
    expect(onRetry).toHaveBeenCalled()
    act(() => setFatal(null))
    expect(screen.queryByRole('alertdialog')).toBeNull()
  })
})
