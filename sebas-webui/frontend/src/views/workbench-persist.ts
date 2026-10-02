/**
 * 工作台形态持久化（fix-webui-qa-round9 4.6，agent-workbench）。
 *
 * 操作员上次的工作台形态——聚焦会话 key——记在 localStorage：
 * `sebas.workbench-focus`。core 重启后服务端焦点指针清零，重新打开工作台时
 * 由此恢复聚焦（会话仍在场时）；项目分组展开态由 rail 自身的
 * `sebas.rail-expanded`（4.2 既有）承载，这里只管聚焦半边。
 *
 * 每次访问都防御性包裹——隐私模式、opaque origin、抛错的 storage 一律降级
 * 为「无上次形态」，绝不影响工作台装载（与 split-persist 同款姿态，存储
 * 探针复用 [`defaultStorage`]）。
 */

import { defaultStorage } from './split-persist.js'

/** 聚焦会话 key 的 localStorage 键（proposal D1 定名风格）。 */
export const WORKBENCH_FOCUS_STORAGE_KEY = 'sebas.workbench-focus'

/** 读上次聚焦的会话 key；`null` = 无上次形态（首次使用 / 存储不可用）。 */
export function loadPersistedWorkbenchFocus(store: Storage | null = defaultStorage()): string | null {
  try {
    return store?.getItem(WORKBENCH_FOCUS_STORAGE_KEY) ?? null
  } catch {
    return null
  }
}

/**
 * 记下当前聚焦的会话 key（聚焦 detail 装载成功的唯一漏斗调用）；
 * `null` = 焦点清空（会话移除）时抹掉形态。写失败静默放弃。
 */
export function savePersistedWorkbenchFocus(
  key: string | null,
  store: Storage | null = defaultStorage(),
): void {
  try {
    if (key === null) store?.removeItem(WORKBENCH_FOCUS_STORAGE_KEY)
    else store?.setItem(WORKBENCH_FOCUS_STORAGE_KEY, key)
  } catch {
    // 写不进去就放弃——形态恢复是尽力而为。
  }
}
