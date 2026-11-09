import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { ApiError, type UsageTimeseries } from '../../api/client.js'
import { UsagePage } from './UsagePage.js'

afterEach(cleanup)

function usage(patch: Partial<UsageTimeseries> = {}): UsageTimeseries {
  const model = {
    model: 'm1',
    requests: 2,
    input_tokens: 100,
    output_tokens: 50,
    cache_read_tokens: 10,
    cache_creation_tokens: 5,
  }
  return {
    granularity: 'day',
    days: 30,
    tz_offset: 0,
    buckets: [{ bucket: '2026-11-01', models: [model] }],
    totals: model,
    ...patch,
  }
}

describe('UsagePage', () => {
  it('renders window totals and per-model readings from the real API shape', async () => {
    const load = vi.fn(async () => usage())
    render(<UsagePage load={load} />)
    await screen.findByText('请求数')
    expect(load).toHaveBeenCalledWith(expect.objectContaining({ granularity: 'day', source: 'all' }))
    expect(screen.getByText('输入 tokens')).toBeTruthy()
    expect(screen.getByText(/m1 · 2 次 · 输入 100/)).toBeTruthy()
    expect(screen.getByRole('img', { name: /按模型分列的 token 用量折线图/ })).toBeTruthy()
  })

  it('reloads when source or granularity changes', async () => {
    const load = vi.fn(async () => usage())
    render(<UsagePage load={load} />)
    await waitFor(() => expect(load).toHaveBeenCalledTimes(1))
    fireEvent.click(screen.getByText('小时'))
    await waitFor(() => expect(load).toHaveBeenLastCalledWith(expect.objectContaining({ granularity: 'hour' })))
    fireEvent.click(screen.getByText('router'))
    await waitFor(() => expect(load).toHaveBeenLastCalledWith(expect.objectContaining({ source: 'router' })))
  })

  it('renders an explicit empty state', async () => {
    const load = vi.fn(async () =>
      usage({
        buckets: [{ bucket: '2026-11-01', models: [] }],
        totals: {
          model: '',
          requests: 0,
          input_tokens: 0,
          output_tokens: 0,
          cache_read_tokens: 0,
          cache_creation_tokens: 0,
        },
      }),
    )
    render(<UsagePage load={load} />)
    expect(await screen.findByText('暂无用量数据')).toBeTruthy()
  })

  it('distinguishes router-unreachable from a generic failure', async () => {
    const load = vi.fn(async () => {
      throw new ApiError(503, 'connection refused', 'router_unreachable')
    })
    render(<UsagePage load={load} />)
    expect((await screen.findByRole('alert')).textContent).toContain('router 不可达')
    expect(screen.getByRole('button', { name: /重\s*试/ })).toBeTruthy()
  })

  it('renders the partial-source notice while keeping local data', async () => {
    const load = vi.fn(async () => usage({ router_cause: 'router closed' }))
    render(<UsagePage load={load} />)
    expect(await screen.findByText(/router 部分不可达/)).toBeTruthy()
    expect(screen.getByText('请求数')).toBeTruthy()
  })
})
