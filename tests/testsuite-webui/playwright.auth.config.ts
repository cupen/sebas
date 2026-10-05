import os from 'node:os'
import path from 'node:path'
import { defineConfig, devices } from '@playwright/test'

// Local REPO_ROOT on purpose: importing it from ./playwright.config would
// execute that module's TESTSUITE_SCENE_FILE default (port 9899) as a side
// effect and pin the wrong scene pointer for this assembly.
const REPO_ROOT = path.resolve(import.meta.dirname, '../..')

/**
 * Auth-on form of the suite (task 3.8): same harness, TESTSUITE_AUTH=1 → the
 * sandbox comes up on port 9898 with the unified test account admin/admin
 * provisioned in the sandbox-local auth.db (SEBAS_WEBUI_AUTH_DB). auth.spec.ts
 * and users-admin.spec.ts run here.
 */
process.env.TESTSUITE_AUTH = '1'
// Same default the harness script derives for port 9898; the keep-on-fail
// reporter reads the pointer from here.
process.env.TESTSUITE_SCENE_FILE ||= path.join(os.tmpdir(), 'sebas-testsuite-webui-scene-9898')

const WEBUI_PORT = 9898

export default defineConfig({
  testDir: './tests',
  // users-admin.spec.ts 与 auth.spec.ts 共用本装配（都需要 auth-on + 预置
  // admin）——users-admin 是 2026-09-23 验收补的 Users 管理闭环旅程；
  // agents-gate 是 gate-agent-directory-writes 补的 agents 写执法旅程
  // （viewer/member 经 users API 现场自愈建户）；qa-round7-ws-auth 是
  // fix-webui-qa-round7 3.2 的未认证 WS 静默 + 登录后实时链路旅程；
  // provider-gate 是 fix-webui-qa-round10 3.x 的 provider/别名变更面
  // 角色执法旅程（member/viewer 只读 + 服务端 403 + admin 全管）；
  // qa-round14-rbac 是 fix-webui-qa-round14 的 viewer 只读口径旅程
  // （纯 GET 只读视图不发 switch + /sessions 角色可见性 + 服务端半边）。
  testMatch:
    /auth\.spec\.ts|users-admin\.spec\.ts|agents-gate\.spec\.ts|qa-round7-ws-auth\.spec\.ts|provider-gate\.spec\.ts|qa-round14-rbac\.spec\.ts/,
  timeout: 30_000,
  retries: 1,
  workers: 1,
  // collect-json 只写机器可读结果（add-testsuite-report），keep-on-fail 仍是
  // 沙箱 keep/clean 的唯一权威，两者并列互不干扰。
  reporter: [['list'], ['./tests/reporters/keep-on-fail.ts'], ['./tests/reporters/collect-json.ts']],
  use: {
    baseURL: `http://127.0.0.1:${WEBUI_PORT}`,
    headless: true,
    trace: 'on-first-retry',
  },
  webServer: {
    command: 'invoke testsuite-webui-server',
    url: `http://127.0.0.1:${WEBUI_PORT}/health`,
    cwd: REPO_ROOT,
    timeout: 120_000,
    reuseExistingServer: process.env.TESTSUITE_REUSE === '1',
  },
  projects: [{ name: 'chromium-auth', use: { ...devices['Desktop Chrome'] } }],
})
