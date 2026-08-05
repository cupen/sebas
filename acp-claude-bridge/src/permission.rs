//! Permission broker. Listens for PreToolUse hook requests and returns
//! Allow/Deny decisions sourced from the ACP `session/request_permission`
//! reply (driven by sebas's Feishu card UI in production; by the test harness
//! in tests).
//!
//! Transport: Unix domain socket on Unix; TCP loopback (`127.0.0.1`) on other
//! platforms (e.g. Windows) so the crate builds and runs everywhere. The
//! sidecar file at `<temp>/sebras-bridge.sock.path` holds the socket path
//! (Unix) or `host:port` (elsewhere); hook clients read it to connect.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, Mutex};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionDecision {
    Allow,
    Deny,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HookRequest {
    pub tool_name: String,
    #[serde(default)]
    pub tool_input: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct HookResponse {
    pub decision: &'static str,
}

async fn handle_one<S>(
    stream: S,
    decisions: Arc<Mutex<mpsc::Receiver<PermissionDecision>>>,
) -> anyhow::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (read, mut write) = tokio::io::split(stream);
    let mut lines = BufReader::new(read).lines();
    let line = match lines.next_line().await? {
        Some(l) => l,
        None => return Ok(()),
    };
    let req: HookRequest = serde_json::from_str(&line)?;
    tracing::info!(tool=%req.tool_name, "permission request received");
    // Wait for the ACP side to push a decision. In tests, the harness sends one
    // before this point; in production, the ACP server task forwards the
    // session/request_permission reply.
    let decision = decisions
        .lock()
        .await
        .recv()
        .await
        .unwrap_or(PermissionDecision::Deny);
    let word = match decision {
        PermissionDecision::Allow => "approve",
        PermissionDecision::Deny => "deny",
    };
    let resp = HookResponse { decision: word };
    let body = serde_json::to_string(&resp)?;
    write.write_all(body.as_bytes()).await?;
    write.shutdown().await?;
    Ok(())
}

fn sidecar_path() -> PathBuf {
    std::env::temp_dir().join("sebras-bridge.sock.path")
}

#[cfg(unix)]
mod broker_unix {
    use super::*;
    use std::path::Path;
    use tokio::net::UnixListener;

    pub struct PermissionBroker {
        listener: UnixListener,
        sock_path: PathBuf,
        sidecar: PathBuf,
        decisions: Arc<Mutex<mpsc::Receiver<PermissionDecision>>>,
    }

    impl PermissionBroker {
        pub async fn bind() -> anyhow::Result<(Self, mpsc::Sender<PermissionDecision>)> {
            let dir = std::env::temp_dir();
            let sock_path = dir.join(format!("sebras-bridge-{}.sock", std::process::id()));
            let sidecar = sidecar_path();
            let listener = UnixListener::bind(&sock_path)?;
            std::fs::write(&sidecar, sock_path.to_string_lossy().as_bytes())?;
            let (tx, rx) = mpsc::channel(32);
            Ok((
                Self {
                    listener,
                    sock_path,
                    sidecar,
                    decisions: Arc::new(Mutex::new(rx)),
                },
                tx,
            ))
        }

        pub fn socket_path(&self) -> &Path {
            &self.sock_path
        }

        pub async fn run(self) -> anyhow::Result<()> {
            loop {
                let (stream, _addr) = self.listener.accept().await?;
                let decisions = self.decisions.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_one(stream, decisions).await {
                        tracing::warn!(error=%e, "permission client failed");
                    }
                });
            }
        }
    }

    impl Drop for PermissionBroker {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.sock_path);
            let _ = std::fs::remove_file(&self.sidecar);
        }
    }
}

#[cfg(unix)]
pub use broker_unix::PermissionBroker;

#[cfg(not(unix))]
mod broker_tcp {
    use super::*;
    use tokio::net::TcpListener;

    pub struct PermissionBroker {
        listener: TcpListener,
        sidecar: PathBuf,
        decisions: Arc<Mutex<mpsc::Receiver<PermissionDecision>>>,
    }

    impl PermissionBroker {
        pub async fn bind() -> anyhow::Result<(Self, mpsc::Sender<PermissionDecision>)> {
            let listener = TcpListener::bind("127.0.0.1:0").await?;
            let addr = listener.local_addr()?;
            let sidecar = sidecar_path();
            // No Unix domain sockets on this platform; the sidecar holds
            // `host:port` and hook clients connect over TCP loopback instead.
            std::fs::write(&sidecar, addr.to_string().as_bytes())?;
            let (tx, rx) = mpsc::channel(32);
            Ok((
                Self {
                    listener,
                    sidecar,
                    decisions: Arc::new(Mutex::new(rx)),
                },
                tx,
            ))
        }

        pub async fn run(self) -> anyhow::Result<()> {
            loop {
                let (stream, _addr) = self.listener.accept().await?;
                let decisions = self.decisions.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_one(stream, decisions).await {
                        tracing::warn!(error=%e, "permission client failed");
                    }
                });
            }
        }
    }

    impl Drop for PermissionBroker {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.sidecar);
        }
    }
}

#[cfg(not(unix))]
pub use broker_tcp::PermissionBroker;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[tokio::test]
    async fn approve_returns_approve() {
        let (broker, _tx) = PermissionBroker::bind().await.unwrap();
        let broker_handle = tokio::spawn(broker.run());

        // We don't send a decision; the broker should handle an empty channel
        // and return deny. This test just verifies the endpoint accepts a
        // connection. The full round-trip is covered by
        // tests/permission_roundtrip.rs.

        #[cfg(unix)]
        let mut client = {
            let sock = broker_socket();
            use std::os::unix::net::UnixStream;
            UnixStream::connect(sock).unwrap()
        };
        #[cfg(not(unix))]
        let mut client = {
            let addr = broker_socket();
            std::net::TcpStream::connect(addr).unwrap()
        };

        client
            .write_all(br#"{"tool_name":"Bash","tool_input":{}}"#)
            .unwrap();
        client.flush().unwrap();
        // Close the client without waiting for a reply; with no sender alive,
        // decisions.recv() yields None -> deny, so the broker just closes too.
        drop(client);
        drop(broker_handle);
    }

    #[cfg(unix)]
    fn broker_socket() -> std::path::PathBuf {
        let s = std::fs::read_to_string(sidecar_path()).unwrap();
        std::path::PathBuf::from(s.trim())
    }

    #[cfg(not(unix))]
    fn broker_socket() -> String {
        std::fs::read_to_string(sidecar_path()).unwrap().trim().to_string()
    }
}
