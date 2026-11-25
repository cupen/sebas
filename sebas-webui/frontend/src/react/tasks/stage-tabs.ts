/**
 * stage 视图切换的纯状态（agent-workbench：tab 状态按项目各自记忆）。
 *
 * tab 只影响 stage 渲染，不参与路由也不触碰聚焦会话指针——切 tab 不中断任何
 * 会话（spec「switching tabs does not affect sessions」）。
 */

export type StageTab = 'conversation' | 'tasks'

export type StageTabState = Record<string, StageTab>

export const DEFAULT_STAGE_TAB: StageTab = 'conversation'

/** 某项目当前 tab（未设置/无项目 = 会话）。 */
export function tabForProject(state: StageTabState, projectId: string | null): StageTab {
  if (projectId === null) return DEFAULT_STAGE_TAB
  return state[projectId] ?? DEFAULT_STAGE_TAB
}

/** 写入某项目的 tab，其余项目原样保留（项目切换不互相覆盖）。 */
export function withProjectTab(
  state: StageTabState,
  projectId: string | null,
  tab: StageTab,
): StageTabState {
  if (projectId === null || state[projectId] === tab) return state
  return { ...state, [projectId]: tab }
}
