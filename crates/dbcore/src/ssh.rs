//! SSH tunnels: password, key (+ passphrase), agent, one jump host, auto-reconnect.

use crate::error::Result;
use crate::model::{Secrets, SshConfig};

/// A local TCP listener on 127.0.0.1 forwarding each accepted connection over SSH
/// (direct-tcpip) to `target_host:target_port`. If the SSH session drops, the next
/// accepted connection reconnects transparently.
pub struct Tunnel {
    pub local_port: u16,
}

impl Tunnel {
    pub async fn open(
        ssh: &SshConfig,
        secrets: &Secrets,
        target_host: &str,
        target_port: u16,
    ) -> Result<Tunnel> {
        todo!(
            "A3: tunnel via {}@{} to {target_host}:{target_port} ({})",
            ssh.user,
            ssh.host,
            secrets.ssh_password.is_some()
        )
    }

    /// Stops listening and closes the SSH session.
    pub async fn close(self) {
        todo!("A3: close tunnel on {}", self.local_port)
    }
}
