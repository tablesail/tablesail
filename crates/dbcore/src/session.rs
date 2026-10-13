//! An open connection: optional SSH tunnel + driver.

use std::sync::Arc;

use crate::Engine;
use crate::driver::Driver;
use crate::drivers;
use crate::error::Result;
use crate::model::{ConnectParams, ConnectionConfig, Secrets};
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
        let port = if config.port == 0 {
            config.engine.default_port()
        } else {
            config.port
        };

        // SQLite is a local file: there is nothing to tunnel to.
        let tunnel = match &config.ssh {
            Some(ssh) if config.engine != Engine::Sqlite => {
                Some(Tunnel::open(ssh, &secrets, &config.host, port).await?)
            }
            _ => None,
        };

        let params = match &tunnel {
            Some(t) => ConnectParams {
                config: config.clone(),
                password: secrets.db_password.clone(),
                host: "127.0.0.1".to_owned(),
                port: t.local_port,
            },
            None => ConnectParams {
                config: config.clone(),
                password: secrets.db_password.clone(),
                host: config.host.clone(),
                port,
            },
        };

        let connected = async {
            let driver = drivers::connect(&params).await?;
            match driver.server_version().await {
                Ok(version) => Ok((driver, version)),
                Err(e) => {
                    driver.close().await;
                    Err(e)
                }
            }
        }
        .await;

        match connected {
            Ok((driver, server_version)) => Ok(Session {
                config,
                driver,
                tunnel,
                server_version,
            }),
            Err(e) => {
                if let Some(t) = tunnel {
                    t.close().await;
                }
                Err(e)
            }
        }
    }

    /// Closes the driver, then the tunnel.
    pub async fn close(self) {
        self.driver.close().await;
        if let Some(tunnel) = self.tunnel {
            tunnel.close().await;
        }
    }
}
