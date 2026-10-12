# make-pi-the-builtin-agent — GUI 验收记录（task 6.2）

沙箱：`invoke testsuite-webui-sandbox`（端口 9879，auth 关，core --webui + 独立 router --debug）。
PATH 垫 fake npm + fake node（真实 npm 未触发，argv 日志为证）；**沙箱初始不含 pi**（验证「未装 → 安装 → 可用」全链）。
浏览器：headless Chromium（CDP :9333）经 browser-harness 驱动，1440×1000。

## 验收清单

| # | 检查项 | 结果 | 证据 |
|---|---|---|---|
| 1 | 未装 pi：`native` 行 display=`PI`、徽标「内置 · PI」、置灰且 cause=「pi 未安装——到「设置 → Agent」一键安装 pi」（无 env 标识符） | ✅ | `agents-before-install.png` + HTTP catalog |
| 2 | Settings → Agent 分区说明更新为「内置 PI agent 恒在不可删」；安装区出现 pi 配方行（未安装 + 安装按钮） | ✅ | `agents-before-install.png` |
| 3 | 点击安装 → fake npm 执行（argv：`install --global --prefix <SB>/agent-tools/pi @earendil-works/pi-coding-agent`），**真实 npm 全程未被触发** | ✅ | fake-npm-argv.log |
| 4 | **安装后 native(PI) 即时变可达（免重启）** | ✅ | `agents-after-install.png`（native 行「可达」）+ HTTP（reachable: True）——依赖本次动态探测源修复（见下） |
| 5 | 安装建出的 `pi` 行入目录（path 指私有 bin、可达） | ✅ | `agents-after-install.png`（pi 行）+ HTTP |
| 6 | 新建会话对话框：Agent 下拉含 **PI（上次使用）** 且可预选；空目录时如实提示「将使用 agent 内置的默认模型」 | ✅ | `new-session-dialog.png` |
| 7 | native(PI) 会话（web key）发消息 → pi 回合完成（transcript「pi says hello」）；回合身份显示 `pi` | ✅ | `session-view.png` + HTTP transcript |
| 8 | composer 切模型 → `set_model` 往返生效：转录渲染「模型已切换：anthropic/m-1 → anthropic/m-2」，composer 模型芯片显示 `anthropic/m-2` | ✅ | `session-view.png` + HTTP entries |

## 过程中发现并修复的实现缺口（3c 规则：小问题主 agent 直接修）

- **探测源装配期冻结**：3a#2 把 native 探测源在 run.rs 装配期一次性解析注入，导致「一键安装建出的 store 行（path 指私有 bin）不参与可达性解析」——装完 pi 内置槽位仍置灰，违反本 change spec「resolve from the agents-store row」与「availability recovers without reload」。
- **修复**：`native_probe` 从冻结 `AgentKindSource` 改为解析器闭包（`NativeProbeResolve`，每次 `execution_bodies` 调用重新走 config ∪ store ∪ 缺省解析）；run.rs 注入捕获 `Arc<Config>` 的闭包；测试注入点同步。

## 未覆盖（如实标注）

- env 注入（provider env → pi 子进程）的 GUI 形态未单独演示——沙箱默认 provider 未设置时按 D3「如实不注入」；该能力由进程级 e2e `native_slot_runs_pi_with_provider_env_and_model_switch` 断言（fake pi env 日志含注入的 `ANTHROPIC_API_KEY`/`ANTHROPIC_BASE_URL`）。
- 真实 npm / 真实 pi 二进制不在自动化验收面（design Non-goal）；操作员手跑一次真机安装即可补足。

## 结论

GUI 主链路（未装指引 → 一键安装 → 即时可选 → 建会话 → pi 回合 → 切模型）逐项通过。task 6.2 完成。
