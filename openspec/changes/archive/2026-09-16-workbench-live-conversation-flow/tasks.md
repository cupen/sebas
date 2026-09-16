# Tasks — workbench-live-conversation-flow

## 1. Core 通道回合事件传输

- [x] 1.1 core：订阅会话执行体的 ACP 回合事件（文本/思考/工具），经核心通道以新帧类型下发；core 侧按约 250ms 窗口合并文本/思考增量（大小上限立即冲刷），工具事件逐条直发；定位四元组 `(session, turn position, entry position, seq)` 齐全。验证：core 侧单测覆盖合并窗口、冲刷触发、seq 单调、落库 `TurnEntry` 不受合并影响
- [x] 1.2 webui `SessionBackend`：进程内后端与 core_channel 客户端两条实现都暴露回合事件订阅面。验证：两条后端的单测各自转发同一帧契约（既有 backend 测试基建内断言）

## 2. WS 转播与前端增量渲染

- [x] 2.1 `events.rs` 新增 `turn.delta` / `turn.tool` 事件变体（dotted type 序列化）。验证：events.rs 既有「dotted type tag」测试模式新增两例
- [x] 2.2 前端 `ws.ts` 事件词汇表 + `transcript-view.ts` 增量追加：按四元组定位追加文本/思考进块尾、工具卡片即现即收；seq 去重；快照 refetch 永远赢。验证：vitest 覆盖追加、乱序/重复丢弃、快照收敛三态

## 3. 聚焦即拉起（带 resume）

- [x] 3.1 webui 后端：聚焦无活子进程的会话时触发既有 spawn 路径（无 prompt），resume 沿 acp-session-mapping；同一会话幂等；失败非致命（占位保留、错误如实上报）。验证：cargo 单测覆盖触发、幂等、失败保占位三态
- [x] 3.2 模型芯片三态：启动中（spawn 窗口）→ agent 上报模型 → spawn 完成为空的诚实「无可用模型」。验证：workbench-composer vitest 三态断言（复用 model-chip-unavailable 测试基建）

## 4. 底沿重组与会话头去交互化

- [x] 4.1 `workbench-composer.ts`：mode 下拉移入底沿左端（远端会话 effective/desired 呈现不变）；提交按钮去文字、图标即状态（aria-label/title 保留五态语义）；agent 锁标签迁出。验证：composer vitest 调整断言（data-state 五态、mode 提交走 `POST /mode`）
- [x] 4.2 `dashboard.ts` 会话头删除 All sessions/Archive/Close/mode 控件（保留纯展示：agent 锁、last active 等）；`project-rail.ts` 会话行溢出菜单新增归档项，确认弹窗合并「将丢弃 N 条待执行」警告。验证：dashboard / project-rail vitest 断言头部零按钮与菜单归档流

## 5. 版面对齐与 rail 宽度

- [x] 5.1 分隔条 12→6px；对话区与输入框共用同一套水平内边距 token（边缘齐平）；竖向空隙收紧。验证：布局 vitest/快照 + 沙箱截图目检（`invoke testsuite-webui-sandbox`）
- [x] 5.2 `split-persist.ts` rail 默认 280px（≥1440px 视口）、上限 520px；行标题 12 个中文字无截断。验证：vitest 断言默认值/clamp/12 字标题不截断

## 6. 流式与已读锚联动

- [x] 6.1 `transcript-view.ts`：流式追加在「聚焦 + 贴底」时经既有防抖路径推进读锚；未贴底/未聚焦不写锚。验证：vitest 覆盖贴底推进（角标不闪）、上滚计未读两态（配合 session-unread-badge 测试）

## 7. 套件与质量门

- [x] 7.1 e2e：fake-claude/fake-acp-agent 期刊注入增量事件，断言 WS 帧到浏览器、对话区流式上屏；聚焦占位会话自动 resume 场景。验证：`invoke testsuite-e2e --case <new>` 通过
- [x] 7.2 全量质量门：`rtk cargo test`、`rtk cargo clippy`、前端 vitest 全绿；沙箱端到端冒烟（注册项目 → 新建会话 → 聚焦拉起 → 流式输出 → 归档经 rail 菜单）。验证：退出码 0 + 冒烟截图记录
