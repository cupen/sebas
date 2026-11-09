/**
 * Presentation labels for the React shell. Kept framework-agnostic (no Lit,
 * no DOM) so unit tests and every React surface share one vocabulary.
 */

import type { SessionRow, SessionSummary } from '../../api/client.js'

/** Readable placeholder for a session with no operator label or message. */
export const UNNAMED_SESSION_LABEL = '未命名会话'

/**
 * Naming chain shared with the Lit rail (`fullSessionLabel`): operator label →
 * first-message preview → readable placeholder. Machine identifiers
 * (`session_id_short`, encoded-key tails, `web%00web-…`) are never display
 * names: a zero-message placeholder has no user-facing text of its own, so
 * showing a UUID fragment or an undecoded key would just be noise.
 */
export function sessionLabel(
  row: Pick<SessionRow, 'label' | 'prompt_preview'> & {
    session_id_short?: string | null
    encoded_key?: string
    status?: string
    status_slug?: string
  },
): string {
  const label = typeof row.label === 'string' ? row.label.trim() : ''
  if (label) return label
  const preview = typeof row.prompt_preview === 'string' ? row.prompt_preview.trim() : ''
  if (preview) return preview
  return UNNAMED_SESSION_LABEL
}

export function summaryLabel(summary: SessionSummary): string {
  return sessionLabel({
    label: null,
    prompt_preview: null,
    session_id_short: summary.session_id,
    encoded_key: summary.encoded_key,
  })
}

const STATUS_LABELS: Record<string, string> = {
  starting: '启动中',
  queued: '排队中',
  working: '运行中',
  waiting: '等待你处理',
  done: '完成',
  failed: '失败',
  dormant: '休眠',
}

export function statusLabel(status: string): string {
  return STATUS_LABELS[status] ?? status
}

/** Colour token name; text always accompanies it (never colour alone). */
export function statusTone(status: string): 'ok' | 'warn' | 'err' | 'info' | 'neutral' {
  if (status === 'done') return 'ok'
  if (status === 'failed') return 'err'
  if (status === 'waiting') return 'warn'
  if (status === 'starting' || status === 'queued') return 'info'
  if (status === 'working') return 'warn'
  return 'neutral'
}

export function relativeTime(unixSeconds: number, now = Date.now()): string {
  if (!unixSeconds) return '未知'
  const seconds = Math.max(0, Math.floor(now / 1000) - unixSeconds)
  if (seconds < 60) return `${seconds} 秒前`
  if (seconds < 3600) return `${Math.floor(seconds / 60)} 分钟前`
  if (seconds < 86_400) return `${Math.floor(seconds / 3600)} 小时前`
  return `${Math.floor(seconds / 86_400)} 天前`
}
