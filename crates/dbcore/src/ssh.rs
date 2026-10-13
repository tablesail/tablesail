//! SSH tunnels: password, key (+ passphrase), agent, one jump host, auto-reconnect.
//!
//! A `Tunnel` binds `127.0.0.1:0`; every accepted connection opens a
//! `direct-tcpip` channel over one shared SSH session and is copied both ways.
//! The SSH session lives behind a mutex so that a dropped session is rebuilt
//! (same config, same secrets) by the next connection instead of failing it.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use russh::client::{self, Handle, Handler, Msg};
use russh::keys::{PrivateKeyWithHashAlg, PublicKeyOrCertificate, ssh_key};
use russh::{Channel, Disconnect, MethodKind};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::task::{JoinHandle, JoinSet};

use crate::error::{Error, Result};
use crate::model::{Secrets, SshAuth, SshConfig};

/// dbcore has no logging dependency; tunnel diagnostics go to stderr.
macro_rules! note {
    ($($arg:tt)*) => {
        eprintln!("[ssh] {}", format_args!($($arg)*))
    };
}

/// TCP connect + SSH handshake + authentication, per hop.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const KEEPALIVE: Duration = Duration::from_secs(15);

/// A local TCP listener on 127.0.0.1 forwarding each accepted connection over SSH
/// (direct-tcpip) to `target_host:target_port`. If the SSH session drops, the next
/// accepted connection reconnects transparently.
pub struct Tunnel {
    pub local_port: u16,
    link: Arc<Link>,
    accept: JoinHandle<()>,
}

impl Tunnel {
    pub async fn open(
        ssh: &SshConfig,
        secrets: &Secrets,
        target_host: &str,
        target_port: u16,
    ) -> Result<Tunnel> {
        if ssh.jump.as_ref().is_some_and(|j| j.jump.is_some()) {
            return Err(Error::Unsupported(
                "Only one SSH jump host is supported.".into(),
            ));
        }
        let link = Arc::new(Link {
            ssh: ssh.clone(),
            secrets: secrets.clone(),
            state: Mutex::new(None),
        });
        // Connect eagerly so authentication and host-key problems surface here,
        // not as a hang on the first query.
        link.ensure_connected().await?;

        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|e| Error::Ssh(format!("could not open a local port: {e}")))?;
        let local_port = listener
            .local_addr()
            .map_err(|e| Error::Ssh(format!("could not open a local port: {e}")))?
            .port();

        let accept = tokio::spawn(accept_loop(
            listener,
            link.clone(),
            target_host.to_owned(),
            target_port,
        ));
        Ok(Tunnel {
            local_port,
            link,
            accept,
        })
    }

    /// Stops listening and closes the SSH session.
    pub async fn close(self) {
        self.accept.abort();
        self.link.shutdown().await;
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        // Aborting the accept task also aborts every forwarded connection (its JoinSet drops).
        self.accept.abort();
    }
}

async fn accept_loop(listener: TcpListener, link: Arc<Link>, host: String, port: u16) {
    let mut conns = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (mut socket, peer) = match accepted {
                    Ok(v) => v,
                    Err(e) => {
                        note!("tunnel accept failed: {e}");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                };
                let _ = socket.set_nodelay(true);
                let link = link.clone();
                let host = host.clone();
                conns.spawn(async move {
                    match link.open_channel(&host, port, peer.port()).await {
                        Ok(channel) => {
                            let mut stream = channel.into_stream();
                            let _ = tokio::io::copy_bidirectional(&mut socket, &mut stream).await;
                        }
                        Err(e) => note!("tunnel to {host}:{port}: {e}"),
                    }
                });
            }
            Some(_) = conns.join_next(), if !conns.is_empty() => {}
        }
    }
}

// ---------------------------------------------------------------------------
// The shared SSH session
// ---------------------------------------------------------------------------

struct Link {
    ssh: SshConfig,
    secrets: Secrets,
    state: Mutex<Option<Connection>>,
}

/// The authenticated session to the target SSH host, plus the jump session
/// whose channel carries it (it must stay alive as long as `target` does).
struct Connection {
    target: Handle<Client>,
    jump: Option<Handle<Client>>,
}

impl Connection {
    fn is_closed(&self) -> bool {
        self.target.is_closed() || self.jump.as_ref().is_some_and(|j| j.is_closed())
    }
}

impl Link {
    async fn ensure_connected(&self) -> Result<()> {
        let mut state = self.state.lock().await;
        if state.as_ref().is_none_or(Connection::is_closed) {
            *state = None;
            *state = Some(connect_chain(&self.ssh, &self.secrets).await?);
        }
        Ok(())
    }

    /// Opens a forwarding channel, reconnecting once if the SSH session is gone.
    async fn open_channel(
        &self,
        host: &str,
        port: u16,
        originator_port: u16,
    ) -> Result<Channel<Msg>> {
        let mut state = self.state.lock().await;
        let mut last_err = None;
        for _ in 0..2 {
            if state.as_ref().is_none_or(Connection::is_closed) {
                *state = None;
                *state = Some(connect_chain(&self.ssh, &self.secrets).await?);
            }
            let Some(conn) = state.as_ref() else {
                break;
            };
            match conn
                .target
                .channel_open_direct_tcpip(host, port as u32, "127.0.0.1", originator_port as u32)
                .await
            {
                Ok(channel) => return Ok(channel),
                Err(e) if session_gone(&e) || conn.is_closed() => {
                    note!("SSH session lost ({e}); reconnecting");
                    *state = None;
                    last_err = Some(e);
                }
                Err(russh::Error::ChannelOpenFailure(reason)) => {
                    return Err(Error::Ssh(format!(
                        "the SSH server refused to connect to {host}:{port} ({reason:?})"
                    )));
                }
                Err(e) => return Err(Error::Ssh(format!("opening channel to {host}:{port}: {e}"))),
            }
        }
        Err(Error::Ssh(format!(
            "SSH connection lost{}",
            last_err.map(|e| format!(": {e}")).unwrap_or_default()
        )))
    }

    async fn shutdown(&self) {
        let conn = self.state.lock().await.take();
        if let Some(conn) = conn {
            let _ = conn
                .target
                .disconnect(Disconnect::ByApplication, "", "en")
                .await;
            if let Some(jump) = &conn.jump {
                let _ = jump.disconnect(Disconnect::ByApplication, "", "en").await;
            }
        }
    }
}

fn session_gone(e: &russh::Error) -> bool {
    matches!(
        e,
        russh::Error::SendError
            | russh::Error::Disconnect
            | russh::Error::HUP
            | russh::Error::Inconsistent
            | russh::Error::KeepaliveTimeout
            | russh::Error::InactivityTimeout
            | russh::Error::IO(_)
    )
}

async fn connect_chain(ssh: &SshConfig, secrets: &Secrets) -> Result<Connection> {
    match &ssh.jump {
        Some(jump) => {
            let jump_handle = tokio::time::timeout(
                CONNECT_TIMEOUT,
                connect_hop(
                    jump,
                    secrets.jump_password.as_deref(),
                    secrets.jump_passphrase.as_deref(),
                    None,
                ),
            )
            .await
            .map_err(|_| timed_out(jump))??;
            let target = tokio::time::timeout(
                CONNECT_TIMEOUT,
                connect_hop(
                    ssh,
                    secrets.ssh_password.as_deref(),
                    secrets.ssh_passphrase.as_deref(),
                    Some(&jump_handle),
                ),
            )
            .await
            .map_err(|_| timed_out(ssh))??;
            Ok(Connection {
                target,
                jump: Some(jump_handle),
            })
        }
        None => {
            let target = tokio::time::timeout(
                CONNECT_TIMEOUT,
                connect_hop(
                    ssh,
                    secrets.ssh_password.as_deref(),
                    secrets.ssh_passphrase.as_deref(),
                    None,
                ),
            )
            .await
            .map_err(|_| timed_out(ssh))??;
            Ok(Connection { target, jump: None })
        }
    }
}

fn timed_out(cfg: &SshConfig) -> Error {
    Error::Ssh(format!(
        "timed out connecting to {}:{}",
        cfg.host,
        ssh_port(cfg)
    ))
}

fn ssh_port(cfg: &SshConfig) -> u16 {
    if cfg.port == 0 { 22 } else { cfg.port }
}

fn client_config() -> Arc<client::Config> {
    Arc::new(client::Config {
        keepalive_interval: Some(KEEPALIVE),
        keepalive_max: 3,
        nodelay: true,
        ..Default::default()
    })
}

/// One SSH hop: TCP (or a channel through `via`), handshake, host-key check, authentication.
async fn connect_hop(
    cfg: &SshConfig,
    password: Option<&str>,
    passphrase: Option<&str>,
    via: Option<&Handle<Client>>,
) -> Result<Handle<Client>> {
    let port = ssh_port(cfg);
    let handler = Client {
        host: cfg.host.clone(),
        port,
    };
    let config = client_config();

    let mut handle = match via {
        Some(jump) => {
            let channel = jump
                .channel_open_direct_tcpip(cfg.host.as_str(), port as u32, "127.0.0.1", 0)
                .await
                .map_err(|e| {
                    Error::Ssh(format!(
                        "the jump host could not reach {}:{port}: {e}",
                        cfg.host
                    ))
                })?;
            client::connect_stream(config, channel.into_stream(), handler)
                .await
                .map_err(|e| handler_error(e, cfg, port))?
        }
        None => {
            let socket = TcpStream::connect((cfg.host.as_str(), port))
                .await
                .map_err(|e| Error::Ssh(format!("could not reach {}:{port}: {e}", cfg.host)))?;
            let _ = socket.set_nodelay(true);
            client::connect_stream(config, socket, handler)
                .await
                .map_err(|e| handler_error(e, cfg, port))?
        }
    };

    authenticate(&mut handle, cfg, password, passphrase).await?;
    Ok(handle)
}

fn handler_error(e: HandlerError, cfg: &SshConfig, port: u16) -> Error {
    match e {
        HandlerError::HostKey(msg) => Error::Ssh(msg),
        HandlerError::Russh(e) => Error::Ssh(format!(
            "SSH handshake with {}:{port} failed: {e}",
            cfg.host
        )),
    }
}

// ---------------------------------------------------------------------------
// Host keys: trust on first use
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum HandlerError {
    Russh(russh::Error),
    HostKey(String),
}

impl From<russh::Error> for HandlerError {
    fn from(e: russh::Error) -> Self {
        HandlerError::Russh(e)
    }
}

struct Client {
    host: String,
    port: u16,
}

impl Handler for Client {
    type Error = HandlerError;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> std::result::Result<bool, Self::Error> {
        let key = server_public_key.public_key();
        let Some(path) = known_hosts_path() else {
            note!("no home directory: SSH host keys are not verified");
            return Ok(true);
        };
        match russh::keys::check_known_hosts_path(&self.host, self.port, &key, &path) {
            Ok(true) => Ok(true),
            Ok(false) => {
                if let Err(e) = russh::keys::known_hosts::learn_known_hosts_path(
                    &self.host, self.port, &key, &path,
                ) {
                    note!("could not record the host key of {}: {e}", self.host);
                }
                Ok(true)
            }
            Err(russh::keys::Error::KeyChanged { line }) => Err(HandlerError::HostKey(format!(
                "host key for {} changed (it no longer matches {} line {line}). This could be a \
                 man-in-the-middle attack; if the server was reinstalled, remove that line and retry.",
                self.host,
                path.display()
            ))),
            Err(e) => {
                // An unreadable known_hosts must not lock the user out of their server.
                note!("known_hosts check failed ({e}); trusting {}", self.host);
                Ok(true)
            }
        }
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::home_dir()
}

fn known_hosts_path() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".ssh").join("known_hosts"))
}

fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\"))
        && let Some(home) = home_dir()
    {
        return home.join(rest);
    }
    if path == "~"
        && let Some(home) = home_dir()
    {
        return home;
    }
    PathBuf::from(path)
}

// ---------------------------------------------------------------------------
// Authentication
// ---------------------------------------------------------------------------

async fn authenticate(
    handle: &mut Handle<Client>,
    cfg: &SshConfig,
    password: Option<&str>,
    passphrase: Option<&str>,
) -> Result<()> {
    let target = format!("{}@{}", cfg.user, cfg.host);
    let rejected = |what: &str| Error::Ssh(format!("authentication to {target} failed ({what})"));
    let ssh_err = |e: russh::Error| Error::Ssh(format!("authentication to {target}: {e}"));

    match &cfg.auth {
        SshAuth::Password => {
            let password = password
                .filter(|p| !p.is_empty())
                .ok_or_else(|| Error::Ssh(format!("an SSH password is required for {target}")))?;
            let result = handle
                .authenticate_password(cfg.user.as_str(), password)
                .await
                .map_err(ssh_err)?;
            match result {
                russh::client::AuthResult::Success => Ok(()),
                russh::client::AuthResult::Failure {
                    remaining_methods, ..
                } => {
                    // Many servers expose passwords only through PAM's keyboard-interactive.
                    if remaining_methods.contains(&MethodKind::KeyboardInteractive)
                        && keyboard_interactive(handle, &cfg.user, password)
                            .await
                            .map_err(ssh_err)?
                    {
                        Ok(())
                    } else {
                        Err(rejected("wrong password or user"))
                    }
                }
            }
        }
        SshAuth::Key { path } => {
            let path = expand_tilde(path);
            let passphrase = passphrase.filter(|p| !p.is_empty()).map(str::to_owned);
            let key_path = path.clone();
            // Decrypting a key (bcrypt KDF) takes a noticeable fraction of a second.
            let key = tokio::task::spawn_blocking(move || {
                russh::keys::load_secret_key(&key_path, passphrase.as_deref())
            })
            .await
            .map_err(|e| Error::Ssh(e.to_string()))?
            .map_err(|e| match e {
                russh::keys::Error::KeyIsEncrypted => Error::Ssh(format!(
                    "the key {} is encrypted; a passphrase is required",
                    path.display()
                )),
                russh::keys::Error::IO(io) => {
                    Error::Ssh(format!("cannot read key {}: {io}", path.display()))
                }
                other => Error::Ssh(format!(
                    "cannot load key {} (wrong passphrase or unsupported format): {other}",
                    path.display()
                )),
            })?;
            let hash = handle
                .best_supported_rsa_hash()
                .await
                .ok()
                .flatten()
                .flatten();
            let result = handle
                .authenticate_publickey(
                    cfg.user.as_str(),
                    PrivateKeyWithHashAlg::new(Arc::new(key), hash),
                )
                .await
                .map_err(ssh_err)?;
            if result.success() {
                Ok(())
            } else {
                Err(rejected("the server did not accept this key"))
            }
        }
        SshAuth::Agent => {
            if authenticate_agent(handle, &cfg.user).await? {
                Ok(())
            } else {
                Err(rejected("no key in the ssh-agent was accepted"))
            }
        }
    }
}

/// Answers every keyboard-interactive prompt with the password.
async fn keyboard_interactive(
    handle: &mut Handle<Client>,
    user: &str,
    password: &str,
) -> std::result::Result<bool, russh::Error> {
    use russh::client::KeyboardInteractiveAuthResponse as R;
    let mut response = handle
        .authenticate_keyboard_interactive_start(user, None)
        .await?;
    for _ in 0..5 {
        match response {
            R::Success => return Ok(true),
            R::Failure { .. } => return Ok(false),
            R::InfoRequest { prompts, .. } => {
                let answers = prompts.iter().map(|_| password.to_owned()).collect();
                response = handle
                    .authenticate_keyboard_interactive_respond(answers)
                    .await?;
            }
        }
    }
    Ok(false)
}

#[cfg(unix)]
async fn authenticate_agent(handle: &mut Handle<Client>, user: &str) -> Result<bool> {
    use russh::keys::agent::client::AgentClient;
    let agent = AgentClient::connect_env().await.map_err(|e| {
        Error::Ssh(format!(
            "cannot reach the ssh-agent (is SSH_AUTH_SOCK set?): {e}"
        ))
    })?;
    try_agent_identities(handle, user, agent).await
}

#[cfg(windows)]
async fn authenticate_agent(handle: &mut Handle<Client>, user: &str) -> Result<bool> {
    use russh::keys::agent::client::AgentClient;
    if let Ok(agent) = AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent").await {
        return try_agent_identities(handle, user, agent).await;
    }
    let agent = AgentClient::connect_pageant()
        .await
        .map_err(|e| Error::Ssh(format!("cannot reach the ssh-agent or Pageant: {e}")))?;
    try_agent_identities(handle, user, agent).await
}

#[cfg(not(any(unix, windows)))]
async fn authenticate_agent(_handle: &mut Handle<Client>, _user: &str) -> Result<bool> {
    Err(Error::Unsupported(
        "ssh-agent is not supported on this platform.".into(),
    ))
}

#[allow(dead_code)]
async fn try_agent_identities<S>(
    handle: &mut Handle<Client>,
    user: &str,
    mut agent: russh::keys::agent::client::AgentClient<S>,
) -> Result<bool>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    use russh::keys::agent::AgentIdentity;
    let identities = agent
        .request_identities()
        .await
        .map_err(|e| Error::Ssh(format!("ssh-agent: {e}")))?;
    if identities.is_empty() {
        return Err(Error::Ssh(
            "the ssh-agent has no identities (run ssh-add)".into(),
        ));
    }
    let hash = handle
        .best_supported_rsa_hash()
        .await
        .ok()
        .flatten()
        .flatten();
    for identity in identities {
        let result = match &identity {
            AgentIdentity::PublicKey { key, .. } => {
                let hash = if matches!(key.algorithm(), ssh_key::Algorithm::Rsa { .. }) {
                    hash
                } else {
                    None
                };
                handle
                    .authenticate_publickey_with(user, key.clone(), hash, &mut agent)
                    .await
            }
            AgentIdentity::Certificate { certificate, .. } => {
                handle
                    .authenticate_certificate_with(user, certificate.clone(), hash, &mut agent)
                    .await
            }
        };
        match result {
            Ok(r) if r.success() => return Ok(true),
            Ok(_) => {}
            // A failing signer or a broken agent pipe: try the next identity, the
            // server decides whether we have run out of attempts.
            Err(e) => note!("ssh-agent identity rejected: {e}"),
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tilde_expansion() {
        assert_eq!(expand_tilde("/etc/key"), PathBuf::from("/etc/key"));
        if let Some(home) = home_dir() {
            assert_eq!(
                expand_tilde("~/.ssh/id_ed25519"),
                home.join(".ssh/id_ed25519")
            );
        }
    }

    #[test]
    fn dead_session_errors() {
        assert!(session_gone(&russh::Error::SendError));
        assert!(session_gone(&russh::Error::Disconnect));
        assert!(!session_gone(&russh::Error::RequestDenied));
    }
}
