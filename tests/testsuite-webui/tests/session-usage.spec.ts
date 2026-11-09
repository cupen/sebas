/**
 * Journey — 会话级 token 用量可见（add-webui-round7-gaps 1.2，tasks.md 1.2）。
 *
 * 功能：会话用量呈现 / 对应 delta：usage-statistics「会话级 token 用量可见」
 *
 * fake-claude 桩按成功回合序号递增携带 usage（第 n 个成功回合 result 帧 in
 * 100n / out 10n；进程全局计数）——所以断言一律**相对**：先读非零基线，第二
 * 回合后断言数值严格增长，绝不锚绝对值（同一 harness 里先行旅程也会消耗
 * 回合计数）。会话头芯片（data-testid="session-usage"）的数据源是引擎随
 * 会话快照透传的 usage（detail / summary / 相位帧三处同源）。
 *
 * fakeacp（通用 ACP 内核，从不发 UsageUpdate）按 spec 如实呈现
 * 「未上报 token」（引擎投影只认 reported 过的会话——review 钉过的
 * 「{0,0} 冒充已上报」缺陷已修：卡态记 usage_reported 事实，快照按它
 * 门控；round7 主 agent 实施轮收口），
 * 详见本次 review 报告。
 */
import { expect, test } from '@playwright/test'
import {
  createSession,
  ErrorCollector,
  FocusedSession,
  waitStatus,
} from './helpers/index'

/** 会话 composer 的用量芯片（React 以 Tag 呈现累计）。 */
function usageChip(page: import('@playwright/test').Page) {
  return page.getByTestId('composer').getByText(/^(入 \d+ · 出 \d+|token 未上报)$/)
}

/** 解析芯片文本 `入 N · 出 M` 为数值对；不匹配即抛错。 */
function parseUsage(text: string): { input: number; output: number } {
  const m = /入 (\d+) · 出 (\d+)/.exec(text)
  if (!m) throw new Error(`usage chip text does not match "入 N · 出 M": ${text}`)
  return { input: Number(m[1]), output: Number(m[2]) }
}

test.describe('会话用量呈现', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('claude 会话累计随回合增长', () => {
    test('one turn shows non-zero totals, a second turn grows them', async ({ page }) => {
      test.setTimeout(60_000)
      const t = Date.now()
      const detail = new FocusedSession(page)

      // Round 1 via API（确定性），完成后深链进入会话呈现面。
      const key = await createSession(page.request, { prompt: `usage-r1-${t}` })
      await waitStatus(page.request, key, ['done'])
      await page.goto(`/sessions/${key}`)
      await expect(detail.host).toBeVisible()

      // 一回合后：芯片出现非零累计（spec「数值与引擎累计一致」——fake-claude
      // 的回合终值即引擎快照值，此处锚非零 + 形状，不锚绝对值）。
      const chip = usageChip(page)
      await expect(chip).toBeVisible({ timeout: 15_000 })
      await expect
        .poll(
          async () => {
            const u = parseUsage((await chip.textContent()) ?? '')
            return Math.min(u.input, u.output)
          },
          { timeout: 15_000, intervals: [250] },
        )
        .toBeGreaterThan(0)
      const first = parseUsage((await chip.textContent()) ?? '')

      // Round 2 只在第一回合收敛后发起（绝不打进运行中的回合）。
      await detail.sendFollowUp(`usage-r2-${t}`)
      await detail.expectStatus('done')
      await expect
        .poll(
          async () => {
            const u = parseUsage((await chip.textContent()) ?? '')
            return u.input
          },
          { timeout: 20_000, intervals: [250] },
        )
        .toBeGreaterThan(first.input)
      const second = parseUsage((await chip.textContent()) ?? '')
      expect(second.output).toBeGreaterThan(first.output)

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('未上报 token 的 agent 如实呈现', () => {
    test('fakeacp session shows the unreported chip, never a fabricated 0', async ({ page }) => {
      // 已知实现缺陷（add-webui-round7-gaps review 3c 实证）：引擎快照对
      // 未上报会话投影 Some({0,0}) 而非 None，芯片呈现实数 0。缺陷修复前
      // 本用例按预期失败；修复后它会「unexpectedly passed」——届时删除本
      // 实现合同：引擎卡态记 usage_reported 事实（真带 token 计数的帧才
      // 置位），快照按它门控——不冒充、不用「全零即未上报」启发式。
      const t = Date.now()

      const key = await createSession(page.request, { prompt: `acp-usage-${t}`, agent: 'fakeacp' })
      await waitStatus(page.request, key, ['done'])
      await page.goto(`/sessions/${key}`)
      const chip = usageChip(page)
      await expect(chip).toBeVisible({ timeout: 15_000 })

      // spec「明确显示未上报/不可得语义，而非 0」：文案即合同，绝不显示伪造的 0。
      await expect(chip).toHaveText('token 未上报')

      expect(collector.clean()).toEqual([])
    })
  })
})
