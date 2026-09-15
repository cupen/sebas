## Why

仓库已有 tag 触发的发布 workflow（ed2e5a3），但从未真正跑过（0 个 tag），且存在实质落差：release 构建的前端嵌入靠 build.rs 隐式发现 pnpm，失败会静默把占位页嵌进发布产物；tag 与 Cargo.toml 版本号无一致性校验；预发布 tag 与正式版无区别；docker 镜像只带主控二进制。在第一次正式发布之前补齐这些差距，成本最低。

## What Changes

- release.yml / docker.yml 的 tag 触发路径加 tag↔Cargo.toml 版本一致性校验，不一致即失败
- release.yml 显式安装 Node + pnpm 构建前端，并加构建产物自检（二进制含占位页特征文本即 fail）
- 预发布识别：`vX.Y.Z-<后缀>` 形态的 tag → GitHub Release 标 pre-release（镜像 tag 照打）
- GitHub Release 正文自动生成 notes（上一 tag 以来的 commits）
- Dockerfile 增产 sebas-node，docker 镜像带双二进制（ENTRYPOINT 不变，节点机覆盖 command 使用）
- 推送 v0.1.0 做首次实战发布验收：Actions 全绿、Release 资产齐全（双平台归档 + sha256）、ghcr 镜像可拉取

## Capabilities

### New Capabilities

- `tag-release-pipeline`：tag 触发的正式发布流水线的外部可观察行为——tag↔版本一致性校验、双平台归档与 sha256、前端真实嵌入保障、预发布语义、ghcr 镜像发布（双二进制）

### Modified Capabilities

（无——deployment spec 是 Ansible 部署 role 的规约，不在本变更范围）

## Impact

- `.github/workflows/release.yml`、`.github/workflows/docker.yml`、`Dockerfile`
- 不改任何 Rust 源码；ci.yml 与 main→latest 镜像语义不动

## Non-goals

- 不扩发布平台：arm64 / macOS 依赖链从未验证，明确不做
- 不引入 CHANGELOG.md 与 cargo-release / release-please 类版本工具
- 不改 latest 语义：latest 永远跟 main，正式版只有版本号镜像 tag
- 发布 workflow 内不跑测试门禁，靠「tag 只从 CI 绿的 main 打」流程自律
- sebas-node 的容器化节点链路不做实证，只保证二进制进入镜像
