---
name: sebas-testsuite
description: 一键验收 sebas：构建 → 运行进程级 e2e 测试套件 → 汇报验收结论。当用户要求“验收”“测试验收”“跑 e2e”“改动做得对不对”“总结发现问题”时使用此技能，即使没明说 e2e。
---

# sebas 验收（sebas-testsuite）

一键跑完验收并产出结论汇报。核心原则：**快速执行已固化的套件，不重复造轮子**。

## 为什么用 e2e 而不是手搭沙盒

`invoke testsuite-e2e` 是固化的进程级 e2e 套件（构建 + `tests/testsuite_e2e_test.rs`），覆盖 core 启动、router、会话往返、优雅退出——它就是“手动沙盒菜谱”的自动化版本。验收时优先用它；只有用户明确要求 GUI 或套件没覆盖的点才手搭沙盒（此时读 AGENTS.md 的“Sandbox debug recipe”）。

- 旅程级更深覆盖：`invoke testsuite-acceptance`（实时真实 CLI，慢）。除非用户点名，默认跑 e2e。
- 单用例：`invoke testsuite-e2e --case <name>`。

## 前置检查（隔离性）

沙盒规则红线：**绝不重启、抢占、读凭据用户的真实 sebas 实例**（AppImage，端口 9797，真实 `~/.sebas`）。e2e 套件自检隔离，但如果 9797 正在运行，提醒用户持续运行不受影响、不主动关闭。

跑前快速查残留：`/tmp/sbtestsuite.*` 是 webui 沙盒的目录前缀，进程异常退出（SIGKILL/强杀）会残留。**现在入口已带兜底清理**（`tasks.py` 的 `_cleanup_stale_sandboxes` 在 `testsuite-e2e` / `acceptance` / `webui` 启动前 + finally 里各跑一次），正常情况残留会自动清掉；但如果清理后仍有残留，报告需要人工介入而非强杀。

## 执行步骤

1. 构建 + 跑套件（一条命令，长任务设 timeout 600000；入口已带沙盒残留兜底清理）：

   ```bash
   invoke testsuite-e2e
   ```

   构建失败先看 `cargo build` 报错；套件失败读失败用例名和断言输出。

2. 想覆盖 webui 联调再加一条：

   ```bash
   invoke testsuite-webui-sandbox   # 仅当用户要求打开浏览器/GUI 时；否则跳过
   ```

3. 不要为“更快”自作主张改代码、跳过构建、注释掉失败的用例。失败如实报。

## 汇报格式

按用户要求组织：**先说验收了哪些功能，再说发现的问题（重要在前，次要在后）**。

```text
## 验收结论：<通过 | 有阻塞问题 | 部分通过>

### 验收了哪些功能
- <每条一行：功能 → 怎么验证的 → 结果>

### 发现的问题
**重要**（阻塞/影响正确性）：
1. <问题 → 证据 → 建议>

**次要**（体验/警告类）：
1. <同上>
```

问题分级口径：**重要** = 套件测试失败、功能不可用、数据/状态错、启动失败；**次要** = 警告日志、残留进程、非阻塞限制（如沙盒里 native backend 缺 `SEBAS_AGENT_PROVIDER_API_KEY`——这是预期限制，如实标注不算失败）。

收尾：沙盒目录/进程按规则清理后，在汇报末尾说一句残留状态。
