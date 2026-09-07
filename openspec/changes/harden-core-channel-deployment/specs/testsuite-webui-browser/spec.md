# Delta — testsuite-webui-browser

## ADDED Requirements

### Requirement: detached 部署态旅程

套件 SHALL 提供 detached 双进程沙箱变体（core + 独立 webui，auth 关闭），供部署态旅程在浏览器级验证——现有浏览器沙箱为 `core --webui` 单进程形态，通道客户端不在场，无法触达"核心不可达"路径。

#### Scenario: 一键运行 detached 旅程用例

- **WHEN** `invoke testsuite-webui --case deployment` 以 detached 变体运行
- **THEN** chromium 指向独立 webui 端口，旅程按下列场景执行并在结束自清理

### Requirement: 核心不可达横幅与降级提示旅程

在 detached 变体下，套件 SHALL 验证全局横幅与降级提示的诚实呈现。

#### Scenario: core 停止后横幅出现

- **WHEN** 双进程装配达可达后终止 core，页面处于任意视图
- **THEN** 全局"核心不可达"横幅出现且包含上报 cause

#### Scenario: core 停止期间加项目出现降级提示

- **WHEN** core 停止期间经项目选择器注册一个合法目录
- **THEN** 项目落栏且伴随"核心不可达，已写入本地注册表"降级提示；composer 提交门禁同时呈现不可达态

#### Scenario: core 恢复后横幅与降级态消失

- **WHEN** core 重新启动并完成通道握手
- **THEN** 无需刷新页面，横幅消失，composer 恢复可提交
