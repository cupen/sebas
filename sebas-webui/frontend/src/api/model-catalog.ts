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

import type { ProviderAdmin, ModelAliasEntry } from './client.js'
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
 *
 * （fix-webui-qa-round14 3.1/3.2）`aliases` 是别名短名清单（模型选择面的
 * 并入项）。注意 `unavailable` 语义随别名消费面微调：目录 pairs 为空但
 * 存在别名时不再是「全不可用」——别名本身是可用的模型取值，消费方仍应
 * 渲染别名选择面（catalog 为 null 时按「无目录模型」处理）。
 */
export interface LoadedCatalog {
  catalog: ModelCatalog | null
  /** 别名短名（fix-webui-qa-round14 3.1/3.2）；读取失败/无别名 = 空表。 */
  aliases: ModelAliasChoice[]
  unavailable: boolean
}

/**
 * 并取 providers + defaults 并规整（原 composer 内联逻辑的唯一留存处）：
 * defaults 读取失败不拖垮目录（双 null 兜底）；providers 读取失败或目录为
 * 空 = 显式不可用（本函数不抛——消费方拿统一形状，无需各自 try/catch）。
 * 创建对话框与 composer 模型芯片共用，避免双份 fetch 漂移（design D2）。
 *
 * （fix-webui-qa-round14 3.1/3.2，D-4-1）`model_aliases` 随同一份 providers
 * 响应下发（零额外请求），经 [`toAliasChoices`] 规整进 [`LoadedCatalog`]——
 * composer 模型菜单与创建弹窗下拉据此并入别名短名。读取失败 aliases 为空，
 * 不拖垮目录本身。
 */
export async function loadModelCatalog(): Promise<LoadedCatalog> {
  try {
    const [providers, defaults] = await Promise.all([
      api.providers(),
      api.providerDefaults().catch(() => null),
    ])
    const catalog = toModelCatalog(providers.providers, defaults)
    const aliases = toAliasChoices(providers.model_aliases)
    if (catalog.pairs.length === 0 && aliases.length === 0) {
      return { catalog, aliases, unavailable: true }
    }
    // 有目录模型或别名任一在场 = 选择面可用（unavailable=false 时 catalog
    // 仍可能只有空 pairs——消费方按「无目录模型」处理，不渲染空 provider 级）。
    return { catalog, aliases, unavailable: false }
  } catch {
    return { catalog: null, aliases: [], unavailable: true }
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

// ─── fix-webui-qa-round14 3.1/3.2：模型别名进入模型选择面（D-4-1）────────────

/** 一个可选的模型别名（providers 读面 `model_aliases` 的规整形）。 */
export interface ModelAliasChoice {
  /** 别名短名——选中即以它为模型值（经既有 set_model 控制帧路径）。 */
  alias: string
  /** 目标 provider（来源徽标的 tooltip 内容）。 */
  provider: string
  /** 上游模型覆写；缺省 = 目标 provider 默认模型。 */
  upstream_model?: string
}

/** composer 模型菜单「别名」组的显示名（来源徽标同词）。 */
export const ALIAS_GROUP_LABEL = '别名'

/** 模型菜单的一个条目：提交值 + 来源标记（别名条目带 `alias: true`）。 */
export interface ModelMenuEntry {
  value: string
  alias: boolean
}

/**
 * providers 读面的 `model_aliases` 记录 → 规整的别名清单（按名字典序稳定
 * 输出）。`null`/缺省 = 无别名（消费面回退纯目录模型）。
 */
export function toAliasChoices(
  payload: Record<string, ModelAliasEntry> | null | undefined,
): ModelAliasChoice[] {
  return Object.entries(payload ?? {})
    .map(([alias, entry]) => ({
      alias,
      provider: entry.provider,
      ...(entry.upstream_model ? { upstream_model: entry.upstream_model } : {}),
    }))
    .sort((a, b) => a.alias.localeCompare(b.alias))
}

/**
 * 目录模型列表与别名的合并（design 决策 3）：别名条目在前、带来源标记；
 * 同名冲突时别名条目优先（目录/会话模型条目被别名替换——同一个名字只能
 * 出现一次，选中语义归别名）。
 */
export function mergeAliasEntries(
  models: readonly string[],
  aliases: readonly ModelAliasChoice[],
): ModelMenuEntry[] {
  const aliasNames = new Set(aliases.map((a) => a.alias))
  const entries: ModelMenuEntry[] = aliases.map((a) => ({ value: a.alias, alias: true }))
  for (const m of models) {
    if (!aliasNames.has(m)) entries.push({ value: m, alias: false })
  }
  return entries
}

/** 一个别名条目的悬浮说明（来源徽标 tooltip：目标 provider + 上游覆写）。 */
export function aliasEntryTitle(choice: ModelAliasChoice): string {
  return `别名 → ${choice.provider}${choice.upstream_model ? ` · ${choice.upstream_model}` : ''}`
}

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
