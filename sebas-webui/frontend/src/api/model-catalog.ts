/**
 * Model catalog adapter（workbench-conversation-view 4.2，design D7）: the
 * SPA-side seam that turns the raw provider admin payload plus the defaults
 * payload into the shape the composer's two-level selector consumes.
 *
 * One place owns the "reading" of the catalog; the `models` structure is due
 * to be reshaped by a later change, so NOTHING here interprets it — no
 * ordering semantics, no `[1m]`-suffix explanation. Ids and names pass
 * through verbatim; entry lists are normalized only in the sense that a
 * provider without models contributes no models (never a fabricated one).
 */

import type { RouterProviderAdmin } from './client.js'

/** Wire shape of `GET /router/api/defaults`（双 null = 未设置）. */
export interface DefaultsPayload {
  default_provider: string | null
  default_model: string | null
}

/** One flat catalog pair: a model id under its owning provider. */
export interface ModelCatalogPair {
  provider: string
  model: string
}

/** The adapter's output: the flat pair list plus the configured defaults. */
export interface ModelCatalog {
  /** Every (provider, model) pair, payload order preserved. */
  pairs: ModelCatalogPair[]
  /** Configured default provider; `null` = unset. */
  defaultProvider: string | null
  /** Configured default model; `null` = unset. */
  defaultModel: string | null
}

/**
 * Flatten the provider admin list + defaults into the selector's catalog.
 * Providers without a `models` list contribute nothing (they stay a valid
 * two-level choice only if some model exists — with none, there is nothing
 * to pick, and inventing an entry would be a lie). Defaults pass through
 * untouched: the caller decides whether a default that is absent from the
 * catalog can still be preselected (it cannot — no fabricated options).
 */
export function toModelCatalog(
  providers: RouterProviderAdmin[],
  defaults: DefaultsPayload | null,
): ModelCatalog {
  const pairs: ModelCatalogPair[] = []
  for (const p of providers) {
    for (const m of p.models ?? []) {
      pairs.push({ provider: p.name, model: m.id })
    }
  }
  return {
    pairs,
    defaultProvider: defaults?.default_provider ?? null,
    defaultModel: defaults?.default_model ?? null,
  }
}
