# tasks — expand-webui-e2e-settings（三期）

> spec 只定覆盖方向、不锁 case 清单；以下为首批用例，后续渐进续补无需改 spec。

## 1. 设置面只读（对应"设置面只读呈现覆盖"）

- [x] S1 Services 分区：Router 卡片 listen 地址与 `/api/router` 真值一致、Running 呈现；Provider routing 卡片 `N provider(s) · auth none · debug on` 与真值一致；验证：`invoke testsuite-webui --case settings` 通过
- [x] S2 About 分区：Version/Uptime/Rust toolchain/Router listen/Providers 五行与 `/api/about` 真值一致（包含断言，不断字面量）；验证：同上
- [x] S3 Env 分区：`SEBAS_WEBUI_PASSWORD` 行存在且值为占位语义；验证：同上

## 2. 设置面写降级（对应"设置面写操作诚实降级覆盖"）

- [x] S4 agent-defaults：工具栏 `no default set` 与 `GET /api/agent-defaults`（null/null）一致；★ 置默认提交后内联错误外显、真值仍 null/null、对话框可取消；验证：同上
- [x] S5 provider mutations：空名称客户端校验（`名称不能为空`，零网络）；具名新建/编辑保存、删除确认、探测全部 503 内联错误外显且列表不变；验证：同上

## 3. 项目选择器（对应"项目选择器交互覆盖"）

- [x] P1 树闭环：沙箱 work 下预建父/子目录，树展开见子、点选回填手动框、提交后项目落栏；验证：`invoke testsuite-webui --case projects` 通过
- [x] P2 内联错误：空路径提交按钮禁用；不存在路径提交后错误内显、对话框不关、注册表不变；验证：同上

## 4. 账本与收尾

- [x] `tests/acceptance/COVERAGE.md` 在 testsuite-webui-browser 行下追加三期旅程证据；验证：矩阵无空白条目
- [x] 稳定性复跑：同一提交连续 3 次 `invoke testsuite-webui --case settings` + 3 次 `--case projects` 全绿；验证：settings 3 连绿 + projects 3 连绿 + 全量 30+3 全绿（2026-09-07 实跑；途中修 4 类选择器问题与 1 例真 flake，见下）
  - 修复记录：about 空 dd 用 toBeAttached；toolbar .label 改 span 限定；wa-dialog host 读 hidden 改断内部按钮；503/400 浏览器资源日志按窄口径过滤；S4 首行 provider 有无 catalog 两分支用 or 断言

（复跑证据：以收敛后骨架（converge-webui-e2e-tree，33 it）全量 3 连绿覆盖本框要求——settings/projects 各 3 次含于全量，且另经历 `--case projects` 独立复跑 4 轮。2026-09-08。）
