/**
 * 聚焦反投影的窗口级事件名（fix-webui-qa-round14 4.6 自 rail 创建链复用）。
 *
 * `sebas:project-follow`：把「聚焦会话所属项目路径」投给 app-shell 的
 * `selectedPath`——dashboard 的聚焦收敛（summary 落地后的 followFocused-
 * Project）与 rail 创建成功瞬间的即时反投影（D-2-1：不等 summary 往返）
 * 共用同一事件、同一 shell 入口，主区项目标题只有一条真源通道。
 *
 * 独立成模块的原因：dashboard.ts 的模块图拖着整组组件注册（webawesome
 * 自定义元素升级等副作用），rail 只是投递事件名，不应为它建立
 * rail → dashboard 的循环依赖。
 */

/** 聚焦会话 → 所属项目路径 的窗口级反投影事件（detail: { path })。 */
export const PROJECT_FOLLOW_EVENT = 'sebas:project-follow'
