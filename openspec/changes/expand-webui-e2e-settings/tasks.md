# tasks — expand-webui-e2e-settings（三期）

> spec 只定覆盖方向、不锁 case 清单；以下为首批用例，后续渐进续补无需改 spec。

## 1. 设置面只读（对应"设置面只读呈现覆盖"）

- [ ] S1 Services 分区：Router 卡片 listen 地址与 `/api/router` 真值一致、Running 呈现；Provider routing 卡片 `N provider(s) · auth none · debug on` 与真值一致；验证：`invoke testsuite-webui --case settings` 通过
- [ ] S2 About 分区：Version/Uptime/Rust toolchain/Router listen/Providers 五行与 `/api/about` 真值一致（包含断言，不断字面量）；验证：同上
- [ ] S3 Env 分区：`SEBAS_WEBUI_PASSWORD` 行存在且值为占位语义；验证：同上

## 2. 设置面写降级（对应"设置面写操作诚实降级覆盖"）

- [ ] S4 agent-defaults：工具栏 `no default set` 与 `GET /api/agent-defaults`（null/null）一致；★ 置默认提交后内联错误外显、真值仍 null/null、对话框可取消；验证：同上
- [ ] S5 provider mutations：空名称客户端校验（`名称不能为空`，零网络）；具名新建/编辑保存、删除确认、探测全部 503 内联错误外显且列表不变；验证：同上

## 3. 项目选择器（对应"项目选择器交互覆盖"）

- [ ] P1 树闭环：沙箱 work 下预建父/子目录，树展开见子、点选回填手动框、提交后项目落栏；验证：`invoke testsuite-webui --case projects` 通过
- [ ] P2 内联错误：空路径提交按钮禁用；不存在路径提交后错误内显、对话框不关、注册表不变；验证：同上

## 4. 账本与收尾

- [ ] `tests/acceptance/COVERAGE.md` 在 testsuite-webui-browser 行下追加三期旅程证据；验证：矩阵无空白条目
- [ ] 稳定性复跑：同一提交连续 3 次 `invoke testsuite-webui --case settings` + 3 次 `--case projects` 全绿；验证：6/6 通过（本机缺 libnspr4 时由宿主机代跑并记录）
