# Tasks: revamp-settings-nav-and-models-editor

## 1. 导航 IA 与视觉

- [ ] 1.1 重排 `SECTIONS` 为 `generic → appearance → services → models → about`，分区 id `settings` 更名 `generic`、`env` 并入 `generic`；缺省分区改 `generic`；`SECTION_DESC` 同步。验证：`settings-modal.test.ts` 导航顺序用例通过
- [ ] 1.2 导航视觉：行高约 36px、字号 0.875rem、栏宽 160px、整行 hover、当前项左侧 accent 竖条；`appearance|services` 之间与 `about` 上方各一条留白分隔线，About 以 `margin-top: auto` 压底。验证：浏览器沙箱目检（`invoke testsuite-webui-sandbox`）+ 既有渲染测试更新通过
- [ ] 1.3 `readLastSection` 回退验证：写入旧值 `settings`/`env` 后打开弹窗聚焦 `generic`。验证：新增单测用例通过

## 2. Settings 总览拆解与 About 重组

- [ ] 2.1 删除 `renderSettings`、Maintenance 区（重启全部 + 重置 Settings 按钮、确认 dialog、`resetSettings`/restartAll 状态与样式）。验证：grep 无残留；`pnpm vitest` 相关删除用例更新通过
- [ ] 2.2 About 分区改为 INSTANCE（workspace root + 复制、default agent kind、default provider/model + 跳转 Models）在上、BUILD（/api/about）在下；`loadOverview` 并入 About 加载路径。验证：About 分区渲染测试通过；沙箱目检两段顺序
- [ ] 2.3 Generic 分区承载原 Env 环境变量只读表（渲染函数搬迁）。验证：Generic 分区呈现 ENV_VARS 表的单测通过

## 3. Models 编辑器精简

- [ ] 3.1 「Model entries」标签改「Models」，删除两句提示语，「＋ Add model」改为纯 `＋` 通栏按钮。验证：编辑器渲染测试（testid `add-model-entry`）更新通过；沙箱目检

## 4. fetch 交互重做

- [ ] 4.1 删除 provider 行内 🔍 按钮与 `fetchResult` 结果列表 UI（`renderFetchResult`、`pickFetchedModel` 及相关 CSS）。验证：grep 无残留；旧挑选流测试移除
- [ ] 4.2 编辑器 Models 区块标题旁新增 fetch 按钮（provider 有可用 base URL 时渲染，含 preset code-table URL 判定）；成功按 D3 整单替换 `editor.models`（Map 合并保同 id tags、按 id 去重）；失败保留草稿并内联呈现脱敏原因。验证：新增单测覆盖替换/保 tags/去重/失败四场景
- [ ] 4.3 `fetchState` 收敛为 pending/error 进程序，fetch 期间按钮 disabled。验证：单测 + 沙箱手工：editor 内 fetch → 列表替换 → 取消编辑器后存储不变

## 5. 回归与验证

- [ ] 5.1 `pnpm vitest run` 全绿（含更新的 settings-modal 测试套件）
- [ ] 5.2 `invoke testsuite-webui-sandbox` 目检：导航顺序/分隔线/About 压底、Generic=Env 表、About 两段、编辑器 fetch 整单替换全流程
- [ ] 5.3 `openspec validate revamp-settings-nav-and-models-editor --strict` 通过
