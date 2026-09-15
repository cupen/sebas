## Purpose

规约 sebas 的 tag 触发式正式发布流水线：推送版本 tag 即产出可信的 GitHub Release（双平台归档 + sha256 + 自动变更摘要）与 ghcr.io 镜像（双二进制），并以 tag↔版本一致性校验、前端真实嵌入自检、预发布语义保证「发布出去的产物就是 tag 所指的那个版本、带真 UI、可校验完整性」。

## ADDED Requirements

### Requirement: 推送版本 tag 触发完整发布

推送匹配 `v[0-9]+.*` 的 tag SHALL 触发发布流水线，产出 GitHub Release 与 ghcr.io 镜像；main 与开发分支的普通 push SHALL NOT 产出 Release。

#### Scenario: 推送 v0.1.0 触发完整发布

- **WHEN** 推送 tag `v0.1.0` 且 tag 与 Cargo.toml 版本一致
- **THEN** GitHub Release `v0.1.0` 发布且资产齐全，ghcr.io 出现 `v0.1.0` 镜像 tag

#### Scenario: 普通分支推送不发布

- **WHEN** 推送 main 或开发分支的普通 commit
- **THEN** 不创建 GitHub Release，不产出版本号镜像 tag

### Requirement: tag 与 manifest 版本一致性校验

发布流水线 SHALL 在构建与发布任何资产之前，校验 tag 名去掉 `v` 前缀后与 Cargo.toml 的包版本逐字一致；不一致 SHALL 立即失败且不产出任何 Release 或镜像。

#### Scenario: 不一致的 tag 快速失败

- **WHEN** 推送 `v9.9.9` 而 Cargo.toml 版本为 `0.1.0`
- **THEN** 流水线在校验步骤失败，仓库无 `v9.9.9` Release、ghcr 无 `v9.9.9` 镜像

#### Scenario: 一致的 tag 通过校验

- **WHEN** 推送 `v0.1.0` 且 Cargo.toml 版本为 `0.1.0`
- **THEN** 校验通过，后续构建与发布继续

### Requirement: 发布归档含双二进制与 sha256

tag 发布 SHALL 为每个目标平台（linux x86_64、windows x86_64）产出一个归档（tar.gz / zip），归档内 SHALL 同时含 `sebas` 与 `sebas-node` 两个二进制，且每个归档 SHALL 附带 sha256 校验文件。

#### Scenario: 归档内容完整

- **WHEN** 下载任一平台的发布归档并解包
- **THEN** 解包结果同时含 `sebas` 与 `sebas-node` 可执行文件

#### Scenario: sha256 校验通过

- **WHEN** 对下载的归档按附带的 sha256 文件执行校验
- **THEN** 校验一致，归档未被篡改或截断

### Requirement: 发布二进制嵌入真实 WebUI

发布产物中的 `sebas` 二进制 SHALL 嵌入真实构建的前端 bundle 而非占位页。发布流水线 SHALL 显式准备前端工具链完成嵌入构建，并 SHALL 对构建产物做占位页特征自检——检出占位页特征即失败，不发布残次产物。

#### Scenario: 前端真实嵌入

- **WHEN** 发布构建在无本地缓存的干净 runner 上执行
- **THEN** 前端经显式工具链构建并嵌入，产物自检通过

#### Scenario: 自检拦截占位页产物

- **WHEN** 构建产物内检出占位页特征文本
- **THEN** 发布流水线失败，该产物不进入 Release

### Requirement: 预发布 tag 语义

带 semver 预发布后缀（`-` 后跟 rc/alpha/beta 等标识，如 `v0.2.0-rc.1`）的 tag SHALL 产出被 GitHub 标记为 pre-release 的 Release，镜像仍打该 tag 名作为镜像 tag；无后缀的 tag SHALL 产出正式 Release。

#### Scenario: rc tag 产出预发布

- **WHEN** 推送 `v0.2.0-rc.1`（Cargo.toml 版本同步为 `0.2.0-rc.1`）
- **THEN** GitHub Release `v0.2.0-rc.1` 被标记为 pre-release，ghcr 存在 `v0.2.0-rc.1` 镜像 tag

#### Scenario: 正式 tag 产出正式 Release

- **WHEN** 推送无预发布后缀的 tag
- **THEN** 对应 GitHub Release 不带 pre-release 标记

### Requirement: docker 镜像含双二进制

tag 发布产出的 ghcr.io 镜像 SHALL 同时含 `sebas` 与 `sebas-node`；镜像默认入口 SHALL 仍为主控 `sebas`，`sebas-node` 经覆盖 command 调用。

#### Scenario: 镜像内双二进制可执行

- **WHEN** 拉取 tag 对应的 ghcr 镜像并分别以 `sebas --version` 与 `sebas-node --version` 运行
- **THEN** 两者均输出版本号

#### Scenario: 默认入口为主控

- **WHEN** 不带 command 启动该镜像
- **THEN** 运行的是主控 `sebas core`

### Requirement: Release 正文自动生成

tag 触发的 GitHub Release 正文 SHALL 由平台自动生成（相对上一 tag 的变更摘要），SHALL NOT 依赖手工撰写或仓库内 CHANGELOG 文件。

#### Scenario: Release 正文非空

- **WHEN** 任一 tag 发布完成
- **THEN** Release 正文含自动生成的变更条目而非空内容
