/**
 * Journey — 单进程形态（core --webui）下 core 进程死亡：SPA 诚实降级。
 *
 * detached 拓扑的停机窗口（fatal 横幅 + 锁定遮罩 + composer 门禁）由
 * deployment.spec / tiered-notices.spec 覆盖——那里 webui 还活着，可达性
 * 有 core 的翻转推送兜底。这里是被同一套测试放过的形态：webui 服务本身
 * 就是死掉的那个进程（bare core --webui 崩溃/被杀），页面只剩已加载的
 * SPA——断连只能靠 WS 关闭自感，提交只能靠 fetch 失败自感。
 *
 * 断言（ GUI 验收 2026-09-22 实测行为收口）：
 * - ws-down 横幅出现（琥珀色重连提示，非 detached 的 fatal 锁定）；
 * - rail 诚实报「无法连接服务器」+ 重试入口，不装作列表新鲜；
 * - 提交面 5s 内 NetworkError 内显（composer-error，role=alert），草稿
 *   保留不丢字，转录无伪成功条目。
 *
 * 拓扑前提：tasks.py `TESTSUITE_ALLOW_CORE_DEATH=1`——core 进程被用例
 * SIGKILL 是被测前提，harness 不得因子进程死亡抢跑清场。本 spec 独占
 * playwright.dead-core.config.ts（端口 9895），主套件绝不可混入：core
 * 一死，同 config 的后续用例全部失联。
 */
import { expect, test } from '@playwright/test'
import {
  createSession,
  ErrorCollector,
  killCoreHard,
  resetState,
  waitStatus,
  Workbench,
} from './helpers/index'

test.describe('单进程 core 死亡（SPA 诚实降级）', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test('core 死亡：ws-down 横幅 + rail 不可达 + 提交 NetworkError 内显且草稿保留', async ({
    page,
  }) => {
    const workbench = new Workbench(page)

    await resetState(page.request)
    const key = await createSession(page.request, { prompt: 'warmup' })
    await waitStatus(page.request, key, ['done'])
    await page.goto(`/sessions/${key}`)
    await expect(workbench.composerTextarea).toBeVisible()

    // 被测动作：服务进程当场死亡（SIGKILL，无优雅退出）。
    await killCoreHard()

    // 客户端自感断连：WS 关闭 → 琥珀重连横幅；rail 报无法连接 + 重试。
    const banner = page.locator('[data-testid="ws-down-banner"]')
    await expect(banner).toBeVisible({ timeout: 15_000 })
    await expect(banner).toContainText('与服务器的连接已断开')
    await expect(page.locator('sebas-project-rail')).toContainText('无法连接服务器', {
      timeout: 15_000,
    })

    // 死亡窗口内的提交：5s 预算内 NetworkError 内显（不吞反馈、不伪成功），
    // 草稿保留（失败不丢字），转录不追加任何条目。
    await workbench.composerTextarea.fill('while core dead')
    await workbench.composerTextarea.press('Enter')
    const composerError = page.locator('sebas-workbench-composer [data-testid="composer-error"]')
    await expect(composerError).toBeVisible({ timeout: 5_000 })
    await expect(composerError).toContainText('NetworkError')
    await expect(workbench.composerTextarea).toHaveValue('while core dead')
    await expect(
      page.locator('sebas-transcript-view .turn-block', { hasText: 'while core dead' }),
    ).toHaveCount(0)

    expect(collector.clean()).toEqual([])
  })
})
