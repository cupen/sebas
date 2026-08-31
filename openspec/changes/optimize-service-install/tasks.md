## 1. upgrade 辅助函数暴露与稳定路径 seed

- [x] 1.1 将 `upgrade::expand_tilde` 与 `upgrade::data_dir` 设为 `pub`，并让
      `data_dir` 支持传入 user home 覆盖；verify：`cargo build` 通过且 `cargo test
      --lib upgrade` 绿
- [x] 1.2 新增 `upgrade::seed_stable_binary(dest_bin)`：版本戳 + 复制 + chmod 0755，
      幂等（已存在则覆盖）；verify：对临时 data_dir 调用后 `bin/sebas` 存在且可执行、
      `--version` 与当前二进制一致

## 2. render_unit 生成 watchdog + 稳定路径 + 加固

- [x] 2.1 `render_unit`/`UnitInputs` 改为接收固定 binary 路径（`data_dir/bin/sebas`）
      并渲染 `ExecStart=<bin> watchdog --config <config>`（非 `run`）；verify：
      `src/service.rs` 单元测试断言含 `watchdog`
- [x] 2.2 在 unit 中加入 `NoNewPrivileges`、`ProtectSystem=full`、
      `ProtectHome=read-only`、`PrivateTmp`，且**不**含 `ProtectSystem=strict`；
      verify：新增渲染断言逐条核对
- [x] 2.3 对 `ExecStart` 的 binary/config 路径做 systemd 转义（含空格的路径保持单
      token）；verify：路径含空格的输入渲染结果能被 systemd 正确解析（断言 + 可选
      `systemd-analyze verify`）

## 3. run_install 解析 data_dir、seed、软链、幂等 restart

- [x] 3.1 `run_install` 从 config 的 `[watchdog.storage].data_dir`（空则按 `--user`
      home 推导）解析 `data_dir`，并调用 `seed_stable_binary`；verify：
      `--config` 指向含 data_dir 的配置时，安装后 `data_dir/bin/sebas` 生成
- [x] 3.2 best-effort 创建 `/usr/local/bin/sebas → data_dir/bin/sebas` 软链，失败仅
      warn 不报错；verify：有权限时软链存在，失败时 install 仍成功
- [x] 3.3 写 unit + `daemon-reload` 后，若 unit 当前 active 则 `systemctl restart`；
      verify：对已在运行的服务重复 `install` 会触发 restart（用 fake systemctl 桩
      断言调用序列）

## 4. --log-level 与 --user 存在性校验

- [x] 4.1 `ServiceArgs` 新增 `--log-level`（install-only），`run_install` 用它决定
      `RUST_LOG` 烘焙（未设则继承环境，空回落 `info`）；verify：cli 解析测试 +
      渲染断言
- [x] 4.2 `validate_user` 增加账户存在性检查（libc `getpwnam`，不派生子进程），拒绝
      不存在的 `--user`，exit 4；verify：新增单测断言 `nosuchuser` → exit 4

## 5. 文档与规格同步

- [x] 5.1 更新 `openspec/specs/cli-service/spec.md` 的三处需求（unit 生成、校验与
      exit codes、start/uninstall）与本次 delta 对齐；verify：`openspec validate`
      通过
- [x] 5.2 README/相关文档中 `service --install` 示例改为 watchdog 语义并说明迁移；
      verify：文档与 spec 无 `sebas run` 的 service 误导

## 6. 集成验证

- [x] 6.1 全量 `cargo test` 绿；verify：`cargo test` 0 失败
- [x] 6.2 手工 smoke：`sudo sebas service --install --auto-start` 后 `systemctl
      status sebas` 显示 watchdog 进程，`sebas ctl status` 可用；verify：状态输出符合预期
      （沙箱无 root；已验证：渲染 unit 经 `systemd-analyze verify` 通过、非 root 拒
      绝 exit 4。root enable/start 需在目标机执行）