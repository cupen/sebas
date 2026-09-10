## 1. Spec fixes applied directly (details in proposal)

- [x] 1.1 router-auth-rate-limit: loopback-gated open router + access-log `-` + admin/metrics surface note
- [x] 1.2 webui: POST model endpoint + route list + admin-auth reality + stale scenario
- [x] 1.3 cli-service: --log-file removed, im/status/services aliases, webui-secret degrade
- [x] 1.4 watchdog: ServiceSet{core} CLI/WebUI allowed, webui default-on, 5s spawn-retry, stderr inherit, Ready-only pipe, ctl events
- [x] 1.5 dispatch-commands: no-session explicit, 暂未支持 replies, dual /settings surfaces, help-card scope
- [x] 1.6 replay-debug: sebas im --dump-inbound, neutral-shape fidelity, no-gates, no-dedup
- [x] 1.7 agent-driver: AcpEvent + ModelChanged
- [x] 1.8 agent-bench: bucket names aligned

## 2. Code fix bundled

- [x] 2.1 `sebas agent-bench` subcommand wired (cli.rs + main.rs; smoke verified 2/2 PASS)
- [x] 2.2 watchdog up-to-date update no longer restarts core (UpdateOutcome + EXIT_UP_TO_DATE=3)

## 3. Validation

- [x] 3.1 `openspec validate --specs` → 0 invalid
- [x] 3.2 `cargo test --workspace` → 1289 passed, 0 failed
