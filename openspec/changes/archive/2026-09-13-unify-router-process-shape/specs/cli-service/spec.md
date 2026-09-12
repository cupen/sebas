## ADDED Requirements

### Requirement: Router runs standalone-only

The router SHALL run only as a standalone process: `sebas router --config
<path> [--debug]`. The `run`/`core` entrypoints SHALL NOT embed a router
server — the `--router` and `--debug` flags on `run`/`core` SHALL be removed
(BREAKING): passing them SHALL fail with an unknown-argument error rather
than starting any in-process router. Deployments that want the router SHALL
run `sebas router` as its own process — manually, as a compose sidecar, or
via the watchdog's managed router child (which spawns the same standalone
entrypoint). The debug `test` provider remains available via
`sebas router --debug`.

#### Scenario: embedded router flag is rejected

- **WHEN** `sebas core --config <path> --router` is invoked
- **THEN** it fails with an unknown-argument error and no router starts
  inside the core process

#### Scenario: standalone router with debug provider

- **WHEN** `sebas router --config <path> --debug` is invoked
- **THEN** the standalone router starts and the debug `test` model answers
  without dialing any upstream

#### Scenario: manual and managed forms share one entrypoint

- **WHEN** the watchdog spawns its managed router child and, separately, an
  operator runs `sebas router` manually
- **THEN** both are the same standalone `sebas router` entrypoint with the
  same HTTP surface
