/**
 * Journey — 零输出回合 notice 的浏览器级真链路（close-acceptance-blind-spots
 * 8.1 收口，spec session-lifecycle「Turn completing without visible output
 * appends a notice」）。
 *
 * 进程级 e2e 由验收套件 zero_output_turn_notice_journey 覆盖（断言投影条目
 * 形状）；本旅程补浏览器层：沙箱的第二个 claude 驱动 agent `claude-empty`
 * 以 `--scenario empty` 跑零输出回合（tasks.py 装配），旅程断言
 *
 * - 空回合有落点：transcript 时间线出现 notice 中性信息条（data-testid=
 *   "notice-entry"，导语「回合已结束且无输出」），时间线 = 操作者回合 +
 *   notice，此外无任何 agent 气泡——回合不再不可见地消失；
 * - 正常回合不受影响：同一沙箱的 claude 桩会话（hello world）不含 notice；
 * - 明暗两态：notice 条目在 light（默认 system→light）与 dark（colorScheme
 *   模拟）主题下同样可见（4.2 的两态渲染走真实 token 翻转）。
 */
import { expect, test } from '@playwright/test'
import {
  createSession,
  ErrorCollector,
  FocusedSession,
  resetState,
  waitStatus,
} from './helpers/index'

test.describe('零输出回合 notice（close-acceptance-blind-spots）', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('空回合有落点（浏览器真链路）', () => {
    test('empty 桩会话：时间线 = prompt + notice 中性条目，此外无 agent 气泡', async ({
      page,
    }) => {
      const detail = new FocusedSession(page)

      await resetState(page.request)
      const key = await createSession(page.request, {
        prompt: 'say nothing',
        agent: 'claude-empty',
      })
      await waitStatus(page.request, key, ['done'])
      await page.goto(`/sessions/${key}`)
      await expect(detail.sessionHead).toBeVisible()

      // notice 条目在时间线上可见：中性信息条 +「回合已结束且无输出」导语
      // （与后端 ZERO_OUTPUT_NOTICE 措辞对齐，进程级 journey 钉过形状）。
      const notice = page.locator('sebas-transcript-view [data-testid="notice-entry"]')
      await expect(notice).toBeVisible()
      await expect(notice).toContainText('回合已结束且无输出')

      // 时间线恰好两个回合块：操作者 prompt + notice。零输出回合不再
      // 不可见地消失，也没有多余的 agent 气泡（时间线序 = spec 的
      // prompt → notice 落点序，进程级 journey 钉过同序）。
      const blocks = page.locator('sebas-transcript-view .turn-block')
      await expect(blocks).toHaveCount(2)
      await expect(blocks.nth(0)).toContainText('say nothing')
      await expect(blocks.nth(1)).toContainText('回合已结束且无输出')
      await expect(page.locator('sebas-transcript-view .turn-block.is-notice')).toHaveCount(1)

      expect(collector.clean()).toEqual([])
    })

    test('正常回合（claude 桩 hello world）不追加 notice 条目', async ({ page }) => {
      const detail = new FocusedSession(page)

      await resetState(page.request)
      const key = await createSession(page.request, { prompt: 'hello', agent: 'claude' })
      await waitStatus(page.request, key, ['done'])
      await page.goto(`/sessions/${key}`)
      await expect(detail.sessionHead).toBeVisible()

      // 正常产出正文/工具输出的回合没有 notice——零输出落点只对空回合生效。
      await expect(page.locator('sebas-transcript-view [data-testid="notice-entry"]')).toHaveCount(
        0,
      )
      await expect(page.locator('sebas-transcript-view .turn-block.is-notice')).toHaveCount(0)
      // 回合本身照常可见：操作者 prompt + hello world 正文气泡。
      await expect(detail.turnWith('hello world')).toBeVisible()

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('notice 明暗两态', () => {
    test('light 主题（默认 system→light）：notice 条目可见且页面不带 wa-dark', async ({
      page,
    }) => {
      const detail = new FocusedSession(page)

      await resetState(page.request)
      const key = await createSession(page.request, {
        prompt: 'say nothing light',
        agent: 'claude-empty',
      })
      await waitStatus(page.request, key, ['done'])
      await page.goto(`/sessions/${key}`)
      await expect(detail.sessionHead).toBeVisible()

      await expect(page.locator('html')).not.toHaveClass(/wa-dark/)
      await expect(page.locator('sebas-transcript-view [data-testid="notice-entry"]')).toBeVisible()

      expect(collector.clean()).toEqual([])
    })

    test.describe('dark 主题（prefers-color-scheme: dark）', () => {
      test.use({ colorScheme: 'dark' })

      test('notice 条目可见且页面带 wa-dark', async ({ page }) => {
        const detail = new FocusedSession(page)

        await resetState(page.request)
        const key = await createSession(page.request, {
          prompt: 'say nothing dark',
          agent: 'claude-empty',
        })
        await waitStatus(page.request, key, ['done'])
        await page.goto(`/sessions/${key}`)
        await expect(detail.sessionHead).toBeVisible()

        // 主题机制：wa-dark 类挂在 <html>（theme.ts 单一开关），notice 条目
        // 随 tokens.css 同源翻转——深色面下中性信息条依旧可见可读。
        await expect(page.locator('html')).toHaveClass(/wa-dark/)
        await expect(
          page.locator('sebas-transcript-view [data-testid="notice-entry"]'),
        ).toBeVisible()

        expect(collector.clean()).toEqual([])
      })
    })
  })
})
