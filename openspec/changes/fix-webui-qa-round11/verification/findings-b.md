# QA-B 会话核心链路 黑盒 GUI 验收报告（sebas WebUI）

- 测试人：QA-B（Playwright + 本机 Chromium，黑盒 GUI：locator 真实交互、截图+只读 aria DOM
  交叉验证、全程只读收集 console）
- 被测：http://127.0.0.1:9877/（admin/admin，沙箱；操作员真实实例未触碰）
- 存档说明：本文件由主 agent 从会话记录恢复入库存档（原 Temp 副本被外部因素清除，
  截图/console 原始文件不可再生，结论完整）。

## 一、缺陷清单

### B-1（P1 功能坏）native 会话生命周期状态恒为「Queued」，永不推进到 Done
- 任建 native 会话 → 发消息 → 回合完成（转录完整、summary 已出）→ 状态徽章恒「Queued」，
  顶部「N 活跃 0 休眠」失真；仅 ACP 会话显示 Done。

### B-2（P2 体验）native 流式回合无「停止/取消」入口
- test/long 流式期间（实测 177→1248 字符）全程无停止按钮（3 轮采样 stop:false）；
  ACP 会话在等待审批时该按钮存在。

### B-3（P2 体验）后台完成的回合无任何通知提醒
- 切到历史页等待回合完成：无 toast/通知（通知层存在——创建会话有 toast——但回合终点未接入）。

### B-4（P3 打磨）多行消息在气泡中渲染为单行
- 三行输入（textarea 含 2 个 \n）发送后气泡单行空格连接；wire 往返仍含 \n（渲染层问题）。

### B-5（P3 打磨）深链已关闭会话时前端持续轮询，console 404 刷屏
- 页面呈现清晰（居中「会话不可得」+ 红条 + 输入禁用），但单次驻留 5-6 条 404。

### B-6（P3 打磨）未登录冷启动即连受保护 WS，认证失败噪音
- 与 A-3 同根，合并处理。

### 观察项（不计缺陷）
- O-1 native 会话标题不自动生成（与 B-1 同源：native 元数据不回填）。
- O-2 bash 工具 Windows 报 `io error: unix only`（平台限制；审批→执行→错误内联→summary 链路正常）。
- O-3 tools-parallel 审批卡「队列串行 + 末尾并排」，批准/拒绝互不干扰，机制可用。
- O-4 usage 只计 router 透传流量（native test/* 入图，ACP 不计；架构口径）。
- O-5 native 会话 key 前缀显示「飞书 · agent-xxx」，与「Native Kernel」标签不一致。

## 二、通过清单（要点）
登录+工作台布局；新建会话对话框（Agent 下拉/权限四档+说明/provider 缺省提示）；ACP bare 回合+CJK
往返；权限模式 ask 批准全链（卡→批准→✓已执行→tool_result→收尾）与拒绝路径（✗已拒绝+会话可复用）；
edit/allow/auto 档语义（edit 档 Bash 仍出卡；allow/auto 无卡直通；auto 无收尾正文=桩设计预期）；
模式切换即时生效+系统回执；test/text 回显（含 CJK）；test/thinking 可折叠过程块+块序；test/full 混排；
test/tool-use 批准路径；tools-parallel 互不干扰；test/long 流式增量与滚动跟随；test/empty 温和提示；
test/error 清晰呈现且可恢复；模型切换回执+下回合生效；会话列表/深链 4/4 自洽；会话关闭确认；
刷新持久化（转录恢复+URL 保持）；会话 token 累计；/usage 与流量对账（19 请求、8 模型分列）；
空输入禁用；5000 字符不崩；多行输入端保留（渲染丢失 = B-4）；暗色对比度/长词折行。

## 三、受阻未测
1. test/tool-use 拒绝路径（第二轮场景模型不声明工具）；审批拒绝呈现由 ACP 路径覆盖（组件共享）。
2. 未读游标/分隔线完整验证（fake 秒回无法制造窗口）。
3. native bash 工具实际执行效果（Windows 平台限制；write 工具写入成功旁证）。
4. T9 首轮误操作声明：在已关闭会话页执行的一轮无效，已在有效会话重测通过。

## 四、console 汇总
43 行、0 pageerror：32×WS 冷启动认证失败（B-6）+ 11×已关闭会话 404 轮询（B-5）；
test/error 的 5xx 仅页面内呈现，无 console 污染。
