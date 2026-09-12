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
import { api } from './client.js'

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

// ─── workbench-interaction-polish 2.1 / D2/D3：共用加载与预选 ────────────────

/**
 * 目录加载结果：`catalog` 只在 `unavailable=false` 时可信。空目录与读取
 * 失败都落在 `unavailable=true`——消费方显示显式不可用提示，绝不伪造选项、
 * 绝不渲染空列表（4.4 语义，由 composer 芯片与创建对话框共用）。
 */
export interface LoadedCatalog {
  catalog: ModelCatalog | null
  unavailable: boolean
}

/**
 * 并取 providers + defaults 并规整（原 composer 内联逻辑的唯一留存处）：
 * defaults 读取失败不拖垮目录（双 null 兜底）；providers 读取失败或目录为
 * 空 = 显式不可用（本函数不抛——消费方拿统一形状，无需各自 try/catch）。
 * 创建对话框与 composer 模型芯片共用，避免双份 fetch 漂移（design D2）。
 */
export async function loadModelCatalog(): Promise<LoadedCatalog> {
  try {
    const [providers, defaults] = await Promise.all([
      api.routerProviders(),
      api.routerDefaults().catch(() => null),
    ])
    const catalog = toModelCatalog(providers.providers, defaults)
    if (catalog.pairs.length === 0) return { catalog, unavailable: true }
    return { catalog, unavailable: false }
  } catch {
    return { catalog: null, unavailable: true }
  }
}

/**
 * 预选规则（原 composer 内联逻辑的唯一留存处）：配置的 default provider /
 * default model 在目录内才用（不伪造选项）；否则取目录第一对。空目录返回
 * 全 null。
 */
export function preselectFromCatalog(catalog: ModelCatalog): {
  provider: string | null
  model: string | null
} {
  if (catalog.pairs.length === 0) return { provider: null, model: null }
  const provider =
    catalog.defaultProvider && catalog.pairs.some((p) => p.provider === catalog.defaultProvider)
      ? catalog.defaultProvider
      : catalog.pairs[0]!.provider
  const models = catalog.pairs.filter((p) => p.provider === provider).map((p) => p.model)
  const wanted =
    catalog.defaultProvider === provider && catalog.defaultModel !== null
      ? catalog.defaultModel
      : null
  const model = wanted && models.includes(wanted) ? wanted : (models[0] ?? null)
  return { provider, model }
}

/** 一个会话模型分组：provider 归属组，或目录查不到的「会话提供」兜底组。 */
export interface SessionModelGroup {
  /** Provider 名；兜底组为 `null`（UI 显示「会话提供」）。 */
  provider: string | null
  models: string[]
}

/** 兜底组的显示名（design D3：会话提供、置底）。 */
export const SESSION_PROVIDED_GROUP_LABEL = '会话提供'

/**
 * （workbench-interaction-polish 4.2，design D3）把会话平铺的
 * `available_models` 交叉引用目录分成两级：目录查得到 id → 其 provider 组
 * （保持会话给出的模型顺序；provider 首次出现顺序跟随首条匹配）；查不到 →
 * 「会话提供」兜底组置底。`catalog` 为 null（目录整体不可得）时所有 id 落
 * 入唯一的兜底组——消费方据「单组且 provider===null」退化为单层平铺
 * （仍可用，不伪造分组）。
 */
export function groupSessionModels(
  sessionModels: string[],
  catalog: ModelCatalog | null,
): SessionModelGroup[] {
  const groups: SessionModelGroup[] = []
  const byProvider = new Map<string, SessionModelGroup>()
  const fallback: SessionModelGroup = { provider: null, models: [] }
  for (const id of sessionModels) {
    const hit = catalog?.pairs.find((p) => p.model === id)
    if (hit) {
      let g = byProvider.get(hit.provider)
      if (!g) {
        g = { provider: hit.provider, models: [] }
        byProvider.set(hit.provider, g)
        groups.push(g)
      }
      g.models.push(id)
    } else {
      fallback.models.push(id)
    }
  }
  if (fallback.models.length > 0) groups.push(fallback)
  // 目录整体不可得：全部落兜底组 → 单组平铺（仍可用，不伪造分组）。
  return groups
}
