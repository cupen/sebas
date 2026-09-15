## Context

现状与动机见 proposal.md。事实底座：release.yml（taiki-e create-gh-release-action + upload-rust-binary-action，双平台归档 + sha256 已配好）、docker.yml（tag→版本号镜像 tag，main→latest，`latest=false` 显式关掉 tag 推 latest）、Dockerfile（Node22+corepack 多阶段构建，只产出主控）、build.rs（先找 `pnpm` 再找 `corepack pnpm`，都失败则静默嵌占位页并仅发 cargo:warning）。已查证：taiki-e/create-gh-release-action@v1 无 prerelease 输入、无自动 notes 输入。仓库 0 个 tag，整条链路零实战。

## Goals / Non-Goals

**Goals:**

- 发布前拦截三类坏产物：版本不一致 tag、占位页二进制、缺 sebas-node 的镜像
- Release 具备预发布语义与自动变更摘要
- 推 v0.1.0 完成首次真实发布并逐项验收

**Non-Goals:**

- 不扩平台、不引入 CHANGELOG/版本工具、不改 latest 语义、不加测试门禁（proposal Non-goals）
- 不动 ci.yml 与 main→latest 的 docker 路径行为

## Decisions

1. **tag 校验与预发布判定抽成 composite action**（`.github/actions/verify-release-tag/`）：bash 读 Cargo.toml 首个 `^version` 与 `GITHUB_REF_NAME` 去 `v` 前缀逐字比对，不一致 `exit 1`；输出 `is_prerelease`（tag 含 `-` 即预发布——`v[0-9]+.*` 模式下连字符只出现在 semver 预发布段）。release.yml 与 docker.yml 共用，消除双份漂移。备选：两边各复制一段 inline step（简单但会漂移）；cargo-metadata+jq（windows runner 无 jq 保证，弃）。
2. **Release 创建从 taiki-e create-gh-release-action 换成 `gh release create --generate-notes`**：已查证 taiki-e 该 action 不支持 prerelease 与自动 notes，续用要靠「先建再 edit」两段式；gh 预装于双平台 runner，`--generate-notes` 与 `--prerelease`（按 `is_prerelease` 条件传）一步到位，正文由 GitHub 生成。upload-rust-binary-action 继续负责资产上传（支持上传到已存在的 release，两者本就是标准组合）。
3. **占位页双保险**：upload 步骤前插 `setup-node@v4`（node 22）+ `pnpm/action-setup@v4`（version 11.24.0，与 ci.yml 对齐；显式安装不经 corepack，规避 runner 差异），build.rs 的 discover_pnpm 命中 PATH 即真实构建；action 完成后对 `target/<target>/release/sebas[.exe]` 跑 `grep -q "Frontend bundle not built"` 占位页特征文本，命中即 fail（产物级证据，不依赖构建过程的旁路观察）。
4. **Dockerfile 增产 sebas-node**：builder 阶段追加 `cargo build --release --locked -p sebas-node --bin sebas-node`（跨包 bin 必须带 `-p`，与 ci.yml 一致），runtime 阶段 COPY 进 `/usr/local/bin/`；ENTRYPOINT/CMD 不变，节点机 `docker run <image> sebas-node …` 覆盖 command。
5. **docker.yml 仅在 tag 路径接校验**：composite action 步骤挂 `if: startsWith(github.ref, 'refs/tags/')`，main→latest 路径零改动。
6. **本地验证用 actionlint（能装则装）+ YAML 解析**；workflow 真值只能靠真实 tag 运行验证。

## Risks / Trade-offs

- [upload-rust-binary-action 上传到已存在 release 的兼容性从未实战] → 首发即验证；失败则该 action 兜底建 release 的行为兜住（社区标准组合）
- [首发中途失败，tag 已消耗] → 预案：删 Release + 删 tag 重推（无人消费前安全），tasks 里显式列步骤
- [ghcr 首推自动建 package，默认可见性可能私有] → 非阻塞；交付报告里提醒到 package settings 调整
- [预发布路径首发无法真实验证] → composite action 判定逻辑本地 bash 单测；真实 rc 路径留待首个 rc tag，报告里如实声明
- [grep 扫大二进制自检耗时] → `grep -q` 短路 + 特征文本仅占位页携带，秒级

## Migration Plan

无数据迁移。workflow/Dockerfile 改动合入 main 即生效；随后按 tasks 执行 v0.1.0 首发验收；失败按预案删 Release+tag 重推，回滚即 `git revert` workflow 提交。

## Open Questions

无——notes 具体格式、ghcr 可见性为非阻塞细节，实现期顺手处理。
