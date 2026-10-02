/**
 * native 转录补全与影子队列的浏览器级复测（fix-webui-qa-round8 2.3/5.2）。
 *
 * 装配：`playwright.native.config.ts`（端口 9894，TESTSUITE_NATIVE=1）——
 * harness 把 native 内核指向沙箱内已在跑的 debug router，默认模型
 * `test/text`、可用模型 = 九场景 + bare `test`。默认沙箱没有这组 env，
 * 本文件只在 native 装配下运行（native config 的 testMatch 同名单）。
 *
 * 覆盖：
 * - agent-workbench「转录为操作者提交渲染用户气泡（不分执行体）」native
 *   半边（2.3）：composer 提交 → `is-user` 气泡先于回复上屏（2.1 内核宿主
 *   在投递前落的 prompt 条目）；
 * - agent-workbench「会话内模型切换留痕」native 半边（5.2）：API 切模型 →
 *   `model_change` 系统条目含新旧模型名（native override 路径）；
 * - agent-workbench「排队提交在 native 会话可见」（2.3）：回合中追加提交 →
 *   待执行栈出现条目、可移除（栈与 API 真源一致）；留在栈上的条目在回合
 *   结束后按序执行——2.2 影子队列「记入 → 对账推进 → 清空」的浏览器半边。
 *
 * 确定性基座：`test/long` 的 SSE 滴流 ≈1.9s（1014 字符 / 32 字符帧 ×
 * 60ms，另加内核进程/CLI 开销）——「回合进行中」窗口按时间构造：开轮提交
 * （API）后立即的追加提交（+数百 ms）必然落在窗口内，不依赖回复内容出现
 * （native 回复经内核聚合落地，落地时点≈回合结束，不能当窗口锚）。
 */
import { expect, test } from '@playwright/test'
import {
  createSession,
  ensureSceneProject,
  ErrorCollector,
  getSession,
  resetState,
  sendMessage,
} from './helpers/index'

/** 把 0 轮占位会话的模型切到场景名（native 的可用模型来自 SEBAS_AGENT_MODELS）。 */
async function setModel(request: Parameters<typeof sendMessage>[0], key: string, model: string) {
  const resp = await request.post(`/api/sessions/${key}/model`, { data: { model_id: model } })
  expect(resp.ok(), `switch model to ${model}: HTTP ${resp.status()}`).toBe(true)
}

/** composer 提交（操作者路径）：聚焦会话的 follow-up 输入 + Enter。 */
async function composerSubmit(page: import('@playwright/test').Page, text: string): Promise<void> {
  const input = page.locator('sebas-workbench-composer wa-textarea textarea')
  await input.fill(text)
  await input.press('Enter')
}

test.describe('native 转录补全与影子队列（fix-webui-qa-round8）', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('用户气泡与模型切换留痕', () => {
    test('native 会话发消息见用户气泡（先于回复），中程切模型落留痕条目', async ({ page }) => {
      await resetState(page.request)
      await ensureSceneProject(page.request)
      const key = await createSession(page.request, { prompt: null, agent: 'native' })
      await page.goto(`/sessions/${key}`)

      const tag = `bubble-${Date.now()}`
      await composerSubmit(page, tag)

      // 用户气泡先于该回合的回复（spec scenario「在该回合回复之前出现这条
      // 消息的用户气泡，内容与提交原文一致」）。断言锚定提交原文自己的
      // 气泡—回复对，而非「转录首个回合块」：native 占位会话的创建会以空
      // prompt 拉起一个种子回合（既有形态：spawn 的首 prompt 驱动首轮，
      // prompt=null 即空轮回——curl 可复现：entry0 空 prompt + 空回显），
      // 它先于操作者提交在场，不属本需求判据。
      const transcript = page.locator('sebas-transcript-view')
      await expect
        .poll(async () => {
          const blocks = await transcript.locator('.turn-block').evaluateAll((els) =>
            els.map((el) => ({ cls: el.className, text: el.textContent ?? '' })),
          )
          const user = blocks.findIndex((b) => b.cls.includes('is-user') && b.text.includes(tag))
          const reply = blocks.findIndex(
            (b) => b.cls.includes('is-assistant') && b.text.includes(tag),
          )
          return user >= 0 && reply >= 0 && user < reply
        }, { timeout: 60_000 })
        .toBe(true)

      // 模型切换留痕（native override 路径）：条目含新旧模型名——切换前的
      // 当前模型 = 装配注入的默认模型 test/text。
      await setModel(page.request, key, 'test/long')
      const entry = page.locator('sebas-transcript-view [data-testid="model-change-entry"]')
      await expect(entry).toBeVisible({ timeout: 15_000 })
      await expect(entry).toContainText('test/text')
      await expect(entry).toContainText('test/long')

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('排队提交在 native 会话可见', () => {
    test('回合中追加提交进待执行栈；移除同步 API 真源；留下的条目回合结束后执行', async ({
      page,
    }) => {
      // 执行断言的轮次预算：opener + queued 两轮长文 + 移除相位一轮。
      test.setTimeout(180_000)
      await resetState(page.request)
      await ensureSceneProject(page.request)
      const key = await createSession(page.request, { prompt: null, agent: 'native' })
      await setModel(page.request, key, 'test/long')
      await page.goto(`/sessions/${key}`)

      const transcript = page.locator('sebas-transcript-view')
      const stack = page.locator('sebas-pending-stack [data-testid="pending-stack"]')

      // ── 开轮：test/long 的 SSE 滴流给一个 ≥2s 的在飞窗口（另加内核首启
      // 开销）。开轮提交的 prompt 气泡即时可见（提交即落账）＝提交通路就绪；
      // 随后的追加提交（+数百 ms）必然仍在窗口内。──
      await sendMessage(page.request, key, 'long turn opener')
      await expect(
        transcript.locator('.turn-block.is-user', { hasText: 'long turn opener' }),
      ).toBeVisible({ timeout: 15_000 })

      // 回合进行中追加提交（操作者路径）：待执行栈出现该提交的条目，排队
      // 提交自己的用户气泡同时可见（prompt 条目随提交落账）。
      const queuedTag = `queued-${Date.now()}`
      await composerSubmit(page, queuedTag)
      await expect(stack).toBeVisible({ timeout: 5_000 })
      await expect(stack).toContainText(queuedTag)
      await expect(transcript.locator('.turn-block.is-user', { hasText: queuedTag })).toBeVisible()

      // ── 回合结束后队列按序执行（API 真源）：test/long 每个执行完的回合
      // 恰落一条「🗒 turn summary」条目。0-turn 占位（prompt: null）不产
      // 种子回合（空首轮不开模型轮、无 summary——review 补修期 API 级实证：
      // 转录首条就是 model_change + opener prompt，无种子痕迹），所以恰为
      // 2：opener + queued。影子队列按终态+summary 帧推进清空、栈退场；
      // 宿主队列逐条出队投递保证 pending[0]（queuedTag）先执行。──
      const summaryCount = async () => {
        const d = await getSession(page.request, key)
        return (d.detail?.entries ?? []).filter((e) => e.content.includes('🗒 turn summary'))
          .length
      }
      await expect
        .poll(async () => (await getSession(page.request, key)).detail?.pending?.length ?? 0, {
          timeout: 30_000,
        })
        .toBe(0)
      await expect(stack).toHaveCount(0, { timeout: 30_000 })
      await expect.poll(summaryCount, { timeout: 120_000 }).toBeGreaterThanOrEqual(2)

      // 操作者可见面：长文正文上屏（substring 断言——流式 delta 的分块聚合
      // 形态不进判据）。
      await page.reload()
      await expect(
        transcript.getByText(
          'abcdefghijklmnopqrstuvwxyz abcdefghijklmnopqrstuvwxyz abcdefghijklmnopqrstuvwxyz abcdefghijklmnopqrstuvwxyz',
        ).first(),
      ).toBeVisible({ timeout: 30_000 })

      // ── 移除相位（放在执行断言之后：影子队列的 remove 只撤栈上条目，
      // 内核侧串行队列不可寻址——被移提交是否仍执行不属本断言）。开新一轮，
      // 提交一条并从栈上移除：条目退场，API 真源清空（与 ACP 面同契约）。──
      await sendMessage(page.request, key, 'long turn for remove')
      await expect(
        transcript.locator('.turn-block.is-user', { hasText: 'long turn for remove' }),
      ).toBeVisible({ timeout: 15_000 })
      const doomedTag = `doomed-${Date.now()}`
      await composerSubmit(page, doomedTag)
      await expect(stack).toBeVisible({ timeout: 5_000 })
      await expect(stack.locator('.entry', { hasText: doomedTag })).toBeVisible({ timeout: 5_000 })
      await stack.locator('.entry', { hasText: doomedTag }).locator('.remove').click()
      await expect(stack.locator('.entry', { hasText: doomedTag })).toHaveCount(0, {
        timeout: 10_000,
      })
      await expect
        .poll(
          async () =>
            (await getSession(page.request, key)).detail?.pending?.map((p) => p.text) ?? [],
          { timeout: 10_000 },
        )
        .toEqual([])

      expect(collector.clean()).toEqual([])
    })
  })
})
