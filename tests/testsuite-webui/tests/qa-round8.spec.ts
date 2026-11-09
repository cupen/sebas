/**
 * fix-webui-qa-round8 的三条契约留痕/管理面旅程（默认沙箱装配，端口 9899，
 * ACP 载体 = fake-claude）：
 *
 * 1. permission-flow「升级决策的可见降级」（tasks 1.x 浏览器半边）：ACP
 *    审批卡提交「上抛」→ 转录落 `escalate_downgrade` 中性系统条目（降级
 *    语义 + 工具名 + 操作者原因原文），工具照常执行——降级不再静默；
 *    顺带钉住 agent-workbench「编码会话标识展示友好化」的审批面板展示位
 *    （4.4）：卡片会话 chip 是「渠道 · 本地段」，不带 `%00`。
 *
 * 2. agent-workbench「会话内模型切换留痕」（tasks 5.2 浏览器半边，ACP）：
 *    中程切模型成功后转录落 `model_change` 系统条目（含新旧模型名）。
 *    native 半边在 qa-round8-native.spec.ts（native 装配）。
 *
 * 3. router-model-aliases「模型别名管理有 WebUI 入口」（tasks 6.1 浏览器
 *    半边）：设置 → 「别名」分区 CRUD 一轮——新建（名称 + provider +
 *    可选上游模型）、刷新后仍在、编辑、删除（确认步骤），列表与后端一致。
 *
 * escalate 的确定性基座与 permission.spec.ts 同款：`perm` 触发词经
 * hook_callback 闸门泊车出卡，页面先落座（WS 在场）再触发。
 */
import { expect, test } from '@playwright/test'
import {
  createSession,
  ErrorCollector,
  FocusedSession,
  getSession,
  pickSelectOption,
  resetState,
  ReviewCards,
  SettingsModal,
  waitStatus,
} from './helpers/index'

test.describe('fix-webui-qa-round8 契约留痕与管理面', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('升级决策的可见降级（permission-flow）', () => {
    test('ACP 审批卡提交「上抛」：转录落降级留痕条目（含原因），工具照常执行', async ({
      page,
    }) => {
      const detail = new FocusedSession(page)
      const cards = new ReviewCards(page)

      await resetState(page.request)
      const key = await createSession(page.request, { prompt: 'idle' })
      await waitStatus(page.request, key, ['done'])
      await page.goto(`/sessions/${key}`)
      await expect(detail.host).toBeVisible()
      await detail.sendFollowUp('perm')

      await expect(cards.all().first()).toBeVisible({ timeout: 15_000 })

      // 审批卡由 React 以 role=alertdialog（aria-label 点名工具）呈现。
      await expect(cards.card()).toHaveAccessibleName(/待审批 Bash/)
      // React 审批卡尚未提供「上抛 + 原因」控件（实现缺口）：按语义寻找原因
      // 输入与上抛按钮；缺席时本断言如实失败，不删除旅程。
      const reason = cards.card().getByRole('textbox', { name: /原\s*因/ })
      await expect(reason).toBeVisible({ timeout: 10_000 })
      await reason.fill('need sudo once')
      await cards.card().getByRole('button', { name: /上\s*抛|升级/ }).click()
      await expect(cards.all()).toHaveCount(0, { timeout: 15_000 })

      // 转录的可见降级条目：中性系统条目点名降级语义、工具名与原因原文。
      const entry = page
        .getByTestId('react-transcript')
        .locator('[data-unit-key]', { hasText: '审批降级' })
      await expect(entry).toBeVisible({ timeout: 15_000 })
      await expect(entry).toContainText('仅放行一次')
      await expect(entry).toContainText('Bash')
      await expect(entry).toContainText('need sudo once')

      // 工具执行有痕：降级后的 allow_once 真的放行——'perm done' 落折叠体，
      // 回合照常收尾。
      await detail.expectFoldedText('perm done')
      await detail.expectStatus('done')

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('会话内模型切换留痕（agent-workbench，ACP 半边）', () => {
    // （fix-webui-qa-round8 5.2 review 转正，2026-10-02）此前的缺陷钉子
    // （test.fail）已随引擎修复转正：apply_event 的 ModelChanged 分支收敛
    // 调用 `apply_model_changed`（映射更新 + `model_change` 条目 + publish
    // 一处），泵的流式臂与 dispatch_acp_event 直达线两条到达线都落条目，
    // 与该函数 docstring 的既有声明对齐。本旅程钉的就是「泵线也留痕」这层
    // 接线（单测直调 apply_model_changed 盖不住它）。
    test('中程切模型成功后转录落 model_change 条目，含新旧模型名', async ({ page }) => {
      await resetState(page.request)
      const key = await createSession(page.request, { prompt: 'model entry' })
      await waitStatus(page.request, key, ['done'])

      // 先等 wire 帧观察覆盖 spawn 初值（fake-claude 自报 'fake'）——
      // 切换条目的 from 才有确定值。
      await expect
        .poll(async () => (await getSession(page.request, key)).detail?.current_model, {
          timeout: 15_000,
          intervals: [250],
        })
        .toBe('fake')

      const resp = await page.request.post(`/api/sessions/${key}/model`, {
        data: { model_id: 'opus' },
      })
      expect(resp.ok()).toBe(true)
      await expect
        .poll(async () => (await getSession(page.request, key)).detail?.current_model, {
          timeout: 15_000,
          intervals: [250],
        })
        .toBe('opus')

      // 转录留痕：模型已切换 fake → opus（fake-claude 的 init 观察若也落
      // 一条 from=null 的条目，它不含 opus——按 hasText 精确锁定本次切换）。
      await page.goto(`/sessions/${key}`)
      const entry = page
        .getByTestId('react-transcript')
        .locator('[data-unit-key]', { hasText: 'opus' })
        .filter({ has: page.getByText('模型', { exact: true }) })
      await expect(entry).toBeVisible({ timeout: 15_000 })
      await expect(entry).toContainText('fake')
      await expect(entry).toContainText('opus')

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('全部会话页有固定入口（agent-workbench 4.3）', () => {
    test('项目树的「历史」组头链接区导航到 /sessions 总览页（归档为空也在场）', async ({
      page,
    }) => {
      await resetState(page.request)
      await page.goto('/')
      // React 壳把总览入口放在主导航（常驻，归档为空也可达）。
      const link = page
        .getByRole('navigation', { name: '主导航' })
        .getByRole('button', { name: '会话总览' })
      await expect(link).toBeVisible({ timeout: 10_000 })
      await link.click()
      // SPA 路由（history pushState）：总览页宿主上屏即达。
      await expect(page.getByTestId('sessions-page')).toBeVisible({ timeout: 10_000 })

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('模型别名管理有 WebUI 入口（router-model-aliases）', () => {
    /** 本旅程自建的别名（前缀清理：重跑幂等，也不留给后续旅程脏数据）。 */
    const SEED = ['qa-alias-a', 'qa-alias-b']
    /** 别名的目标 provider（沙箱 provider store 起点为空，先经 API 播种）。 */
    const SRC_PROVIDER = 'qa-alias-src'

    async function removeAlias(request: import('@playwright/test').APIRequestContext, name: string) {
      await request.delete(`/api/model-aliases/${name}`)
    }

    /** 读面真源：providers 快照的 model_aliases 段。 */
    async function aliases(request: import('@playwright/test').APIRequestContext) {
      const resp = await request.get('/api/providers')
      expect(resp.ok()).toBe(true)
      const body = (await resp.json()) as {
        model_aliases?: Record<string, { provider: string; upstream_model?: string }>
      }
      return body.model_aliases ?? {}
    }

    /**
     * 播种/清场别名指向的 provider（models.spec 同款 API 路径；store 起点为
     * 空，新建/编辑校验要求 provider 已存在于 store）。
     */
    async function seedProvider(request: import('@playwright/test').APIRequestContext) {
      await request.delete(`/api/providers/${SRC_PROVIDER}`)
      const created = await request.post('/api/providers', {
        data: { name: SRC_PROVIDER },
      })
      expect(created.ok(), 'seed provider for alias validation').toBe(true)
    }

    /**
     * 编辑器里保存（provider 选择走 [role="combobox"] 的 value + change 事件）。
     * `name` 只在新建时填——编辑态名称即主键、输入框 disabled（fill 会等
     * 可编辑性直到超时）。
     */
    async function fillAndSave(
      page: import('@playwright/test').Page,
      opts: { name?: string; provider: string; upstream: string },
    ) {
      const host = page.getByRole('region', { name: '设置' })
      if (opts.name !== undefined) {
        await host.getByLabel('别名', { exact: true }).fill(opts.name)
      }
      await pickSelectOption(page, host.getByRole('combobox', { name: '别名 provider' }), opts.provider)
      if (opts.upstream) {
        await host.getByLabel('上游模型').fill(opts.upstream)
      }
      await host.getByRole('button', { name: /新建别名|保\s*存/ }).click()
    }

    test('新建、刷新后仍在、编辑、删除（带确认）一轮，列表与后端一致', async ({ page }) => {
      // CRUD 一轮 + 中途 reload：超过主配置 30s 默认预算。
      test.setTimeout(90_000)
      const settings = new SettingsModal(page)
      const request = page.request

      // 重跑幂等：先清掉本旅程的自建别名，并播种别名指向的 provider。
      for (const name of SEED) await removeAlias(request, name)
      await seedProvider(request)

      await page.goto('/')
      await settings.openViaSidebar()
      await settings.openSection('别名')

      const host = page.getByRole('region', { name: '设置' })
      // 沙箱起点无别名 → 空态在场（分区入口本身可见可导航）。
      await expect(host.getByText('还没有别名')).toBeVisible({ timeout: 10_000 })

      // ── 新建：名称 + 目标 provider + 可选上游模型。──
      await fillAndSave(page, {
        name: 'qa-alias-a',
        provider: SRC_PROVIDER,
        upstream: 'claude-opus-4-6',
      })
      const aliasRow = (name: string) =>
        host.getByText(name, { exact: true }).locator('xpath=../..')
      const rowA = aliasRow('qa-alias-a')
      await expect(rowA).toBeVisible({ timeout: 10_000 })
      await expect(rowA).toContainText(SRC_PROVIDER)
      await expect(rowA).toContainText('claude-opus-4-6')
      await expect
        .poll(async () => Object.keys(await aliases(request)), { timeout: 10_000 })
        .toContain('qa-alias-a')

      // ── 刷新后仍在（列表重新从读面拉取）。──
      await page.reload()
      await settings.openViaSidebar()
      await settings.openSection('别名')
      await expect(
        page
          .getByRole('region', { name: '设置' })
          .getByText('qa-alias-a', { exact: true })
          .locator('xpath=../..'),
      ).toBeVisible({ timeout: 10_000 })

      // ── 编辑：换上游模型后保存，行与后端一致。──
      // React 别名面板尚无「编辑别名」入口（实现缺口）：按语义寻找行内编辑
      // 按钮；缺席时如实失败。
      await host
        .getByText('qa-alias-a', { exact: true })
        .locator('xpath=../..')
        .getByRole('button', { name: /编辑别名 qa-alias-a/ })
        .click()
      await fillAndSave(page, {
        provider: SRC_PROVIDER,
        upstream: 'claude-sonnet-4-5',
      })
      await expect(
        host.getByText('qa-alias-a', { exact: true }).locator('xpath=../..'),
      ).toContainText('claude-sonnet-4-5', { timeout: 10_000 })
      await expect
        .poll(
          async () => (await aliases(request))['qa-alias-a']?.upstream_model ?? '',
          { timeout: 10_000 },
        )
        .toBe('claude-sonnet-4-5')

      // ── 删除：React 行内直接删除（当前无确认步骤——实现缺口，按语义寻找
      // 确认对话框；缺失时如实失败）。──
      await fillAndSave(page, { name: 'qa-alias-b', provider: SRC_PROVIDER, upstream: '' })
      const rowB = host.getByText('qa-alias-b', { exact: true }).locator('xpath=../..')
      await expect(rowB).toBeVisible({ timeout: 10_000 })

      await rowB.getByRole('button', { name: /删\s*除/ }).click()
      const confirm = page.getByRole('dialog', { name: /删除别名/ })
      await expect(confirm).toContainText('qa-alias-b')
      await confirm.getByRole('button', { name: /删\s*除|确\s*认/ }).click()

      await expect(rowB).toHaveCount(0, { timeout: 10_000 })
      await expect
        .poll(async () => Object.keys(await aliases(request)), { timeout: 10_000 })
        .not.toContain('qa-alias-b')
      // 未被删的条目原位保留。
      await expect(
        host.getByText('qa-alias-a', { exact: true }).locator('xpath=../..'),
      ).toBeVisible()

      await settings.close()

      // 卫生：清场（别名 + 播种的 provider；重跑幂等由开头的清场兜底）。
      for (const name of SEED) await removeAlias(request, name)
      await request.delete(`/api/providers/${SRC_PROVIDER}`)

      expect(collector.clean()).toEqual([])
    })
  })
})
