/**
 * 权限模式的单一词汇出处（polish-workbench-walkthrough-ux 4.1）。
 *
 * 同一个词在创建弹窗与会话面板（composer 权限模式下拉）必须口径唯一：
 * 四个模式各带一句中文解释，两处下拉同源渲染，杜绝「一边裸词一边带解释」
 * 的漂移。状态章（会话头部）的中文措辞也在这里（4.2），auto 与 ungated
 * 徽章合一后不再有英文 `UNGATED` / 红色 `UNKNOWN` 外露。
 */

/** 单个权限模式的词汇条目：wire 值 + 统一显示文案 + 非内联解释。 */
export interface ModeOption {
  /** wire 上的模式值（`ask` | `edit` | `allow` | `auto`）。 */
  value: string
  /** 下拉选项的统一显示文案（裸词，菜单项不带内联注解）。 */
  label: string
  /**
   * 模式的中文解释（simplify-mode-menus，恢复 7.2 前的解释语义）：只走
   * 非内联通道——`wa-option` 的悬浮 `title` 与创建弹窗 `wa-select` 的
   * 动态 hint 行——绝不拼回选项标签。措辞与 `modeBadgeLabel` 各自独立
   * （徽章求短、悬浮求解释），同文件相邻定义保证口径可见。
   */
  description: string
}

/**
 * 四个权限模式的下拉选项（唯一出处；创建弹窗与 composer 下拉同源渲染）。
 * add-agent-settings-and-session-titles 7.2：选项标签首字母大写（Ask /
 * Edit / Allow / Auto）；simplify-mode-menus：空值占位项与旧「默认」
 * 常量一并退役（菜单恰为四词，spec「SHALL NOT render
 * an empty or placeholder mode state」），解释语义经 description 以
 * title/hint 通道回归——wire 值 `ask|edit|allow|auto` 不变，dashboard
 * 的中文徽标走 modeBadgeLabel（同样不动）。
 */
export const MODE_OPTIONS: readonly ModeOption[] = [
  { value: 'ask', label: 'Ask', description: '逐次询问' },
  { value: 'edit', label: 'Edit', description: '自动接受编辑' },
  { value: 'allow', label: 'Allow', description: '放行并留审计' },
  { value: 'auto', label: 'Auto', description: '自动执行（不门控，留审计）' },
]

/**
 * 模式的状态章中文措辞（4.2）：会话头部的模式章用词；auto 即原 ungated
 * 语义（「自动执行」琥珀章），不再另挂英文 `UNGATED` 徽章。未知值如实
 * 显示原词，不造红色 `UNKNOWN`。
 */
export function modeBadgeLabel(mode: string): string {
  switch (mode) {
    case 'ask':
      return '逐次询问'
    case 'edit':
      return '自动接受编辑'
    case 'allow':
      return '放行'
    case 'auto':
      return '自动执行'
    default:
      return mode
  }
}
