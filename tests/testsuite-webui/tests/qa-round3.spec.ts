/**
 * Journey — fix-webui-qa-round3 review 补层（review 阶段新增，只加测试不改实现）。
 *
 * 单测（790 基线）钉的是源码合同（样式表文本、纯函数、handler 接线）；这里
 * 补浏览器层，逐条对应 change 的 delta 需求：
 *
 *   - D2 mode 下拉连续指针交互（webui MODIFIED「会话 mode 在 dashboard 可见
 *     可切」场景「下拉可再次展开」）：composer 工具条 + 创建对话框两处，
 *     **真实鼠标**选择一项后再次点击必须重新展开，且展开布尔与渲染面一致
 *     （open=true 时选项真实可见）——实现者声明根因未经真实浏览器复现，
 *     本旅程是它的第一手复验。
 *   - D3 /sessions 栅格 1280 收纳 + D4 /usage 控件 1280 不溢出（webui
 *     「Session dashboard and focus semantics」场景「sessions grid stays
 *     usable at 1280」+ usage-statistics 场景「controls stay inside the
 *     container」）：scrollWidth ≤ clientWidth、卡片操作与刷新按钮 rect
 *     都在视口内。
 *   - D8 统计卡随窗口重算（usage-statistics 场景「summary cards follow the
 *     selected window」）：载荷顶层 totals **漂移**时卡片只认 buckets 现场累加；
 *     切到全零小时窗后卡片与空态一并翻转。
 *   - D6 聚焦头部改名即时同步（webui「rail 与头部同一标题源」的浏览器面）。
 *   - D7 History 条目呈现归档时刻现用标签（agent-workbench「History group is
 *     the archive」场景「History shows the current label at archive time」
 *     +「restore keeps the label」）：行名、只读视图头、恢复后的 rail 行。
 *   - D9 项目「…」菜单上移/下移重排序（agent-workbench「Rail project order is
 *     operator-controlled」场景 reorder entry/persists）：真实菜单点击 →
 *     顺序翻转 → 刷新保持；边界位禁用。
 *   - D10 会话选择写 URL（webui「selection writes the session URL」「reload
 *     keeps focus from URL」）：rail 选择 pushState 到 /sessions/{key} 不重载、
 *     刷新自持焦点、后退到 / 不白屏不重挂、未知键如实降级。
 *   - D1 thinking 折叠缺省形态可区分（agent-workbench「Workbench renders the
 *     focused session」场景「thinking fold is titled and distinguishable」
 *     +「text never wears a process chip」）的**真实渲染**面：data-kind、
 *     胶囊底色（计算样式非透明）、二级 thinking glyph、正文段零 PROCESS 词。
 *
 * 与既有 journey 的分工：thinking-process-fold / mode / session-label /
 * archive-identity / projects / usage 各自钉旧的合同不动；本文件只钉 round3
 * 的新增面。所有用例维持 ErrorCollector 纪律（干净 console 收尾）。
 */
import { expect, test } from '@playwright/test'
import fs from 'node:fs'
import path from 'node:path'
import {
  createSession,
  ensureSceneProject,
  ErrorCollector,
  FocusedSession,
  getSession,
  ProjectRail,
  resetState,
  sceneDir,
  setSessionLabel,
  SettingsModal,
  waitStatus,
} from './helpers/index'

/** 转义字符串使其可安全内插进 RegExp（D10 的 URL 断言用）。 */
function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

test.describe('fix-webui-qa-round3 review 补层', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('D2 mode 下拉连续指针交互', () => {
    test('composer 工具条：真实鼠标选择一项后，再次点击重新展开且渲染面一致', async ({
      page,
      request,
    }) => {
      test.setTimeout(60_000)
      await resetState(request)
      const key = await createSession(request, { prompt: 'hello', mode: 'ask' })
      await waitStatus(request, key, ['done'])

      await page.goto(`/sessions/${key}`)
      const modeSwitch = page.locator(
        'sebas-workbench-composer wa-select[data-testid="mode-switch"]',
      )
      await expect(modeSwitch).toBeVisible()
      const allowOption = modeSwitch.locator('wa-option[value="allow"]')

      // 第一次真实鼠标展开 + 真实鼠标选中一项（此前套件只走过程序化置值）。
      await modeSwitch.click()
      await expect(allowOption).toBeVisible({ timeout: 10_000 })
      await allowOption.click()

      // 选择真实送达：POST /api/sessions/{key}/mode → desired_mode=allow。
      await expect
        .poll(async () => (await getSession(request, key)).detail?.desired_mode ?? null, {
          timeout: 10_000,
        })
        .toBe('allow')

      // 选择后列表收起，展开布尔与渲染面一致（false ↔ 选项不可见）。
      await expect(allowOption).toBeHidden()
      expect(
        await modeSwitch.evaluate((el) => (el as unknown as { open: boolean }).open),
      ).toBe(false)

      // ── D2 核心：再次以鼠标点击 → 选项列表重新展开 ─────────────────────
      await modeSwitch.click()
      await expect(allowOption).toBeVisible({ timeout: 10_000 })
      // 展开布尔与实际渲染一致（场景「下拉的展开指示与列表实际显隐一致」的
      // 行为面：open=true 且浮层真实渲染——wa-popup active、listbox 未隐藏）。
      const rendered = await modeSwitch.evaluate((el) => {
        const sel = el as unknown as { open: boolean; shadowRoot: ShadowRoot | null }
        const popup = sel.shadowRoot?.querySelector('wa-popup') as
          | (Element & { active?: boolean })
          | null
        const listbox = sel.shadowRoot?.querySelector('.listbox') as HTMLElement | null
        return {
          open: sel.open,
          popupActive: popup?.active === true,
          listboxRendered: !!listbox && !listbox.hidden,
        }
      })
      expect(rendered.open).toBe(true)
      expect(rendered.popupActive).toBe(true)
      expect(rendered.listboxRendered).toBe(true)

      expect(collector.clean()).toEqual([])
    })

    test('创建对话框：真实鼠标选择 mode 后，再次点击重新展开', async ({ page, request }) => {
      test.setTimeout(60_000)
      const log = (m: string) => console.log(`[qa-round3 D2-dialog] ${m}`)
      await resetState(request)
      const { name: projectName } = await ensureSceneProject(request)
      const rail = new ProjectRail(page)
      await page.goto('/')
      await rail.openNewSessionDialog(projectName)
      log('dialog open')

      const modeSelect = rail
        .newSessionDialog()
        .locator('[data-testid="dialog-mode-select"]')
      await expect(modeSelect).toBeVisible()
      const editOption = modeSelect.locator('wa-option[value="edit"]')

      // 真实鼠标展开 + 选中（对话框侧同样从未走过真实指针选择）。
      await modeSelect.click({ timeout: 8_000 })
      log('select clicked (1st)')
      await expect(editOption).toBeVisible({ timeout: 10_000 })
      log('options visible (1st)')
      await editOption.click({ timeout: 8_000 })
      log('option clicked')
      await expect
        .poll(
          async () =>
            await modeSelect.evaluate((el) => (el as unknown as { value: string }).value),
        )
        .toBe('edit')
      log('value=edit')

      // 选择后收起；再次鼠标点击必须重新展开（QA 实锤的第二次展开失败点）。
      await expect(editOption).toBeHidden()
      log('options hidden after select')
      await modeSelect.click({ timeout: 8_000 })
      log('select clicked (2nd)')
      await expect(editOption).toBeVisible({ timeout: 10_000 })
      log('options visible (2nd) — D2 dialog OK')

      // 收尾：先 Escape 收起弹层（开着的选择弹层会盖住取消钮，直接点取消
      // 会因遮挡点不到），再取消对话框。
      await page.keyboard.press('Escape')
      await rail.cancelNewSessionDialog()
      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('D3 /sessions 1280 收纳', () => {
    test('1280 视口：/sessions 无横向溢出，卡片操作入口完整可达', async ({
      page,
      request,
    }) => {
      test.setTimeout(60_000)
      await page.setViewportSize({ width: 1280, height: 720 })
      await resetState(request)
      const tag = `qa1280-${Date.now()}`
      const keys: string[] = []
      for (let i = 0; i < 3; i += 1) {
        const key = await createSession(request, { prompt: `${tag}-${i}` })
        await waitStatus(request, key, ['done'])
        keys.push(key)
      }

      await page.goto('/sessions')
      await expect(page.locator('sebas-sessions .page-title')).toBeVisible()
      // 栅格至少三卡（spec 场景前提「卡片不少于三张」；此前用例遗留的已关闭
      // 会话也按 known session 上列表，故断言下限而非恰数）。
      const cards = page.locator('sebas-sessions article.scard')
      await expect
        .poll(async () => await cards.count(), { timeout: 15_000 })
        .toBeGreaterThanOrEqual(3)

      // 溢出断言：outlet 不得宽出宿主 main（根因修复前 outlet 按内容盒多出
      // 64px、被宿主裁掉不可回收），main 自身不得出现横向滚动量。
      const geo = await page.evaluate(() => {
        const app = document.querySelector('sebas-app')
        const main = app?.shadowRoot?.querySelector('main')
        const outlet = main?.querySelector('.outlet')
        if (!(main instanceof HTMLElement) || !(outlet instanceof HTMLElement)) return null
        return {
          outletRight: outlet.getBoundingClientRect().right,
          mainRight: main.getBoundingClientRect().right,
          mainScrollWidth: main.scrollWidth,
          mainClientWidth: main.clientWidth,
        }
      })
      expect(geo, 'app shell main/outlet not found').not.toBeNull()
      expect(geo!.outletRight).toBeLessThanOrEqual(geo!.mainRight + 1)
      expect(geo!.mainScrollWidth).toBeLessThanOrEqual(geo!.mainClientWidth + 1)

      // 任一卡片的操作入口（Close）完整落在视口内（不被右缘裁剪）。
      const closeButtons = cards.locator('wa-button[aria-label^="关闭会话"]')
      await expect(closeButtons.first()).toBeVisible()
      for (let i = 0; i < (await closeButtons.count()); i += 1) {
        const box = await closeButtons.nth(i).boundingBox()
        expect(box, `card ${i} close button box`).not.toBeNull()
        expect(box!.x).toBeGreaterThanOrEqual(-0.5)
        expect(box!.x + box!.width).toBeLessThanOrEqual(1280.5)
      }

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('D4+D8 /usage 控件收纳与窗口口径', () => {
    const CLAUDE = {
      model: 'claude-sonnet',
      requests: 2,
      input_tokens: 10,
      output_tokens: 50,
      cache_read_tokens: 5,
      cache_creation_tokens: 2,
    }
    const GPT = {
      model: 'gpt-4o-mini',
      requests: 1,
      input_tokens: 4,
      output_tokens: 8,
      cache_read_tokens: 0,
      cache_creation_tokens: 0,
    }
    /** 天窗：桶和 3/14/58/7；顶层 totals **故意漂移**（99…）——卡片只准认桶。 */
    const DAY_DRIFT = {
      granularity: 'day',
      days: 3,
      tz_offset: 480,
      buckets: [
        { bucket: '2026-09-27', models: [] },
        { bucket: '2026-09-28', models: [CLAUDE, GPT] },
        { bucket: '2026-09-29', models: [] },
      ],
      totals: {
        model: 'total',
        requests: 99,
        input_tokens: 999,
        output_tokens: 9999,
        cache_read_tokens: 99,
        cache_creation_tokens: 99,
      },
    }
    /** 小时窗：24 个全零桶，totals 依旧漂移——空态判别也必须只认窗口。 */
    const HOUR_ZERO = {
      granularity: 'hour',
      days: 1,
      tz_offset: 480,
      buckets: Array.from({ length: 24 }, (_, h) => ({
        bucket: String(h).padStart(2, '0'),
        models: [],
      })),
      totals: DAY_DRIFT.totals,
    }

    test('1280 视口刷新按钮在视口内；卡片随窗口重算且无视漂移 totals', async ({
      page,
    }) => {
      await page.setViewportSize({ width: 1280, height: 720 })
      // 按 URL granularity 参数各答各的窗（day=漂移 totals 的新数据窗，
      // hour=全零窗）。
      await page.route(/\/api\/usage\/timeseries/, (route) => {
        const granularity = new URL(route.request().url()).searchParams.get('granularity')
        const payload = granularity === 'hour' ? HOUR_ZERO : DAY_DRIFT
        void route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify(payload),
        })
      })

      await page.goto('/usage')
      const view = page.locator('sebas-usage')
      await expect(view).toBeVisible()

      // D4：刷新按钮等控件完整落在 1280 视口内（右缘不溢出）。
      const reload = view.locator('[data-testid="usage-reload"]')
      await expect(reload).toBeVisible()
      const box = await reload.boundingBox()
      expect(box, 'usage reload button box').not.toBeNull()
      expect(box!.x).toBeGreaterThanOrEqual(-0.5)
      expect(box!.x + box!.width).toBeLessThanOrEqual(1280.5)
      expect(box!.y).toBeGreaterThanOrEqual(-0.5)
      expect(box!.y + box!.height).toBeLessThanOrEqual(720.5)

      // D8：卡片数字 = 桶现场累加（3/14/58/7），不是漂移的 totals（99…）。
      const summary = view.locator('[data-testid="usage-summary"]')
      await expect(summary.locator('.stat', { hasText: '请求数' }).locator('.num')).toHaveText(
        '3',
      )
      await expect(
        summary.locator('.stat', { hasText: '输入 tokens' }).locator('.num'),
      ).toHaveText('14')
      await expect(
        summary.locator('.stat', { hasText: '输出 tokens' }).locator('.num'),
      ).toHaveText('58')
      await expect(
        summary.locator('.stat', { hasText: '缓存 tokens' }).locator('.num'),
      ).toHaveText('7')

      // 切到全零小时窗：卡片与空态随**窗口**翻转（totals 仍漂移 99 也不上屏）。
      await view.locator('[data-testid="granularity-hour"]').click()
      await expect(view.getByText('暂无用量数据')).toBeVisible({ timeout: 10_000 })
      await expect(summary).toHaveCount(0)
      await expect(view.locator('sebas-line-chart')).toHaveCount(0)

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('D6 聚焦头部改名即时同步', () => {
    test('rail 改名后聚焦头部就地跟随（不重载，头部与 rail 同源同值）', async ({
      page,
      request,
    }) => {
      test.setTimeout(60_000)
      await resetState(request)
      const { name: projectName } = await ensureSceneProject(request)
      const tag = `headsync-${Date.now()}`
      const key = await createSession(request, { prompt: tag })
      await waitStatus(request, key, ['done'])

      await page.goto(`/sessions/${key}`)
      const rail = new ProjectRail(page)
      await rail.ensureProjectExpanded(projectName)
      // 聚焦头部先呈现首条 prompt 预览（头部命名链 = session-head-name，
      // fullSessionLabel：label → preview → 短 id；.chat 是 chat_id 不是名）。
      const headName = page.locator('sebas-dashboard [data-testid="session-head-name"]')
      await expect(headName).toBeVisible({ timeout: 15_000 })
      await expect(headName).toContainText(tag)

      // rail … 菜单改名（与 session-label.spec 同款驱动）。
      const row = rail.host
        .locator('li.session-item:not(.archived)', { hasText: tag })
        .first()
      await row.hover()
      await row.locator('wa-dropdown button[title="会话操作"]').click()
      await row.locator('wa-dropdown-item[value="rename"]').click()
      const dialog = page.locator('sebas-project-rail wa-dialog[label="重命名会话"]')
      await expect(
        dialog.locator('h2, [role="heading"]', { hasText: '重命名会话' }).first(),
      ).toBeVisible()
      const input = dialog.locator('wa-input[data-testid="rename-input"] input')
      await input.click()
      await input.pressSequentially('聚焦名A')
      await dialog.locator('wa-button').filter({ hasText: '保存' }).click()

      // 免刷新探针：头部更新必须发生在同一文档里。
      await page.evaluate(() => {
        const dash = document.querySelector('sebas-app')?.shadowRoot?.querySelector(
          'sebas-dashboard',
        )
        if (dash) (dash as unknown as { __qaNoReload?: boolean }).__qaNoReload = true
      })
      // 头部就地翻新（事件直改共享行，不等 WS 相位帧/节流刷新）；rail 行同值。
      await expect(headName).toContainText('聚焦名A', { timeout: 3_000 })
      await expect(
        rail.host
          .locator('li.session-item:not(.archived)', { hasText: '聚焦名A' })
          .first()
          .locator('.session-name'),
      ).toHaveText('聚焦名A', { timeout: 3_000 })
      expect(
        await page.evaluate(
          () =>
            (
              document.querySelector('sebas-app')?.shadowRoot?.querySelector(
                'sebas-dashboard',
              ) as unknown as { __qaNoReload?: boolean } | null
            )?.__qaNoReload === true,
        ),
      ).toBe(true)

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('D7 History 归档时刻标签', () => {
    test('改名后归档：History 行、只读视图头、恢复后的 rail 行都用现用标签', async ({
      page,
      request,
    }) => {
      test.setTimeout(90_000)
      await resetState(request)
      await ensureSceneProject(request)
      const tag = `hist-old-${Date.now()}`
      const label = `归档新名-${Date.now()}`
      const key = await createSession(request, { prompt: tag })
      await waitStatus(request, key, ['done'])
      // 归档前改名（API 半边）：operator_label 随归档快照落档。
      await setSessionLabel(request, key, label)
      await request.post(`/api/sessions/${key}/archive`)

      const rail = new ProjectRail(page)
      await page.goto('/')
      await rail.expandHistory()

      // History 行呈现归档时刻现用标签，不回退旧自动标题（首条 prompt 预览）。
      const archivedRow = rail.host
        .locator('li.session-item.archived', { hasText: label })
        .first()
      await expect(archivedRow).toBeVisible({ timeout: 10_000 })
      await expect(archivedRow.locator('.session-name')).toHaveText(label)
      expect(
        await rail.host.locator('li.session-item.archived', { hasText: tag }).count(),
      ).toBe(0)

      // 归档只读视图头同源取数。
      await archivedRow.click()
      const archivedView = page.locator('sebas-dashboard [data-testid="archived-view"]')
      await expect(archivedView).toBeVisible({ timeout: 10_000 })
      await expect(archivedView.locator('.ident .chat')).toContainText(label)

      // 恢复保留标签：回到 rail 时行名仍是归档时刻的现用标签。
      await archivedView.locator('[data-testid="archived-restore"]').click()
      await page.locator('sebas-dashboard [data-testid="restore-confirm"]').click()
      await expect(archivedView).toHaveCount(0, { timeout: 15_000 })
      await expect(
        rail.host
          .locator('li.session-item:not(.archived)', { hasText: label })
          .first()
          .locator('.session-name'),
      ).toHaveText(label, { timeout: 15_000 })

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('D9 项目重排序菜单', () => {
    test('…菜单上移/下移即时翻转顺序并跨刷新保持；边界位对应项禁用', async ({
      page,
      request,
    }) => {
      test.setTimeout(90_000)
      await resetState(request)
      const { name: sceneName } = await ensureSceneProject(request)
      // 两个一次性项目（场景目录内，出界注册会被 400 拒）。
      const t = Date.now()
      const names = [`r3-up-${t}`, `r3-down-${t}`]
      for (const n of names) {
        const dir = path.join(sceneDir(), n)
        fs.mkdirSync(dir, { recursive: true })
        const resp = await request.post('/api/projects', { data: { path: dir } })
        if (!resp.ok()) throw new Error(`addProject ${n} failed: HTTP ${resp.status()}`)
      }

      const rail = new ProjectRail(page)
      await page.goto('/')
      await expect(rail.host).toBeVisible()
      const railNames = () =>
        rail.host
          .locator('.row .name > span:first-child')
          .allTextContents()
          .then((ts) => ts.map((s) => s.trim()))
      await expect
        .poll(async () => (await railNames()).length, { timeout: 10_000 })
        .toBeGreaterThanOrEqual(3)

      /** 打开某项目行的 … 菜单（触发钮 hover 显现，同 session 菜单纪律）。 */
      const openProjectMenu = async (name: string) => {
        const row = rail.projectRow(name)
        await row.hover()
        await row.locator('button[aria-label^="项目操作"]').click()
        await expect(row.locator('wa-dropdown-item[data-testid="project-move-down"]')).toBeVisible()
        return row
      }

      // 相对断言：r3-up- 下移一格 = 与其后继互换（不依赖初始全序——同秒
      // added_at 的尾序不确定，projects.spec 1.2 已有教训）。
      const before = await railNames()
      const idx = before.indexOf(`r3-up-${t}`)
      expect(idx).toBeGreaterThanOrEqual(0)
      expect(idx).toBeLessThan(before.length - 1)
      const swapped = [...before]
      ;[swapped[idx], swapped[idx + 1]] = [swapped[idx + 1]!, swapped[idx]!]

      const row = await openProjectMenu(`r3-up-${t}`)
      await row.locator('wa-dropdown-item[data-testid="project-move-down"]').click()
      await expect.poll(railNames, { timeout: 10_000 }).toEqual(swapped)

      // 跨刷新保持（POST /api/projects/reorder 落盘）。
      await page.reload()
      await expect(rail.host).toBeVisible()
      await expect.poll(railNames, { timeout: 10_000 }).toEqual(swapped)

      // 边界位：首行「上移」禁用、末行「下移」禁用（属性或反射属性任一为真）。
      const isDisabled = (loc: ReturnType<ProjectRail['projectRow']>) =>
        loc.evaluate((el) => {
          const item = el as unknown as { disabled?: boolean }
          return el.hasAttribute('disabled') || item.disabled === true
        })
      const first = await openProjectMenu(swapped[0]!)
      await expect
        .poll(() => isDisabled(first.locator('wa-dropdown-item[data-testid="project-move-up"]')), {
          timeout: 5_000,
        })
        .toBe(true)
      await page.keyboard.press('Escape')
      const last = await openProjectMenu(swapped[swapped.length - 1]!)
      await expect
        .poll(() => isDisabled(last.locator('wa-dropdown-item[data-testid="project-move-down"]')), {
          timeout: 5_000,
        })
        .toBe(true)
      await page.keyboard.press('Escape')

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('D10 会话选择写 URL', () => {
    test('选择写 /sessions/{key} 不重载；SPA 段内后退地址回落且实例不重挂；刷新自持；未知键降级', async ({
      page,
      request,
    }) => {
      test.setTimeout(90_000)
      await resetState(request)
      const { name: projectName } = await ensureSceneProject(request)
      const tagA = `url-a-${Date.now()}`
      const tagB = `url-b-${Date.now()}`
      const keyA = await createSession(request, { prompt: tagA })
      const keyB = await createSession(request, { prompt: tagB })
      await waitStatus(request, keyA, ['done'])
      await waitStatus(request, keyB, ['done'])

      const rail = new ProjectRail(page)
      const detail = new FocusedSession(page)
      await page.goto('/')
      await rail.ensureProjectExpanded(projectName)

      // rail 选择 → 地址写深链（pushState），页面不重载（探针随后在同一元素
      // 上存活验证），会话就地渲染。
      await rail
        .host.locator('li.session-item:not(.archived)', { hasText: tagA })
        .first()
        .click()
      await expect(page).toHaveURL(new RegExp(`/sessions/${escapeRegExp(keyA)}$`))
      await expect(detail.userTurn(tagA)).toBeVisible({ timeout: 15_000 })
      await page.evaluate(() => {
        const dash = document.querySelector('sebas-app')?.shadowRoot?.querySelector(
          'sebas-dashboard',
        )
        if (dash) (dash as unknown as { __qaSameInstance?: boolean }).__qaSameInstance = true
      })

      // 同文档 SPA 段：再点 B → 地址写 B 的深链，dashboard 实例不被拆毁
      // 重建（三态同一模板字面量——探针存活即证据）。
      await rail
        .host.locator('li.session-item:not(.archived)', { hasText: tagB })
        .first()
        .click()
      await expect(page).toHaveURL(new RegExp(`/sessions/${escapeRegExp(keyB)}$`))
      try {
        await expect(detail.userTurn(tagB)).toBeVisible({ timeout: 15_000 })
      } catch (err) {
        // 诊断快照：URL 已写 B，但工作台到底呈现了什么？
        const diag = await page.evaluate(() => {
          const app = document.querySelector('sebas-app')
          const dash = app?.shadowRoot?.querySelector('sebas-dashboard')
          const tv = dash?.shadowRoot?.querySelector('sebas-transcript-view')
          const stream = dash?.shadowRoot?.querySelector('.turn-stream-area')
          return {
            dashFound: !!dash,
            deepLinkKey:
              (dash as unknown as { deepLinkKey?: string | null } | null)?.deepLinkKey ?? null,
            transcriptText: (tv?.shadowRoot?.textContent ?? stream?.textContent ?? '').slice(0, 400),
          }
        })
        console.log(`[qa-round3 D10] B-turn missing; url=${page.url()} diag=${JSON.stringify(diag)}`)
        throw err
      }
      expect(
        await page.evaluate(
          () =>
            (
              document.querySelector('sebas-app')?.shadowRoot?.querySelector(
                'sebas-dashboard',
              ) as unknown as { __qaSameInstance?: boolean } | null
            )?.__qaSameInstance === true,
        ),
      ).toBe(true)

      // 浏览器后退（同文档 popstate，段内无 reload）：地址回 A 的深链——
      // 后退语义优先，不被工作台抢写；实例仍在、无白屏、无 pageerror。
      await page.goBack()
      await expect(page).toHaveURL(new RegExp(`/sessions/${escapeRegExp(keyA)}$`))
      await expect(page.locator('sebas-dashboard')).toBeVisible()
      expect(
        await page.evaluate(
          () =>
            (
              document.querySelector('sebas-app')?.shadowRoot?.querySelector(
                'sebas-dashboard',
              ) as unknown as { __qaSameInstance?: boolean } | null
            )?.__qaSameInstance === true,
        ),
      ).toBe(true)
      await page.goBack()
      await expect(page).toHaveURL(/\/$/)
      await expect(page.locator('sebas-dashboard')).toBeVisible()
      expect(
        await page.evaluate(
          () =>
            (
              document.querySelector('sebas-app')?.shadowRoot?.querySelector(
                'sebas-dashboard',
              ) as unknown as { __qaSameInstance?: boolean } | null
            )?.__qaSameInstance === true,
        ),
      ).toBe(true)
      expect(collector.pageErrors).toEqual([])

      // 带深链刷新 → 焦点自持（冷恢复同会话）。
      await page.goto(`/sessions/${keyA}`)
      await expect(detail.userTurn(tagA)).toBeVisible({ timeout: 15_000 })

      // 未知键深链：如实降级（「会话不可得」），不白屏。
      await page.goto('/sessions/ghost%00session')
      await expect(
        page.locator('sebas-dashboard .empty-stream', { hasText: '会话不可得' }),
      ).toBeVisible({ timeout: 15_000 })
      await expect(page.locator('sebas-dashboard')).toBeVisible()

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('D5 探测错误完整可读（真实渲染）', () => {
    test('provider 抓取失败的长错误独占整行换行铺开，不被表单右缘裁切', async ({
      page,
      request,
    }) => {
      const settings = new SettingsModal(page)

      await resetState(request)
      // 自定义 provider（哑 base url——探测经浏览器路由拦截应答，不外呼）。
      // 载荷形状照抄 models.spec 的可过关形态（缺 models 会被 store 拒）；
      // 名字带时间戳——playwright 重试复用同一沙箱，固定名会撞 409。
      const providerName = `probe-fail-${Date.now()}`
      const created = await request.post('/api/providers', {
        data: {
          name: providerName,
          base_url_anthropic: 'http://127.0.0.1:1/v1',
          models: [{ id: 'probe-m', tags: [] }],
        },
      })
      expect(created.ok(), `provider create: ${await created.text()}`).toBe(true)

      // 长错误体（webui 反代形态的 error 字段）——模拟不可达上游的冗长 cause。
      const longCause =
        'probe failed: dial tcp 127.0.0.1:1: connect: connection refused while negotiating ' +
        'anthropic /v1/models handshake (retry budget exhausted after 3 attempts, ' +
        'last error: EOF before response headers, upstream alert certificate expired, ' +
        'see provider console for details and re-issue the credential with correct scopes)'
      await page.route(
        new RegExp(`/api/providers/${providerName}/probe$`),
        (route) =>
          void route.fulfill({
            status: 502,
            contentType: 'application/json',
            body: JSON.stringify({ error: longCause }),
          }),
      )

      await page.goto('/')
      await settings.openViaSidebar()
      await settings.openSection('模型')
      const row = settings.panel.locator('.provider-row', { hasText: providerName })
      await expect(row).toBeVisible({ timeout: 10_000 })
      await row.locator('button[title="编辑"]').click()
      const editor = page.locator('sebas-settings-modal wa-dialog.provider-editor')
      await expect(editor.locator('button[data-testid="fetch-models"]')).toBeVisible({
        timeout: 10_000,
      })
      await editor.locator('button[data-testid="fetch-models"]').click()

      // 错误完整呈现（含尾段——单行裁切下 tail 不可见）。
      const err = editor.locator('.fetch-error')
      await expect(err).toBeVisible({ timeout: 10_000 })
      await expect(err).toContainText(longCause.slice(-60))

      // 浏览器层换行合同：任意点可断 + 非单行（换行铺开）+ 盒宽不出容器。
      const geo = await err.evaluate((el) => {
        const cs = getComputedStyle(el)
        const box = el.getBoundingClientRect()
        const head = el.parentElement!.getBoundingClientRect()
        return {
          overflowWrap: cs.overflowWrap,
          whiteSpace: cs.whiteSpace,
          height: box.height,
          right: box.right,
          headRight: head.right,
        }
      })
      expect(geo.overflowWrap).toBe('anywhere')
      expect(geo.whiteSpace).toBe('normal')
      // 单行高度 ≈ 0.72rem × 1.5 ≈ 17px；长文换行必然 ≥2 行。
      expect(geo.height).toBeGreaterThan(25)
      // 不被表单右缘裁掉（D5 缺陷形态：单行挤同一行、右缘截断）。
      expect(geo.right).toBeLessThanOrEqual(geo.headRight + 1)

      // 收尾：先关 provider 编辑器（开着会盖住设置面板的关闭钮）。
      await editor.locator('wa-button').filter({ hasText: '取消' }).click()
      await expect(editor).toBeHidden({ timeout: 10_000 })
      await settings.close()
      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('D1 thinking 折叠缺省形态可区分（真实渲染）', () => {
    test('收起行带胶囊底色与 data-kind=thinking；二级条目挂 glyph；正文零 PROCESS 词', async ({
      page,
      request,
    }) => {
      test.setTimeout(90_000)
      const detail = new FocusedSession(page)
      await resetState(request)

      // claude-thinking 桩：一回合交替 thinking/正文（同 thinking-process-fold
      // journey 的数据源），这里钉 round3 新增的呈现面而非旧合同。
      const key = await createSession(request, { prompt: null, agent: 'claude-thinking' })
      await page.goto(`/sessions/${key}`)
      await expect(detail.composerTextarea).toBeVisible({ timeout: 15_000 })
      await detail.sendFollowUp('think please')
      await detail.expectStatus('done', 30_000)

      const assistantTurn = page
        .locator('sebas-dashboard sebas-transcript-view .turn-block.is-assistant')
        .first()
      await expect(assistantTurn).toBeVisible()

      // 两个 thinking 过程折叠，收起行 data-kind=thinking（run 成员构成词）。
      const folds = assistantTurn.locator('div.process-fold')
      await expect(folds).toHaveCount(2, { timeout: 15_000 })
      for (let i = 0; i < 2; i += 1) {
        await expect(folds.nth(i)).toHaveAttribute('data-kind', 'thinking')
      }

      // 缺省（收起）形态与正文可区分：折叠行的计算样式带**非透明**胶囊底色
      // 与圆角（surface-2 胶囊真实上屏，不只是样式表文本存在）。
      for (let i = 0; i < 2; i += 1) {
        const linkStyle = await folds
          .nth(i)
          .locator('button.fold-link')
          .evaluate((el) => {
            const cs = getComputedStyle(el)
            return { background: cs.backgroundColor, radius: parseFloat(cs.borderRadius) }
          })
        expect(linkStyle.background).not.toBe('rgba(0, 0, 0, 0)')
        expect(linkStyle.background).not.toBe('transparent')
        expect(linkStyle.radius).toBeGreaterThan(0)
      }

      // 正文段零过程词：「PROCESS」只出现在折叠行（.fold-link .label）。
      const bodies = assistantTurn.locator('.body:not(.fold-body):not(.item-body)')
      await expect(bodies).toHaveCount(2)
      for (const text of await bodies.allInnerTexts()) {
        expect(text.toLowerCase()).not.toContain('process')
      }

      // 二级 thinking 条目收起行挂 thinking glyph（与工具条目视觉分开）。
      // 结算重分组会把折叠收回收起态——展开与断言同轮询。
      const thinkingItems = assistantTurn.locator(
        'div.process-item[data-element-type="thinking"]',
      )
      await expect
        .poll(
          async () => {
            await detail.expandAllFolds()
            return thinkingItems.count()
          },
          { timeout: 20_000 },
        )
        .toBe(2)
      await expect(thinkingItems.first().locator('button.fold-link .item-kind-icon')).toBeVisible()

      expect(collector.clean()).toEqual([])
    })
  })
})
