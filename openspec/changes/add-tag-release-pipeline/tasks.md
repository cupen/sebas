## 1. verify-release-tag composite action

- [ ] 1.1 新建 `.github/actions/verify-release-tag/action.yml`：bash 步骤取 Cargo.toml 首个 `^version` 与 `GITHUB_REF_NAME` 去 `v` 前缀逐字比对，不一致输出两值并 `exit 1`；定义输出 `is_prerelease`（tag 名含 `-` 即 true）。验证：把校验逻辑提为本地 bash 脚本，以 `GITHUB_REF_NAME=v0.1.0`（通过）、`v9.9.9`（失败）、`v0.2.0-rc.1`（通过且 is_prerelease=true）三例断言

## 2. release.yml 改造

- [ ] 2.1 create-release job：移除 taiki-e/create-gh-release-action，改为 `gh release create "$GITHUB_REF_NAME" --generate-notes --verify-tag`，`is_prerelease=true` 时追加 `--prerelease`；job 首步挂 verify-release-tag。验证：对照 design 决策 2 评审 + 组 4 静态门禁
- [ ] 2.2 upload-assets job 两个平台在构建前插入 `actions/setup-node@v4`（node 22）与 `pnpm/action-setup@v4`（version 11.24.0）。验证：对照 design 决策 3 评审 + 组 4 静态门禁
- [ ] 2.3 upload-rust-binary-action 之后加产物自检步：对 `target/<target>/release/sebas`（windows 加 `.exe`）跑 `grep -q "Frontend bundle not built"`，命中 `exit 1`。验证：本地用带占位页特征文本的假二进制与真二进制各跑一次 grep 断言（真二进制可用 `cargo build --bin sebas` 产物）
- [ ] 2.4 确认 job 依赖链：upload-assets `needs: create-release` 保持，校验失败即全链不发布。验证：对照 spec「不一致的 tag 快速失败」场景推演触发路径

## 3. Dockerfile 与 docker.yml

- [ ] 3.1 Dockerfile builder 阶段追加 `RUN cargo build --release --locked -p sebas-node --bin sebas-node`，runtime 阶段 COPY sebas-node 至 `/usr/local/bin/`；ENTRYPOINT/CMD 不动。验证：对照 design 决策 4 评审；本地 docker 可用则 `docker build` 跑通，否则以首发实证兜底并在报告中注明
- [ ] 3.2 docker.yml 在登录 ghcr 前挂 verify-release-tag，步骤带 `if: startsWith(github.ref, 'refs/tags/')`；main→latest 路径行为零改动。验证：对照 design 决策 5 评审推演 tag/main 两条触发路径 + 组 4 静态门禁

## 4. 静态验证

- [ ] 4.1 三个 workflow 与 composite action 过 actionlint（可用 scoop/choco/go install 安装；确实装不上则以 `python -c "yaml.safe_load(...)"` 逐文件解析替代并在报告注明）。验证：actionlint 退出码 0（或替代手段通过）

## 5. v0.1.0 首发实战验收

- [ ] 5.1 改动经评审流程合入 main（feat 分支 rebase + --no-ff）。验证：`git log` 可见合并提交且 workflow 文件与任务 1–4 产物一致
- [ ] 5.2 向用户确认后推送 `v0.1.0`，盯 Release 与 Docker 两个 workflow 运行至全绿。验证：Actions 两 workflow 结论 success
- [ ] 5.3 验收 GitHub Release：非 draft、非 pre-release、正文含自动生成 notes；资产 = linux tar.gz + windows zip + 各自 `.sha256` 共 4 件；下载解包含 `sebas` 与 `sebas-node`，`sha256sum -c` 通过。验证：逐项对照 spec「发布归档含双二进制与 sha256」场景
- [ ] 5.4 验收镜像：`docker pull ghcr.io/cupen/sebas:v0.1.0`，`sebas --version` 与 `sebas-node --version` 均出版本号，不带 command 启动冒烟进 sebas core；本地无法跑容器则以 registry manifest API + 推送日志为证。验证：对照 spec「docker 镜像含双二进制」场景；顺带提醒用户检查 ghcr package 可见性
- [ ] 5.5 若首发失败：删除 Release 与远端 tag（`git push --delete origin v0.1.0`）修复后重推，仅限无人消费窗口；预案执行情况记入报告。验证：最终以全绿的 Actions 运行与 5.3/5.4 验收结果为准
- [ ] 5.6 交付报告：发布结论、ghcr 可见性提醒、预发布路径未经真实发布验证的声明（留待首个 rc tag）。验证：报告覆盖上述三点
