//! An open connection: optional SSH tunnel + driver.

use std::sync::Arc;

use crate::driver::Driver;
use crate::error::Result;
use crate::model::{ConnectionConfig, Secrets};
use crate::ssh::Tunnel;

pub struct Session {
    pub config: ConnectionConfig,
    pub driver: Arc<dyn Driver>,
    pub tunnel: Option<Tunnel>,
    pub server_version: String,
}

impl Session {
    /// Opens the SSH tunnel (if configured), then connects the driver through it.
    pub async fn open(config: ConnectionConfig, secrets: Secrets) -> Result<Session> {
        todo!(
            "A3: open session {} ({})",
            config.name,
            secrets.db_password.is_some()
        )
    }

    pub async fn close(self) {
        todo!("A3: close session {}", self.config.name)
    }
}
