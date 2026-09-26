/**
 * 场景模型（native）浏览器级呈现（extend-test-model-scenarios 3.10）。
 *
 * 装配：`playwright.native.config.ts`（端口 9894，`TESTSUITE_NATIVE=1`）——
 * harness 把 native 内核指向沙箱内已在跑的 debug router（端口随本装配的 webui
 * 端口派生，见 tasks.py 的 `_router_port_for`），
 * 默认模型 `test/text`、可用模型 = 九个场景 + bare `test`。默认沙箱没有这组
 * env，本文件只在 native 装配下运行。
 *
 * 覆盖：
 * - `test/tools-parallel`：审批卡片逐张呈现、各带独立 request_id 与工具名、
 *   逐张决策互不串扰，决策后回合推进到场景终文本（对应 permission-flow
 *   「并行工具调用独立 request_id」）。native 内核按响应序执行 tool_use——
 *   写类工具串行门控（卡片逐张开），只读段并行批量（web_fetch/web_search
 *   两卡同场），与 ACP 载体的「全批同场」形态不同，属载体已知差异；
 * - `test/empty`：零输出回合在 **native 载体**上也落 notice 中性条目（与 ACP
 *   载体的 empty-turn-notice.spec.ts 双载体同断言，对应会话管理「零输出回合
 *   追加通知」）。
 *
 * 3.10 的另两项按 delta spec 的「双载体」口径由既有浏览器旅程承担，账本在
 * tests/acceptance/COVERAGE.md：UI 取消 → stop-settle.spec.ts（ACP 载体）；
 * UI 切模型 → models.spec.ts（切换生效 + composer chip 呈现，ACP 载体）；
 * native 侧的取消/切换生效由进程级 journey 3.6 / 3.7 覆盖。
 */
import { expect, test } from '@playwright/test'
import {
  createSession,
  ensureSceneProject,
  ErrorCollector,
  resetState,
  sendMessage,
} from './helpers/index'

/** 把 0 轮占位会话的模型切到场景名（native 的可用模型来自 SEBAS_AGENT_MODELS）。 */
async function setModel(request: Parameters<typeof sendMessage>[0], key: string, model: string) {
  const resp = await request.post(`/api/sessions/${key}/model`, { data: { model_id: model } })
  expect(resp.ok(), `switch model to ${model}: HTTP ${resp.status()}`).toBe(true)
}

test.describe('场景模型（native）呈现（extend-test-model-scenarios）', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('并行工具调用：审批卡片各自独立', () => {
    test('test/tools-parallel：审批卡逐张呈现、各带独立 request_id，逐张决策后回合推进到终文本', async ({
      page,
    }) => {
      // 3.10 曾以 fixme 豁免（native 审批卡在 webui 里渲染不出来）：WS
      // `permission.requested` 对 native 会话携带的 `session_id` 是 ChannelKey
      // 的 JSON 字符串形，与聚焦会话键的编码形不一致，review-card 的精确匹配
      // 把事件全丢了。缺口已在源头关闭——NativeAgentBackend 的会话键编码统一
      // 走 sebas-channels 唯一实现，native 通知与 ACP 通路在 wire 上同形
      // （编码键），前端精确匹配不需要宽松化；第二层竞态（fake provider 秒回，
      // 推送先于页面挂载到达）由 native 泊车审批读模型兜底——挂载期拉取
      // `/api/sessions/{key}/approvals` 即重建卡面，与 ACP 同契约。
      await resetState(page.request)
      await ensureSceneProject(page.request)
      const key = await createSession(page.request, { prompt: null, agent: 'native', mode: 'ask' })
      await setModel(page.request, key, 'test/tools-parallel')

      // 先落到会话页（WS 在场），再提交——权限请求经 WS 推来的卡片才是被测面。
      await page.goto(`/sessions/${key}`)
      await sendMessage(page.request, key, 'parallel please')

      const cards = page.locator('sebas-review-cards .review-card')
      await expect(cards.first()).toBeVisible({ timeout: 30_000 })

      // native 内核按响应序执行 tool_use（写类串行门控，只读段并行批量）：
      // 卡片逐张出现，web 只读对（web_fetch/web_search）两卡同场。逐张决策
      // ——bash/edit 放行，其余拒绝；每张卡自己的按钮、自己的 request_id，
      // 决策互不串扰。
      const seenIds = new Set<string>()
      const seenTools = new Set<string>()
      const transcript = page.locator('sebas-transcript-view')
      const terminalText = 'test provider: tool loop complete.'
      // getByText 穿透 shadow DOM（宿主元素的 textContent 只含 light DOM），
      // 折叠块的内容在 DOM 里即算在场——与 toContainText 同口径。
      const roundCompleted = () => page.getByText(terminalText).count().then((n) => n > 0)

      while (!(await roundCompleted())) {
        const count = await cards.count()
        if (count === 0) {
          await page.waitForTimeout(200)
          continue
        }
        const card = cards.first()
        const id = (await card.getAttribute('data-request-id')) ?? ''
        const tool = (await card.locator('.tool').innerText()).trim()
        expect(id.length, `card for ${tool} carries a request id`).toBeGreaterThan(0)
        expect(seenIds.has(id), `request id ${id} must be unique across the round`).toBe(false)
        seenIds.add(id)
        seenTools.add(tool)
        const allow = tool === 'bash' || tool === 'edit'
        // 决策按钮是 Web Awesome 自定义元素（与 permission.spec.ts 的
        // page-object 选择器同款：wa-button.allow-once / wa-button.deny）。
        await card.locator(allow ? 'wa-button.allow-once' : 'wa-button.deny').click()
        // 决策已投递：这张卡退场（墓碑），不阻塞下一张卡的出现。
        await expect(
          page.locator(`sebas-review-cards .review-card[data-request-id="${id}"]`),
        ).toHaveCount(0, { timeout: 15_000 })
      }

      // 稳定子集：bash（策略 Ask）+ 两个网络工具（只读段并行门控，Deny +
      // 升级）必出卡；write/edit 也各开一张，不钉总数。
      for (const tool of ['bash', 'web_fetch', 'web_search']) {
        expect(seenTools, `card for ${tool}`).toContain(tool)
      }
      expect(seenIds.size, 'every card had an independent request id').toBeGreaterThanOrEqual(3)

      // 全部决策后回合推进到场景终文本（并行 tool_use → 全部 tool_result →
      // 次轮终文本）。
      await expect(transcript).toContainText(terminalText, { timeout: 60_000 })
      expect(collector.pageErrors).toEqual([])
      expect(collector.consoleErrors).toEqual([])
    })
  })

  test.describe('零输出回合：native 载体的 notice 呈现', () => {
    test('test/empty：回合无可见输出，时间线落 notice 中性条目（与 ACP 载体同断言）', async ({
      page,
    }) => {
      await resetState(page.request)
      await ensureSceneProject(page.request)
      const key = await createSession(page.request, { prompt: null, agent: 'native' })
      await setModel(page.request, key, 'test/empty')

      await page.goto(`/sessions/${key}`)
      await sendMessage(page.request, key, 'say nothing')

      // native 会话的状态投影恒为 `queued`（无 mode/turn_engaged 事实），不能等
      // `done`——直接等被测面：notice 条目出现。
      const notice = page.locator('sebas-transcript-view [data-testid="notice-entry"]')
      await expect(notice).toBeVisible({ timeout: 30_000 })
      await expect(notice).toContainText('回合已结束且无输出')

      // 零输出回合不再不可见地消失：notice 条目在场 + 助手回合照常收尾（收尾标记
      // 落在 notice 之后，与进程级 journey 的落点序一致）。native 的转录**不投影
      // 操作者 prompt**（无 prompt 条目），所以这里不断言提交原文——ACP 载体的
      // empty-turn-notice.spec.ts 才断言 prompt 块。
      const transcript = page.locator('sebas-transcript-view')
      await expect(transcript.locator('.turn-block.is-assistant').last()).toContainText(
        'turn summary',
      )
      // native 泊车审批读模型补齐后，挂载期的 `/api/sessions/{key}/approvals`
      // 拉取返回 200（无泊车 = 空表），不再有旧注释里的 503 网络噪声——
      // 控制台错误与 JS 异常照旧零容忍。
      expect(collector.pageErrors).toEqual([])
      expect(collector.consoleErrors).toEqual([])
    })
  })
})
