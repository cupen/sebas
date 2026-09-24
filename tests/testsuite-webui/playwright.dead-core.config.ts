import os from 'node:os'
import path from 'node:path'
import { defineConfig, devices } from '@playwright/test'
import { REPO_ROOT } from './playwright.config'

// 单进程死亡形态（GUI 验收 2026-09-22 收口的易错点）：harness 以普通
// 两进程形态（core --webui + 独立 router）拉起沙箱，用例把 core 进程
// SIGKILL 后断言 SPA 的诚实降级。core 死亡在主套件是致命的（harness
// 因子进程死亡清场、同 config 其余用例全部失联），因此独占一份 config：
// TESTSUITE_ALLOW_CORE_DEATH=1 让 harness 只认停机信号，本 config 只有
// 这一个 spec，且不做重试——服务已死，重试只会得到第二个死法。

// The harness script publishes its throwaway sandbox dir here; the
// keep-on-fail reporter reads it to preserve the scene on failure.
process.env.TESTSUITE_SCENE_FILE ||= path.join(os.tmpdir(), 'sebas-testsuite-webui-scene-9895')

export const WEBUI_PORT = 9895

export default defineConfig({
  testDir: './tests',
  testMatch: /singleprocess-dead-core\.spec\.ts/,
  timeout: 60_000,
  retries: 0,
  workers: 1,
  // collect-json 只写机器可读结果（add-testsuite-report），keep-on-fail 仍是
  // 沙箱 keep/clean 的唯一权威，两者并列互不干扰。
  reporter: [['list'], ['./tests/reporters/keep-on-fail.ts'], ['./tests/reporters/collect-json.ts']],
  use: {
    baseURL: `http://127.0.0.1:${WEBUI_PORT}`,
    headless: true,
    trace: 'retain-on-failure',
  },
  webServer: {
    // env 经 Playwright 注入而非 shell 前缀——`TESTSUITE_…=… cmd` 的前缀
    // 形式在 Windows cmd 上不是合法命令。端口必须显式传递：harness 的
    // 端口是模式推导的（9899/9898/9896/9897），dead-core 形态没有专属
    // 推导，缺了就会绑去 9899 而 Playwright 在等 9895。
    command: 'invoke testsuite-webui-server',
    env: {
      ...process.env,
      TESTSUITE_ALLOW_CORE_DEATH: '1',
      TESTSUITE_PORT: String(WEBUI_PORT),
    },
    url: `http://127.0.0.1:${WEBUI_PORT}/health`,
    cwd: REPO_ROOT,
    timeout: 120_000,
    reuseExistingServer: process.env.TESTSUITE_REUSE === '1',
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
})
