/**
 * 权限模式的单一词汇出处（polish-workbench-walkthrough-ux 4.1）。
 *
 * 同一个词在创建弹窗与会话面板（composer 权限模式下拉）必须口径唯一：
 * 四个模式各带一句中文解释，两处下拉同源渲染，杜绝「一边裸词一边带解释」
 * 的漂移。状态章（会话头部）的中文措辞也在这里（4.2），auto 与 ungated
 * 徽章合一后不再有英文 `UNGATED` / 红色 `UNKNOWN` 外露。
 */

/** 单个权限模式的词汇条目：wire 值 + 统一显示文案。 */
export interface ModeOption {
  /** wire 上的模式值（`ask` | `edit` | `allow` | `auto`）。 */
  value: string
  /** 下拉选项的统一显示文案（值 + 中文解释）。 */
  label: string
}

/**
 * 四个权限模式的下拉选项（唯一出处；创建弹窗与 composer 下拉同源渲染）。
 * add-agent-settings-and-session-titles 7.2：选项标签首字母大写并移除中文
 * 注释（Ask / Edit / Allow / Auto）——wire 值 `ask|edit|allow|auto` 不变，
 * dashboard 的中文徽标走 modeBadgeLabel（同样不动）。
 */
export const MODE_OPTIONS: readonly ModeOption[] = [
  { value: 'ask', label: 'Ask' },
  { value: 'edit', label: 'Edit' },
  { value: 'allow', label: 'Allow' },
  { value: 'auto', label: 'Auto' },
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

/**
 * composer 下拉的默认态显示文案（4.1）：会话未记录 mode（创建时选了
 * 「默认」或旧会话）时显示「默认（Ask）」而非空白。7.2：与新选项词汇同
 * 源大写（此前是「默认（ask）」）；创建弹窗的空值首项也用这里（消除
 * 重复源）。
 */
export const MODE_DEFAULT_LABEL = '默认（Ask）'
