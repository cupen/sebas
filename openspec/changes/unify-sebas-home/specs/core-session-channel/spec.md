## MODIFIED Requirements

### Requirement: Channel transport and authentication
The core SHALL expose the channel on a local IPC endpoint at a configurable
path defaulting to `<SEBAS_HOME>/run/core.sock` (the sebas home's run
directory): a Unix domain socket with owner-only permissions (0600) on Unix,
mapped deterministically to a named pipe (`\\.\pipe\sebas/<path>`) on
Windows. On Unix every connection SHALL be authenticated by both peer
credentials — the connecting uid MUST equal the core's own uid — and a
shared secret supplied out of band, in the same posture as the watchdog
control RPC; on Windows, where peer-uid credentials are not available, the
shared secret plus the default named-pipe ACL carry the authentication
(peer-uid checks are a Unix-only guarantee). A connection failing either
check SHALL be rejected and closed without processing any request. The
channel SHALL NOT be exposed over TCP.

#### Scenario: foreign uid rejected

- **WHEN** on Unix a process running as a different uid connects to the socket
- **THEN** the connection is rejected and closed, and no request on it is processed

#### Scenario: missing or wrong secret rejected

- **WHEN** a connection does not supply the agreed secret, or supplies a different one
- **THEN** the connection is closed without a response

#### Scenario: cross_uid_rejected_live_process

- **WHEN**（Unix）跨 uid 进程（fork 子进程后 `setuid` 到不同账户——不是同进程改 uid，而是真实跨进程凭证）尝试连接 core socket 并发请求
- **THEN** 连接被拒绝；服务端 SHALL 在日志写 peer-uid 不匹配记录（`warn!("core channel: peer uid mismatch; closing")`）；不进入 request 处理

#### Scenario: stale socket file is reclaimed

- **WHEN** the core starts and a socket file already exists at the path with no
  live listener behind it
- **THEN** the core removes the stale file and binds a fresh socket

#### Scenario: endpoint follows the sebas home

- **WHEN** `SEBAS_HOME` is pinned and the core starts with no explicit
  channel path configured
- **THEN** the channel socket is created at `<SEBAS_HOME>/run/core.sock`
- **AND** no path under `XDG_RUNTIME_DIR` is consulted
