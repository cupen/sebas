// @vitest-environment jsdom
/**
 * toModelCatalog（workbench-conversation-view 4.2，design D7）: the SPA-side
 * adapter seam. Covered inputs:
 *   - empty catalog (no providers / no models)
 *   - a provider without models contributes nothing
 *   - configured defaults absent from the catalog pass through untouched
 *   - the healthy path flattens pairs in payload order
 * preselect-last-used-model 1.1: last-used 记忆读写（localStorage，try/catch）
 * 与三级预选（last-used ∈ 目录 → 该对；否则第一对；空目录 → null；stale 对
 * 不伪造选项）。
 */

import { afterEach, describe, expect, it, vi } from 'vitest'
import type { ProviderAdmin } from './client.js'

// loadModelCatalog 走真实 api 客户端——mock 掉 fetch 面（providers /
// providerDefaults）后导入被测模块。
vi.mock('./client.js', () => ({
  api: {
    providers: vi.fn(),
    providerDefaults: vi.fn(),
  },
}))

import { api } from './client.js'
import {
  LAST_USED_PAIR_KEY,
  loadLastUsedPair,
  loadModelCatalog,
  preselectLastUsed,
  groupSessionModels,
  SESSION_PROVIDED_GROUP_LABEL,
  saveLastUsedPair,
  toModelCatalog,
} from './model-catalog.js'

function provider(name: string, models: ProviderAdmin['models']): ProviderAdmin {
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
    ;(api.providers as ReturnType<typeof vi.fn>).mockResolvedValue({
      providers: [provider('alpha', [{ id: 'a1', tags: [] }])],
    })
    ;(api.providerDefaults as ReturnType<typeof vi.fn>).mockResolvedValue({
      default_provider: 'alpha',
      default_model: 'a1',
    })
    const { catalog, unavailable } = await loadModelCatalog()
    expect(unavailable).toBe(false)
    expect(catalog?.pairs).toEqual([{ provider: 'alpha', model: 'a1' }])
    expect(catalog?.defaultProvider).toBe('alpha')
  })

  it('defaults fetch failing does not sink the catalog', async () => {
    ;(api.providers as ReturnType<typeof vi.fn>).mockResolvedValue({
      providers: [provider('alpha', [{ id: 'a1', tags: [] }])],
    })
    ;(api.providerDefaults as ReturnType<typeof vi.fn>).mockRejectedValue(new Error('503'))
    const { catalog, unavailable } = await loadModelCatalog()
    expect(unavailable).toBe(false)
    expect(catalog?.defaultProvider).toBeNull()
  })

  it('an empty catalog is explicitly unavailable', async () => {
    ;(api.providers as ReturnType<typeof vi.fn>).mockResolvedValue({ providers: [] })
    ;(api.providerDefaults as ReturnType<typeof vi.fn>).mockResolvedValue({
      default_provider: null,
      default_model: null,
    })
    const { catalog, unavailable } = await loadModelCatalog()
    expect(unavailable).toBe(true)
    expect(catalog?.pairs).toEqual([])
  })

  it('a providers read failure is explicitly unavailable (never throws)', async () => {
    ;(api.providers as ReturnType<typeof vi.fn>).mockRejectedValue(new Error('503'))
    const { catalog, unavailable } = await loadModelCatalog()
    expect(unavailable).toBe(true)
    expect(catalog).toBeNull()
  })
})

describe('preselectLastUsed（preselect-last-used-model 1.1 三级规则）', () => {
  const catalog = toModelCatalog(
    [
      provider('alpha', [{ id: 'a1', tags: [] }, { id: 'a2', tags: [] }]),
      provider('beta', [{ id: 'b1', tags: [] }]),
    ],
    // defaults 载荷仍在 adapter 里透传，但不再参与预选——预选只看 last-used。
    { default_provider: 'beta', default_model: 'b1' },
  )

  it('① the last-used pair wins when it is still in the catalog (defaults ignored)', () => {
    expect(
      preselectLastUsed(catalog, { provider: 'alpha', model: 'a2' }),
    ).toEqual({ provider: 'alpha', model: 'a2' })
  })

  it('② a null / missing last-used pair falls back to the catalog first pair', () => {
    expect(preselectLastUsed(catalog, null)).toEqual({ provider: 'alpha', model: 'a1' })
  })

  it('② a stale pair (provider or model gone) falls back to the first pair, never fabricated as an option', () => {
    // provider 不在了。
    expect(preselectLastUsed(catalog, { provider: 'ghost', model: 'a1' })).toEqual({
      provider: 'alpha',
      model: 'a1',
    })
    // provider 在、模型不在了。
    expect(preselectLastUsed(catalog, { provider: 'alpha', model: 'a9' })).toEqual({
      provider: 'alpha',
      model: 'a1',
    })
  })

  it('③ an empty catalog preselects nothing', () => {
    expect(preselectLastUsed(toModelCatalog([], null), { provider: 'alpha', model: 'a1' })).toEqual({
      provider: null,
      model: null,
    })
  })
})

describe('last-used memory (localStorage, preselect-last-used-model 1.1)', () => {
  afterEach(() => {
    localStorage.removeItem(LAST_USED_PAIR_KEY)
  })

  it('save + load round-trips the pair', () => {
    saveLastUsedPair({ provider: 'openai', model: 'gpt-5' })
    expect(loadLastUsedPair()).toEqual({ provider: 'openai', model: 'gpt-5' })
    expect(localStorage.getItem(LAST_USED_PAIR_KEY)).toBe(
      JSON.stringify({ provider: 'openai', model: 'gpt-5' }),
    )
  })

  it('no memory yet reads as null (not an error)', () => {
    expect(loadLastUsedPair()).toBeNull()
  })

  it('a malformed payload reads as null (shape-checked, never thrown)', () => {
    localStorage.setItem(LAST_USED_PAIR_KEY, '{not json')
    expect(loadLastUsedPair()).toBeNull()
    localStorage.setItem(LAST_USED_PAIR_KEY, JSON.stringify({ provider: 7, model: null }))
    expect(loadLastUsedPair()).toBeNull()
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
