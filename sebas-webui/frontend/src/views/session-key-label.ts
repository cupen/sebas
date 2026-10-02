/**
 * （fix-webui-qa-round8 4.4，agent-workbench「编码会话标识展示友好化」）
 * 会话标识的**展示层**友好化：编码串（`web%00web-1709…-4`，URL-safe 的
 * `channel\0reference` percent 形）解码为「渠道 · 本地段」人读标签。
 *
 * **只用于展示位**（审批面板的会话 chip、聚焦链接等）——wire 与路由不变：
 * 会话 key 的编码格式是兼容面（`\0` 分隔符，见 proposal Non-goals），URL、
 * WS 帧的 session_id、switch/detail API 一律仍用编码形。
 */

/** 渠道词表 → 人读渠道名（未知渠道原样透传，不猜）。 */
const CHANNEL_LABELS: Record<string, string> = {
  web: 'Web',
  feishu: '飞书',
}

/**
 * 编码会话键 → 「渠道 · 本地段」标签（纯函数）。
 *
 * - `web%00web-1709…-4` → `Web · web-1709…-4`；
 * - `feishu%00agent-e449d52f` → `飞书 · agent-e449d52f`；
 * - 带 thread 段的键（`web%00ref%00thread`）取渠道 + 首个本地段；
 * - 无法解析（无分隔符 / 解码失败）→ 原样返回（绝不让展示位变空）。
 */
export function friendlySessionKey(encodedKey: string): string {
  if (!encodedKey) return encodedKey
  let decoded = encodedKey
  try {
    decoded = decodeURIComponent(encodedKey)
  } catch {
    decoded = encodedKey
  }
  const parts = decoded.split('\0')
  if (parts.length < 2 || !parts[0] || !parts[1]) {
    // 未编码的裸串（无 NUL）：尝试按 %00 直拆（防御旧调用方）。
    const raw = encodedKey.split('%00')
    if (raw.length >= 2 && raw[0] && raw[1]) {
      return `${CHANNEL_LABELS[raw[0]] ?? raw[0]} · ${raw[1]}`
    }
    return encodedKey
  }
  return `${CHANNEL_LABELS[parts[0]] ?? parts[0]} · ${parts[1]}`
}
