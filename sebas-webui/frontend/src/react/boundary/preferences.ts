/**
 * localStorage boundary (2.4). Reads the keys the Lit app already ships so a
 * React shell opens with the operator's existing preferences. Every access is
 * defensive: private mode / opaque origin / malformed values degrade to the
 * documented default instead of throwing.
 */

import { loadComposerHeight, loadRailWidth, defaultStorage } from '../../views/split-persist.js'
import { loadPersistedWorkbenchFocus } from '../../views/workbench-persist.js'
import type { ThemeMode } from '../../theme.js'

export const STORAGE_KEYS = {
  theme: 'sebas:theme',
  railWidth: 'sebas.rail-width',
  composerHeight: 'sebas.composer-height',
  workbenchFocus: 'sebas.workbench-focus',
  railExpanded: 'sebas.rail-expanded',
  railCollapsed: 'sebas.rail-collapsed',
  railHistoryOpen: 'sebas.rail-history-open',
  seenPrefix: 'sebas:seen:',
  lastSettingsSection: 'lastSettingsSection',
  lastUsedModelPair: 'lastUsedModelPair',
} as const

export type SettingsSectionId =
  | 'generic'
  | 'appearance'
  | 'services'
  | 'users'
  | 'models'
  | 'aliases'
  | 'agents'
  | 'skills'
  | 'env-vars'
  | 'about'

export const SETTINGS_SECTION_IDS: readonly SettingsSectionId[] = [
  'generic',
  'appearance',
  'services',
  'users',
  'models',
  'aliases',
  'agents',
  'skills',
  'env-vars',
  'about',
]

/** The subset the React shell restores at boot. */
export interface StoredPreferences {
  theme: ThemeMode
  railWidth: number | null
  composerHeight: number | null
  workbenchFocus: string | null
  railExpanded: Record<string, boolean>
  railHistoryOpen: boolean
  lastSettingsSection: SettingsSectionId
  lastUsedModelPair: { provider: string; model: string } | null
}

export function safeStorage(explicit?: Storage | null): Storage | null {
  if (explicit !== undefined) return explicit
  return defaultStorage()
}

function rawGet(store: Storage | null, key: string): string | null {
  try {
    return store?.getItem(key) ?? null
  } catch {
    return null
  }
}

function rawSet(store: Storage | null, key: string, value: string): void {
  try {
    store?.setItem(key, value)
  } catch {
    /* best-effort persistence */
  }
}

export function readTheme(store: Storage | null = safeStorage()): ThemeMode {
  const raw = rawGet(store, STORAGE_KEYS.theme)
  return raw === 'dark' || raw === 'light' || raw === 'system' ? raw : 'system'
}

export function writeTheme(mode: ThemeMode, store: Storage | null = safeStorage()): void {
  rawSet(store, STORAGE_KEYS.theme, mode)
}

/** Rail collapse is a new React-shell preference; absent reads as expanded. */
export function readRailCollapsed(store: Storage | null = safeStorage()): boolean {
  return rawGet(store, STORAGE_KEYS.railCollapsed) === '1'
}

export function writeRailCollapsed(
  collapsed: boolean,
  store: Storage | null = safeStorage(),
): void {
  if (collapsed) rawSet(store, STORAGE_KEYS.railCollapsed, '1')
  else {
    try {
      store?.removeItem(STORAGE_KEYS.railCollapsed)
    } catch {
      /* best-effort */
    }
  }
}

export function readRailHistoryOpen(store: Storage | null = safeStorage()): boolean {
  return rawGet(store, STORAGE_KEYS.railHistoryOpen) === '1'
}

export function writeRailHistoryOpen(
  open: boolean,
  store: Storage | null = safeStorage(),
): void {
  if (open) rawSet(store, STORAGE_KEYS.railHistoryOpen, '1')
  else {
    try {
      store?.removeItem(STORAGE_KEYS.railHistoryOpen)
    } catch {
      /* best-effort */
    }
  }
}

/**
 * Parsed expanded-project record. Two shapes are accepted:
 * - the React-shell map `{"/path": false}` — explicit collapse must survive a
 *   reload, so falsy records are first-class (an absent key means "no record",
 *   which falls back to the focus default and would re-expand a collapsed row);
 * - the legacy Lit array of expanded paths `["/a", "/b"]` — migrated on read,
 *   true entries only. Junk array items are ignored, never a parse failure.
 */
export function readRailExpanded(
  store: Storage | null = safeStorage(),
): Record<string, boolean> {
  const raw = rawGet(store, STORAGE_KEYS.railExpanded)
  if (!raw) return {}
  try {
    const parsed: unknown = JSON.parse(raw)
    const out: Record<string, boolean> = {}
    if (Array.isArray(parsed)) {
      for (const item of parsed) {
        if (typeof item === 'string' && item) out[item] = true
      }
      return out
    }
    if (parsed !== null && typeof parsed === 'object') {
      for (const [path, expanded] of Object.entries(parsed as Record<string, unknown>)) {
        if (path && typeof expanded === 'boolean') out[path] = expanded
      }
    }
    return out
  } catch {
    return {}
  }
}

/**
 * Persist the full record, including explicit `false` entries (collapse is a
 * decision, not the absence of one). Non-boolean/empty keys are dropped so a
 * corrupted in-memory record cannot poison storage.
 */
export function writeRailExpanded(
  record: Record<string, boolean>,
  store: Storage | null = safeStorage(),
): void {
  const out: Record<string, boolean> = {}
  for (const [path, expanded] of Object.entries(record)) {
    if (path && typeof expanded === 'boolean') out[path] = expanded
  }
  rawSet(store, STORAGE_KEYS.railExpanded, JSON.stringify(out))
}

/** `lastSettingsSection`; legacy/invalid values fall back to `generic`. */
export function readLastSettingsSection(
  store: Storage | null = safeStorage(),
): SettingsSectionId {
  const raw = rawGet(store, STORAGE_KEYS.lastSettingsSection)
  return SETTINGS_SECTION_IDS.includes(raw as SettingsSectionId)
    ? (raw as SettingsSectionId)
    : 'generic'
}

export function writeLastSettingsSection(
  section: SettingsSectionId,
  store: Storage | null = safeStorage(),
): void {
  rawSet(store, STORAGE_KEYS.lastSettingsSection, section)
}

/** `lastUsedModelPair` JSON; malformed payloads read as null. */
export function readLastUsedModelPair(
  store: Storage | null = safeStorage(),
): { provider: string; model: string } | null {
  const raw = rawGet(store, STORAGE_KEYS.lastUsedModelPair)
  if (!raw) return null
  try {
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
 * Per-session read anchor (`sebas:seen:<raw encoded key>` → `{anchor_count}`).
 * Legacy `seen_ts` payloads read as null = fully read, matching the Lit app.
 */
export function readSeenAnchor(
  sessionKey: string,
  store: Storage | null = safeStorage(),
): number | null {
  const raw = rawGet(store, STORAGE_KEYS.seenPrefix + sessionKey)
  if (raw === null) return null
  try {
    const value: unknown = JSON.parse(raw)
    if (value !== null && typeof value === 'object' && !Array.isArray(value)) {
      const count = (value as { anchor_count?: unknown }).anchor_count
      if (typeof count === 'number' && Number.isFinite(count) && count >= 0) return count
    }
    return null
  } catch {
    return null
  }
}

/** Monotonic write: a lower anchor never rolls back a stored one. */
export function writeSeenAnchor(
  sessionKey: string,
  anchorCount: number,
  store: Storage | null = safeStorage(),
): void {
  const previous = readSeenAnchor(sessionKey, store)
  if (previous !== null && previous >= anchorCount) return
  rawSet(
    store,
    STORAGE_KEYS.seenPrefix + sessionKey,
    JSON.stringify({ anchor_count: anchorCount }),
  )
}

/** Restore everything the shell needs in one read. */
export function loadPreferences(store: Storage | null = safeStorage()): StoredPreferences {
  return {
    theme: readTheme(store),
    railWidth: loadRailWidth(store),
    composerHeight: loadComposerHeight(store),
    workbenchFocus: loadPersistedWorkbenchFocus(store),
    railExpanded: readRailExpanded(store),
    railHistoryOpen: readRailHistoryOpen(store),
    lastSettingsSection: readLastSettingsSection(store),
    lastUsedModelPair: readLastUsedModelPair(store),
  }
}
