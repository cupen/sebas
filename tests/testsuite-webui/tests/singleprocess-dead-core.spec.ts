/**
 * Journey — 单进程形态（core --webui）下 core 进程死亡：SPA 诚实降级。
 *
 * detached 拓扑的停机窗口由 deployment.spec / tiered-notices.spec 覆盖；
 * 这里 webui 服务本身死掉，页面只剩已加载的 React SPA——断连靠 WS 关闭
 * 自感，提交靠 fetch 失败自感。
 *
 * 断言（GUI 验收 2026-09-22 实测行为收口）：
 * - ws-down 横幅出现（琥珀色重连提示，非 detached 的 fatal 锁定）；
 * - rail 诚实报「无法连接服务器」+ 重试入口，不装作列表新鲜；
 * - 提交面 5s 内 NetworkError 内显（composer 内 role=alert），草稿保留，
 *   转录无伪成功条目。
 */
import { expect, test } from '@playwright/test'
import {
  createSession,
  ErrorCollector,
  killCoreHard,
  resetState,
  waitStatus,
} from './helpers/index'
import { ReactWorkbench } from './helpers/pages/react'

test.describe('单进程 core 死亡（SPA 诚实降级）', () => {
  // killCoreHard 的 SIGKILL 等待语义读 /proc（helpers/coreproc.ts
  // requirePosix）——linux-first 套件。
  test.skip(process.platform === 'win32', 'SIGKILL wait semantics need POSIX (linux-first suite)')
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test('core 死亡：ws-down 横幅 + rail 不可达 + 提交 NetworkError 内显且草稿保留', async ({
    page,
  }) => {
    const workbench = new ReactWorkbench(page)

    await resetState(page.request)
    const key = await createSession(page.request, { prompt: 'warmup' })
    await waitStatus(page.request, key, ['done'])
    await page.goto(`/sessions/${key}`)
    await expect(workbench.composerTextarea).toBeVisible()

    // 被测动作：服务进程当场死亡（SIGKILL，无优雅退出）。
    await killCoreHard()

    // 客户端自感断连：WS 关闭 → 琥珀重连横幅；rail 报无法连接 + 重试。
    const banner = page.getByText(/服务器连接已断开/)
    await expect(banner).toBeVisible({ timeout: 15_000 })
    await expect(page.getByRole('complementary', { name: '项目导航' })).toContainText(
      '无法连接服务器',
      { timeout: 15_000 },
    )

    // 死亡窗口内的提交：5s 预算内 NetworkError 内显（不吞反馈、不伪成功），
    // 草稿保留（失败不丢字），转录不追加任何条目。
    await workbench.composerTextarea.fill('while core dead')
    await workbench.composerTextarea.press('Enter')
    const composerError = workbench.composer.getByRole('alert')
    await expect(composerError).toBeVisible({ timeout: 5_000 })
    await expect(composerError).toContainText('NetworkError')
    await expect(workbench.composerTextarea).toHaveValue('while core dead')
    await expect(workbench.unitWith('while core dead')).toHaveCount(0)

    expect(collector.clean()).toEqual([])
  })
})
