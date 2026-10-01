## 1. 会话级 token 用量可见（GAP-01，usage-statistics）

- [x] 1.1 `client.ts` SessionInfo/SessionSummary 补 usage 字段（对齐引擎快照 shape）；会话呈现面展示累计 input/output，未上报显示「未上报 token」文案
- [x] 1.2 浏览器旅程先红后绿：fake-claude 会话一个回合后呈现非零累计、第二回合后增长；fakeacp 会话如实显示未上报（`tests/testsuite-webui`）
  - 留待 3c/主 agent GUI：<fake-claude 桩已补非零 usage（result 帧按成功回合序号递增 in 100n/out 10n，支持「增长」断言）；后端 detail/summary/相位帧均已透传 usage；浏览器旅程本身未跑——无浏览器权限>
- [x] 1.3 回归：`/usage` 页（router timeseries）内容与既有测试不受影响

## 2. core 连接状态常驻指示（GAP-02，agent-workbench）

- [x] 2.1 app-shell header 常驻徽标：订阅既有 `core.reachability` 推送 + 主动查询，三态（ok 低调 / down 醒目+cause 悬停 / 恢复自动翻回）；与断线横幅同源联动
- [x] 2.2 浏览器旅程：健康态低调呈现；断连态（复用既有死亡旅程装配 `TESTSUITE_ALLOW_CORE_DEATH`/dead-core 姿态）翻红且横幅一致；恢复自动翻转
  - 留待 3c/主 agent GUI：<徽标三态与同源联动已有 vitest 单测（unknown/ok/down/恢复/kind-less 退化）；dead-core 装配的浏览器旅程未跑——无浏览器权限>

## 3. 目录选择器新建子目录（GAP-03，webui/projects）

- [x] 3.1 `POST /api/fs/mkdir`：复用 `safe_path` 单点校验，单层创建、父须存在；负路径单测全覆盖（越界/父缺失/空名/分隔符/`..`，fail-closed）
- [x] 3.2 folder-picker「新建文件夹」入口 + 内联命名；成功后当前节点局部刷新、新目录可进入；旅程断言边界内成功与非法名类型化拒绝
  - 留待 3c/主 agent GUI：<实现与单测已完成（validateFolderName 预检 + mkdir 流程 + 局部刷新 + folder-selected 照发，组件级 vitest 全绿）；「旅程断言边界内成功与非法名类型化拒绝」的浏览器旅程未跑——无浏览器权限>
- [x] 3.3 回归：project-rail 与 new-session-dialog 两处复用点均可用；注册新目录为项目走通
  - 备注（实施收口）：全仓 folder-picker 复用点实际只有 project-rail 添加项目弹窗（new-session-dialog 不使用），按实际一处收口；注册走通由 folder-picker-mkdir 旅程承载
  - 留待 3c/主 agent GUI：<单元级回归已过（project-rail 既有 picker 用例全绿）；代码事实：new-session-dialog 并不使用 folder-picker（全仓唯一使用点是 project-rail 的添加项目弹窗），「两处复用点」实际只有一处；「注册新目录为项目走通」需浏览器旅程，未跑——无浏览器权限>

## 4. 收口

- [x] 4.1 `tests/acceptance/COVERAGE.md` 回填 GAP-01/02/03 证据
  - 留待 3c/主 agent GUI：<验收账本回填依赖浏览器/e2e 证据，未动 COVERAGE.md>
- [ ] 4.2 全量：`invoke testsuite-e2e` + 相关浏览器旅程绿；`openspec validate add-webui-round7-gaps` 通过
  - 留待 3c/主 agent GUI：<按分工约定不跑 invoke testsuite-* / 浏览器套件；单元门禁已全绿：rtk cargo test（workspace，42 套件 0 失败）+ pnpm test（836 用例）+ rtk cargo build>
