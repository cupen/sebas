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

import type { ProviderAdmin } from './client.js'
import { api } from './client.js'

/** Wire shape of `GET /api/provider-defaults`（双 null = 未设置）. */
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
 * untouched as adapter output (preselect-last-used-model：创建预选已改用
 * last-used 语义；defaults 载荷只为 router 管理面保留透传，不再参与预选).
 */
export function toModelCatalog(
  providers: ProviderAdmin[],
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
      api.providers(),
      api.providerDefaults().catch(() => null),
    ])
    const catalog = toModelCatalog(providers.providers, defaults)
    if (catalog.pairs.length === 0) return { catalog, unavailable: true }
    return { catalog, unavailable: false }
  } catch {
    return { catalog: null, unavailable: true }
  }
}

// ─── preselect-last-used-model 1.1：上次选择记忆与三级预选 ───────────────────

/**
 * 创建对话框「上次选择」记忆的 localStorage 键：全局一份 `(provider, model)`
 * 对。写入点唯一——创建对话框的确认动作；会话内模型 chip 的切换绝不写它。
 */
export const LAST_USED_PAIR_KEY = 'lastUsedModelPair'

/**
 * 读取上次确认的 (provider, model) 对。localStorage 不可得（jsdom opaque
 * origin、隐私模式等）或载荷形状不对（手改/旧版本残留）→ 如实返回 null，
 * 绝不抛错——记忆缺失只影响预选，不阻塞创建。
 */
export function loadLastUsedPair(): ModelCatalogPair | null {
  try {
    const raw = localStorage.getItem(LAST_USED_PAIR_KEY)
    if (!raw) return null
    const parsed: unknown = JSON.parse(raw)
    if (typeof parsed !== 'object' || parsed === null) return null
    const { provider, model } = parsed as { provider?: unknown; model?: unknown }
    if (typeof provider !== 'string' || typeof model !== 'string') return null
    return { provider, model }
  } catch {
    return null
  }
}

/**
 * 写入上次确认的 (provider, model) 对（唯一调用方：创建对话框确认动作）。
 * storage 不可得时静默放弃——记忆是锦上添花，创建本身不依赖它。
 */
export function saveLastUsedPair(pair: ModelCatalogPair): void {
  try {
    localStorage.setItem(LAST_USED_PAIR_KEY, JSON.stringify(pair))
  } catch {
    // storage 不可得：放弃记忆，不阻塞创建。
  }
}

/**
 * 预选规则（preselect-last-used-model，取代 configured-defaults 语义）：
 * ① 上次确认的对仍在目录内 → 该对（stale 对绝不伪造为选项）；② 否则目录
 * 第一对；③ 目录空 → 全 null（消费方呈显式引导，不渲染空选择器）。
 * 配置的 default provider/model 不再参与预选（adapter 仍透传该载荷，router
 * 管理面继续用）。
 */
export function preselectLastUsed(
  catalog: ModelCatalog,
  lastUsed: ModelCatalogPair | null,
): {
  provider: string | null
  model: string | null
} {
  if (catalog.pairs.length === 0) return { provider: null, model: null }
  const hit = lastUsed
    ? catalog.pairs.find((p) => p.provider === lastUsed.provider && p.model === lastUsed.model)
    : undefined
  const target = hit ?? catalog.pairs[0]!
  return { provider: target.provider, model: target.model }
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
