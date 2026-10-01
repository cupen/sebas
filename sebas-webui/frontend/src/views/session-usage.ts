/**
 * 会话用量芯片的呈现投影（add-webui-round7-gaps 1.1，usage-statistics spec
 * 「会话级 token 用量可见」）。数据源是引擎随会话快照输出的 usage 累计
 * （detail / summary / 相位帧三处同源）；本模块是它到呈现词的唯一投影——
 * 未上报 token 的 agent（通用 ACP：usage 语义是 context/cost，不折算）
 * 如实呈现「未上报」，不以 0 或其它实数冒充。
 */

import type { SessionUsage } from '../api/client.js'

/** 芯片的呈现形状（dashboard 的会话头直接消费）。 */
export interface SessionUsageView {
  /** 芯片主文本。 */
  text: string
  /** 悬停 title（含模型名与累计口径）。 */
  title: string
  /** true = 引擎尚无 usage 事件（含通用 ACP 不上报 token）——未上报姿态。 */
  unreported: boolean
  /** `data-usage` 修饰词，呈现层据此分色（reported 常态 / unreported 弱化）。 */
  tone: 'reported' | 'unreported'
}

/** 未上报的芯片主文本（spec「未上报/不可得语义，而非 0」）。 */
export const USAGE_UNREPORTED_TEXT = '未上报 token'

export function sessionUsageView(usage: SessionUsage | null | undefined): SessionUsageView {
  if (!usage) {
    return {
      text: USAGE_UNREPORTED_TEXT,
      title: '该 agent 未上报 token 用量（通用 ACP 内核只报上下文/费用语义，不折算成 token）',
      unreported: true,
      tone: 'unreported',
    }
  }
  const model = usage.model ? `${usage.model} · ` : ''
  return {
    text: `Token in ${usage.total_input} · out ${usage.total_output}`,
    title: `${model}累计 input ${usage.total_input} · output ${usage.total_output}`,
    unreported: false,
    tone: 'reported',
  }
}
