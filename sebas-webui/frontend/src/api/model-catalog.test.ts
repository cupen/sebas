// @vitest-environment jsdom
/**
 * toModelCatalog（workbench-conversation-view 4.2，design D7）: the SPA-side
 * adapter seam. Covered inputs:
 *   - empty catalog (no providers / no models)
 *   - a provider without models contributes nothing
 *   - configured defaults absent from the catalog pass through untouched
 *   - the healthy path flattens pairs in payload order
 */

import { describe, expect, it } from 'vitest'
import type { RouterProviderAdmin } from './client.js'
import { toModelCatalog } from './model-catalog.js'

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
