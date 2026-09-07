# sebas webui browser-e2e（Playwright）

浏览器级旅程套件：真实 chromium 驱动 webui，被测后端是 `scripts/webui_e2e_server.sh`
装配的一次性沙箱（`sebas core --router --debug --webui` 单进程调试形态 +
`tests/bin` 的 fake-claude 桩）。绝不触碰真实 `~/.sebas` 与端口 9797。

## 一键运行

```bash
invoke webui-e2e                 # 构建（dist 自动重建）→ 全量旅程 → 清理
invoke webui-e2e --case auth     # 仅鉴权旅程（auth-on 形态，端口 9898）
invoke webui-e2e --case first-paint  # 仅单个旅程 spec
```

直接跑（仓库根）：

```bash
cargo build --bin sebas --bin fake-claude
pnpm install --dir tests/webui-e2e
pnpm --dir tests/webui-e2e exec playwright install chromium
pnpm --dir tests/webui-e2e exec playwright test
```

## 旅程与账本

| spec | 旅程 |
|---|---|
| `first-paint.spec.ts` | 首屏结构 + reachability 如实显示（acp ok / native 如实不可用）|
| `session-roundtrip.spec.ts` | composer 发起会话 → 桩回复按序渲染 → Done → 重载恢复 |
| `streaming.spec.ts` | "stream" 触发词：分批 chunk + 运行中瞬态（expect.poll，零固定 sleep）|
| `permission.spec.ts` | "perm" 触发：review card 的 deny / allow-once / allow-session 三条路径 |
| `errors.spec.ts` | "refuse" 非终态拒绝会话存活；"crash" 如实呈现进程死亡 |
| `projects.spec.ts` | folder-picker 添加沙箱内目录 → 项目栏 → 移除 |
| `session-mgmt.spec.ts` | close / archive / 深链 SPA fallback / `/settings` 重定向 / 模型面诚实缺省 |
| `auth.spec.ts` | 仅 auth-on 形态：错误凭据拒绝、admin/admin 登录、登出、深链重定向登录页 |

能力矩阵账本见 `tests/acceptance/COVERAGE.md` 的 `testsuite-webui-browser` 一节。

## 调试开关

| 开关 | 作用 |
|---|---|
| `E2E_KEEP=1 invoke webui-e2e` | 通过后也保留沙箱现场（排障）|
| `E2E_REUSE=1` | 复用上一次保留的沙箱（配合 E2E_KEEP）|
| `E2E_PORT=<port>` | 覆盖沙箱端口（默认 9899；`E2E_AUTH=1` 时 9898）|

任一用例失败时，keep-on-fail reporter 会把沙箱目录保留下来并在输出里打印路径
（含后端日志 `core.log`），供复现。

## 平台适配

Linux（含 headless CI/云端）为主；Windows（Git Bash/msys）尽力而为：
脚本内 `cygpath -m` 转换 config 路径、`.exe` 后缀探测、短沙箱目录名
（named pipe 256 字符上限）、`.gitattributes` 强制 `*.sh eol=lf`。
