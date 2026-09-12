// @vitest-environment jsdom
/**
 * toModelCatalog（workbench-conversation-view 4.2，design D7）: the SPA-side
 * adapter seam. Covered inputs:
 *   - empty catalog (no providers / no models)
 *   - a provider without models contributes nothing
 *   - configured defaults absent from the catalog pass through untouched
 *   - the healthy path flattens pairs in payload order
 */

import { describe, expect, it, vi } from 'vitest'
import type { RouterProviderAdmin } from './client.js'

// loadModelCatalog 走真实 api 客户端——mock 掉 fetch 面（routerProviders /
// routerDefaults）后导入被测模块。
vi.mock('./client.js', () => ({
  api: {
    routerProviders: vi.fn(),
    routerDefaults: vi.fn(),
  },
}))

import { api } from './client.js'
import {
  loadModelCatalog,
  preselectFromCatalog,
  groupSessionModels,
  SESSION_PROVIDED_GROUP_LABEL,
  toModelCatalog,
} from './model-catalog.js'

function provider(name: string, models: RouterProviderAdmin['models']): RouterProviderAdmin {
  return {
    name,
    base_url_anthropic: null,
    base_url_openai_chat: null,
    base_url_openai_responses: null,
    api_key_env: null,
    api_key_configured: true,
    models,
  }
}

describe('toModelCatalog', () => {
  it('flattens providers × models into pairs in payload order', () => {
    const catalog = toModelCatalog(
      [
        provider('alpha', [{ id: 'a1', tags: [] }, { id: 'a2', tags: ['vision'] }]),
        provider('beta', [{ id: 'b1', tags: [] }]),
      ],
      { default_provider: 'beta', default_model: 'b1' },
    )
    expect(catalog.pairs).toEqual([
      { provider: 'alpha', model: 'a1' },
      { provider: 'alpha', model: 'a2' },
      { provider: 'beta', model: 'b1' },
    ])
    expect(catalog.defaultProvider).toBe('beta')
    expect(catalog.defaultModel).toBe('b1')
  })

  it('an empty catalog yields no pairs and keeps the defaults fields', () => {
    const catalog = toModelCatalog([], { default_provider: null, default_model: null })
    expect(catalog.pairs).toEqual([])
    expect(catalog.defaultProvider).toBeNull()
    expect(catalog.defaultModel).toBeNull()
  })

  it('a provider without models contributes no pairs (nothing fabricated)', () => {
    const catalog = toModelCatalog(
      [provider('empty', []), provider('full', [{ id: 'm', tags: [] }])],
      null,
    )
    expect(catalog.pairs).toEqual([{ provider: 'full', model: 'm' }])
  })

  it('defaults absent from the catalog pass through untouched (caller decides)', () => {
    const catalog = toModelCatalog(
      [provider('alpha', [{ id: 'a1', tags: [] }])],
      { default_provider: 'ghost', default_model: 'nope' },
    )
    expect(catalog.pairs).toEqual([{ provider: 'alpha', model: 'a1' }])
    // The adapter never drops or rewrites the configured defaults — the
    // selector decides whether they can preselect (they cannot here).
    expect(catalog.defaultProvider).toBe('ghost')
    expect(catalog.defaultModel).toBe('nope')
  })

  it('model ids pass through verbatim — no interpretation of the structure', () => {
    const catalog = toModelCatalog(
      [provider('p', [{ id: 'weird [1m] id', tags: ['vision', 'audio'] }])],
      null,
    )
    expect(catalog.pairs[0].model).toBe('weird [1m] id')
  })
})

// ─── workbench-interaction-polish 2.1/D2/D3：共用加载、预选与分组 ─────────────

describe('loadModelCatalog', () => {
  it('fetches providers + defaults in parallel and reports a usable catalog', async () => {
    ;(api.routerProviders as ReturnType<typeof vi.fn>).mockResolvedValue({
      providers: [provider('alpha', [{ id: 'a1', tags: [] }])],
    })
    ;(api.routerDefaults as ReturnType<typeof vi.fn>).mockResolvedValue({
      default_provider: 'alpha',
      default_model: 'a1',
    })
    const { catalog, unavailable } = await loadModelCatalog()
    expect(unavailable).toBe(false)
    expect(catalog?.pairs).toEqual([{ provider: 'alpha', model: 'a1' }])
    expect(catalog?.defaultProvider).toBe('alpha')
  })

  it('defaults fetch failing does not sink the catalog', async () => {
    ;(api.routerProviders as ReturnType<typeof vi.fn>).mockResolvedValue({
      providers: [provider('alpha', [{ id: 'a1', tags: [] }])],
    })
    ;(api.routerDefaults as ReturnType<typeof vi.fn>).mockRejectedValue(new Error('503'))
    const { catalog, unavailable } = await loadModelCatalog()
    expect(unavailable).toBe(false)
    expect(catalog?.defaultProvider).toBeNull()
  })

  it('an empty catalog is explicitly unavailable', async () => {
    ;(api.routerProviders as ReturnType<typeof vi.fn>).mockResolvedValue({ providers: [] })
    ;(api.routerDefaults as ReturnType<typeof vi.fn>).mockResolvedValue({
      default_provider: null,
      default_model: null,
    })
    const { catalog, unavailable } = await loadModelCatalog()
    expect(unavailable).toBe(true)
    expect(catalog?.pairs).toEqual([])
  })

  it('a providers read failure is explicitly unavailable (never throws)', async () => {
    ;(api.routerProviders as ReturnType<typeof vi.fn>).mockRejectedValue(new Error('503'))
    const { catalog, unavailable } = await loadModelCatalog()
    expect(unavailable).toBe(true)
    expect(catalog).toBeNull()
  })
})

describe('preselectFromCatalog', () => {
  it('prefers the configured default provider/model when present in the catalog', () => {
    const catalog = toModelCatalog(
      [
        provider('alpha', [{ id: 'a1', tags: [] }]),
        provider('beta', [{ id: 'b1', tags: [] }]),
      ],
      { default_provider: 'beta', default_model: 'b1' },
    )
    expect(preselectFromCatalog(catalog)).toEqual({ provider: 'beta', model: 'b1' })
  })

  it('falls back to the first pair; a default model of another provider only applies with its provider', () => {
    const catalog = toModelCatalog(
      [provider('alpha', [{ id: 'a1', tags: [] }, { id: 'a2', tags: [] }])],
      { default_provider: null, default_model: 'a2' },
    )
    // default provider 未配置 → 首个 provider；default_model 不在「default
    // provider 一致」的预选规则内 → 该 provider 首个模型。
    expect(preselectFromCatalog(catalog)).toEqual({ provider: 'alpha', model: 'a1' })
  })

  it('never fabricates an absent default', () => {
    const catalog = toModelCatalog([provider('alpha', [{ id: 'a1', tags: [] }])], {
      default_provider: 'ghost',
      default_model: 'nope',
    })
    expect(preselectFromCatalog(catalog)).toEqual({ provider: 'alpha', model: 'a1' })
  })

  it('empty catalog preselects nothing', () => {
    expect(preselectFromCatalog(toModelCatalog([], null))).toEqual({
      provider: null,
      model: null,
    })
  })
})

describe('groupSessionModels (design D3)', () => {
  const catalog = toModelCatalog(
    [
      provider('anthropic', [{ id: 'sonnet', tags: [] }, { id: 'haiku', tags: [] }]),
      provider('deepseek', [{ id: 'deepseek-chat', tags: [] }]),
    ],
    null,
  )

  it('groups session models back to their catalog providers, preserving session order', () => {
    const groups = groupSessionModels(['haiku', 'sonnet'], catalog)
    expect(groups).toEqual([
      { provider: 'anthropic', models: ['haiku', 'sonnet'] },
    ])
  })

  it('puts ids the catalog cannot place into the session-provided group at the bottom', () => {
    const groups = groupSessionModels(['sonnet', 'custom-local'], catalog)
    expect(groups).toEqual([
      { provider: 'anthropic', models: ['sonnet'] },
      { provider: null, models: ['custom-local'] },
    ])
    expect(SESSION_PROVIDED_GROUP_LABEL).toBe('会话提供')
  })

  it('an unavailable catalog degrades to one flat group (no fabricated grouping)', () => {
    const groups = groupSessionModels(['sonnet', 'haiku'], null)
    expect(groups).toEqual([{ provider: null, models: ['sonnet', 'haiku'] }])
  })

  it('empty session models yield no groups', () => {
    expect(groupSessionModels([], catalog)).toEqual([])
  })
})
