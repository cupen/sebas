/**
 * Journey — 工具调用合并块（fold-tool-calls-into-process-tree 6.2/6.3/7.3，
 * spec agent-workbench「Tool call blocks merge invocation and result」）。
 *
 * 装配：playwright.native.config.ts（端口 9894，TESTSUITE_NATIVE=1）——native
 * 内核指向沙箱内已在跑的 debug router，场景模型 `test/tool-use` /
 * `test/tools-parallel` 提供确定性工具环（tool_use id 形态 `toolu_test_1` /
 * `toolu_test_p<i>`，**互异**——design「Open Questions」经读场景模型确认）。
 *
 * 载体事实（review F1 已修复）：native 内核对被门控的调用会把
 * `⏳ … awaits approval` / `🛡 … policy` 两条 **markdown** 转录条目落在调用
 * 与结果之间——splitAgentRuns 按 kind 变化切 run，调用与结果条目因此分属
 * 两个过程 run。foldCrossRunToolResults 在回合范围内按 tool_use_id 把结果
 * 搬回调用所在 run（spec「pairing SHALL NOT rely on arrival position」），
 * 搬空的 run 丢弃——被门控的 native 调用同样呈现为**一块**（合并块 +
 * 汇总行 ✓/✗ 章），与 ACP 载体契约一致。
 *
 * 各用例口径：
 * - test/tool-use（必被门控：场景命令 `mkdir -p .sebas-probe` 策略判 Ask）：
 *   7.2 三件齐（API 面 element_type=tool + 结构化标题 + tool_use_id）；
 *   单折叠合并块：汇总行与块收起行双 ✓ 章、展开体参数段+结果段一次可达；
 *   层级不死锁（D7）；7.3 刷新形态不变。
 * - test/tools-parallel：一回合多工具各成**一块**、按 id 精确配对（API 面：
 *   每个 id 恰好 调用+结果 两条且同名）、段数守恒（参数段+结果段 =
 *   2×调用数，零丢失零重复）、标题互不相同、拒绝决策收起态可辨（✗）。
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

const TERMINAL_TEXT = 'test provider: tool loop complete.'

/** `test/tool-use` 场景的确定性命令（agent_loop GATED_COMMAND_PREFIX，策略判 Ask）。 */
const GATED_COMMAND = 'mkdir -p .sebas-probe'

test.describe('工具调用合并块（fold-tool-calls-into-process-tree，native 装配）', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('单工具环：test/tool-use（必被门控 → 跨 run 重聚为一块）', () => {
    test('工具条目三件齐；单折叠合并块：汇总行+块行双 ✓ 章、展开参数+结果一次可达；层级不死锁；刷新形态不变', async ({
      page,
    }) => {
      await resetState(page.request)
      await ensureSceneProject(page.request)
      const key = await createSession(page.request, { prompt: null, agent: 'native', mode: 'ask' })
      await setModel(page.request, key, 'test/tool-use')
      await page.goto(`/sessions/${key}`)
      await sendMessage(page.request, key, 'tool please')

      // 权限卡在场后放行（挂载期竞态由 native 泊车审批读模型兜底，同
      // test-model-scenarios 的口径）；回合推进到场景终文本。
      const card = page.locator('[aria-label="待审批工具调用"] [role="alertdialog"]').first()
      await expect(card).toBeVisible({ timeout: 30_000 })
      await card.locator('button.allow-once').click()
      const transcript = page.locator('[data-testid="react-transcript"]')
      await expect(transcript).toContainText(TERMINAL_TEXT, { timeout: 60_000 })

      // ── 7.2 API 面：工具条目三件齐（element_type=tool + 结构化标题 +
      // tool_use_id），调用与结果同 id。
      const { detail } = await getSession(page.request, key)
      const toolEntries = (detail?.entries ?? []).filter((e) => e.element_type === 'tool')
      expect(toolEntries.length, 'invocation + result entry land as first-class tool').toBe(2)
      const [invocation, result] = toolEntries
      expect(invocation.title, 'invocation title = tool name + key arg').toBe(
        `bash · ${GATED_COMMAND}`,
      )
      expect(invocation.tool_use_id, 'invocation carries the upstream call id').toMatch(
        /^toolu_test_/,
      )
      expect(invocation.content).toContain(GATED_COMMAND)
      expect(result.title, 'result title = ✓ tool（ToolEnd wire 无 args 的退化形态）').toBe(
        '✓ bash',
      )
      expect(result.tool_use_id, 'result pairs with its invocation by id').toBe(
        invocation.tool_use_id,
      )
      expect(result.content.length, 'result content non-empty').toBeGreaterThan(0)

      // ── GUI 面。回合收束后默认全部收起；单棵过程树：调用与结果按 id
      // 跨 run 重聚（F1 修复）——整个回合只剩调用所在的一个过程折叠，
      // 泊车/决策正文 run 之后的孤儿结果折叠不复存在。
      const assistantTurn = page
        .locator('[data-testid="react-transcript"] [data-unit-key]:has-text("agent")')
        .first()
      await expect(assistantTurn).toBeVisible()
      const folds = assistantTurn.locator('details')
      await expect(folds).toHaveCount(1, { timeout: 15_000 })
      // 单棵过程树：过程条目只在折叠体内，不存在顶层结果块。
      const orphanItems = await assistantTurn
        .locator('details > div > div')
        .evaluateAll((els) => els.filter((el) => el.closest('details') === null).length)
      expect(orphanItems, 'no process item outside the fold tree').toBe(0)

      // 汇总行（收起即可读）：计数 = 调用+结果两条，摘要跟踪 run 尾。
      const fold = folds.nth(0)
      const summary = fold.locator('summary')
      await expect(fold).not.toHaveAttribute('open', '')
      await expect(summary).toContainText('2 项')
      await expect(summary).toContainText('bash')

      // 展开父折叠（到达，纯导航）：逐个过程条目在场，内容含参数与结果。
      await summary.click()
      await expect(fold).toHaveAttribute('open', '')
      const items = fold.locator(':scope > div > div')
      await expect(items).toHaveCount(2)
      await expect(fold).toContainText(GATED_COMMAND)
      await expect(fold.locator('pre')).not.toHaveCount(0)

      // 父折叠保持展开：React 只有一层过程树，条目内容默认随展开可见。
      await expect(fold).toHaveAttribute('open', '')

      // 7.3 刷新一致性：配对是纯派生——刷新后单折叠、条目、摘要全部不变。
      await page.reload()
      const reloadedFolds = page
        .locator('[data-testid="react-transcript"] [data-unit-key]:has-text("agent")')
        .first()
        .locator('details')
      await expect(reloadedFolds).toHaveCount(1, { timeout: 15_000 })
      const reloadedSummary = reloadedFolds.nth(0).locator('summary')
      await expect(reloadedSummary).toContainText('2 项')
      await reloadedSummary.click()
      await expect(reloadedFolds.nth(0)).toContainText(`bash · ${GATED_COMMAND}`)

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('并行工具环：test/tools-parallel', () => {
    test('一回合多工具各成一块、按 id 精确配对、段数守恒、标题互异、拒绝收起可辨', async ({
      page,
    }) => {
      await resetState(page.request)
      await ensureSceneProject(page.request)
      const key = await createSession(page.request, { prompt: null, agent: 'native', mode: 'ask' })
      await setModel(page.request, key, 'test/tools-parallel')
      await page.goto(`/sessions/${key}`)
      await sendMessage(page.request, key, 'parallel please')

      // 场景为每个声明工具发一个 tool_use（id 互异 `toolu_test_p<i>`，id 即权限
      // request_id）。逐张决策：放行 bash/edit/write，其余拒绝（同
      // test-model-scenarios 的口径）——已执行与已拒绝两形态同时在场。
      const cards = page.locator('[aria-label="待审批工具调用"] [role="alertdialog"]')
      // getByText 穿透 shadow DOM（同 test-model-scenarios 的口径）。
      const roundCompleted = () =>
        page
          .getByText(TERMINAL_TEXT)
          .count()
          .then((n) => n > 0)
      while (!(await roundCompleted())) {
        const count = await cards.count()
        if (count === 0) {
          await page.waitForTimeout(200)
          continue
        }
        const card = cards.first()
        const id = (await card.getAttribute('data-request-id')) ?? ''
        const tool = (await card.locator('strong').innerText()).trim()
        expect(id.length, `card for ${tool} carries a request id`).toBeGreaterThan(0)
        const allow = tool === 'bash' || tool === 'edit' || tool === 'write'
        await card.locator(allow ? 'button.allow-once' : 'button.deny').click()
        await expect(
          page.locator(`[aria-label="待审批工具调用"] [role="alertdialog"][data-request-id="${id}"]`),
        ).toHaveCount(0, { timeout: 15_000 })
      }

      // ── API 面：native 六件套 + 并行场景 = 每个声明工具一个调用，配对键
      // = tool_use_id。每个 id 恰好两条（调用 + 结果）且同名——并行同名/
      // 异名都不靠位置猜。
      const { detail } = await getSession(page.request, key)
      const toolEntries = (detail?.entries ?? []).filter((e) => e.element_type === 'tool')
      const byId = new Map<string, typeof toolEntries>()
      for (const e of toolEntries) {
        const id = e.tool_use_id ?? ''
        byId.set(id, [...(byId.get(id) ?? []), e])
      }
      const ids = [...byId.keys()]
      expect(
        toolEntries.length,
        `every tool entry carries an id (${toolEntries.length} entries)`,
      ).toBe(toolEntries.filter((e) => e.tool_use_id).length)
      expect(ids.length, 'one call per declared tool').toBeGreaterThanOrEqual(8)
      for (const id of ids) {
        const pair = byId.get(id) ?? []
        expect(pair, `call ${id} has exactly invocation + result`).toHaveLength(2)
        const invocation = pair.find((e) => !/^[✓✗]/.test((e.title ?? '').trim()))
        const result = pair.find((e) => /^[✓✗]/.test((e.title ?? '').trim()))
        expect(invocation, `call ${id} invocation entry`).toBeTruthy()
        expect(result, `call ${id} result entry`).toBeTruthy()
        const invTool = (invocation!.title ?? '').split('·')[0].trim()
        const resultTool = (result!.title ?? '').replace(/^[✓✗]\s*/, '').trim()
        expect(resultTool, `call ${id} result pairs with its own invocation`).toBe(invTool)
      }

      // ── GUI 面：只展开**父折叠**（直接子级 link；子块 link 也叫
      // fold-link，不能混进来，否则后面的段数循环首击会变成收起），断言段
      // 数守恒：每个调用贡献参数段 + 结果段各一——F1 修复后被门控调用的
      // 结果也按 id 重聚回调用块，每调用恰一块，零丢失零重复。
      const assistantTurn = page
        .locator('[data-testid="react-transcript"] [data-unit-key]:has-text("agent")')
        .first()
      // React 的过程折叠是原生 details/summary；全部展开后按 run 数块。
      const allSummaries = assistantTurn.locator('details:not([open]) > summary')
      for (let guard = 0; guard < 60; guard += 1) {
        if ((await allSummaries.count()) === 0) break
        await allSummaries.first().click()
      }
      const blocks = assistantTurn.locator('details')
      const blockCount = await blocks.count()
      expect(blockCount, 'each call renders exactly ONE merged block').toBe(ids.length)
      // 单棵过程树：所有过程条目都在 details 内（无顶层结果块）。
      const orphans = await assistantTurn
        .locator('details > div > div')
        .evaluateAll((els) => els.filter((el) => el.closest('details') === null).length)
      expect(orphans, 'no top-level process item outside the fold tree').toBe(0)

      let sectionCount = 0
      let mergedBlocks = 0
      const titles: string[] = []
      for (let i = 0; i < blockCount; i += 1) {
        const b = blocks.nth(i)
        const summary = b.locator('summary')
        titles.push(((await summary.innerText()) || '').trim())
        if (!(await b.getAttribute('open'))) await summary.click()
        const items = b.locator(':scope > div > div')
        const itemCount = await items.count()
        const hasArgs = (await b.locator('pre').count()) > 0
        const hasResult = (await b.innerText()).includes('✓') || (await b.innerText()).includes('✗')
        expect(hasArgs || hasResult, `block ${titles[i]} renders at least one section`).toBe(
          true,
        )
        sectionCount += itemCount
        if (itemCount >= 2) mergedBlocks += 1
      }
      expect(
        sectionCount,
        'args + result sections are conserved across blocks (2 per call)',
      ).toBe(ids.length * 2)
      expect(mergedBlocks, 'every call merges into ONE block (cross-run re-homing, F1 fixed)').toBe(
        blockCount,
      )
      expect(
        new Set(titles).size,
        `block titles distinct: ${titles.join(' | ')}`,
      ).toBe(titles.length)

      // bash 块的参数就是自己的命令（互不串扰的最小确定性钉子）：标题里的
      // 关键参数与展开参数段一致，且各块命令后缀互异（场景契约 -<index>）。
      const bashBlocks = blocks.filter({ hasText: 'mkdir -p .sebas-probe' })
      const bashCount = await bashBlocks.count()
      const bashArgs = new Set<string>()
      for (let i = 0; i < bashCount; i += 1) {
        const b = bashBlocks.nth(i)
        if (!(await b.getAttribute('open'))) await b.locator('summary').click()
        const text = await b.innerText()
        bashArgs.add(text.match(/mkdir -p \.sebas-probe-\d+/)?.[0] ?? '')
      }
      expect(bashArgs.size, `bash call args are per-call distinct: ${[...bashArgs]}`).toBe(
        bashCount,
      )

      // 拒绝决策的块展开体挂 ✗ 章（permission-flow「拒绝结果可辨识」）。
      const deniedRows = assistantTurn.getByText(/✗/)
      expect(await deniedRows.count(), 'denied calls carry ✗').toBeGreaterThan(0)

      // D7 层级不死锁：React 只有一层过程折叠；展开任一折叠不影响其余折叠
      // 的开态（逐个守门）。
      const multiFold = blocks.last()
      const multiSummary = multiFold.locator('summary')
      if (!(await multiFold.getAttribute('open'))) await multiSummary.click()
      await expect(multiFold).toHaveAttribute('open', '')
      await expect(blocks.first()).toHaveAttribute('open', '')

      expect(collector.clean()).toEqual([])
    })
  })
})
