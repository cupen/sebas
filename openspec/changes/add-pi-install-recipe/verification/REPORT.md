# add-pi-install-recipe — GUI 验收记录（task 4.2）

沙箱与浏览器装配同 `make-pi-the-builtin-agent/verification/REPORT.md`（同一次 GUI 会话覆盖两 change；PATH 垫 fake npm + fake node，**真实 npm 未触发**）。

## 验收清单

| # | 检查项 | 结果 | 证据 |
|---|---|---|---|
| 1 | 可安装区出现 **pi 配方行**（`@earendil-works/pi-coding-agent`，未安装 + 安装按钮），与 claude/opencode 并列 | ✅ | `agents-before-install.png`（下方配方区） |
| 2 | 点击安装 → fake npm 执行：`install --global --prefix <SB>/agent-tools/pi @earendil-works/pi-coding-agent`；**node 前置通过**（fake node `v22.19.0`） | ✅ | fake-npm-argv.log（本次 GUI 会话） |
| 3 | 安装成功 → 目录出现 `pi` 行（driver=pi、path 指私有 bin、可达），安装区状态翻「已安装」 | ✅ | `agents-after-install.png`（pi 行 + 内置区）+ HTTP catalog |
| 4 | 真实 npm 全程未被触发 | ✅ | fake-npm-argv.log 为唯一 npm 调用记录 |
| 5 | 失败分支（node 缺失/版本低 → typed 4xx + 指引） | ✅（非 GUI 层） | 端点集成测试 4 例 + 进程级 e2e `pi_install_with_old_node_*`（3c 补） |

## 结论

pi 配方行的 GUI 链路（呈现 → 安装 → 目录行就绪）逐项通过。task 4.2 完成。
