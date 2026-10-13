//! Engine implementations of `Driver`.

pub mod mongo;
pub mod mysql;
pub mod postgres;
pub mod sqlite;

use std::sync::Arc;

use crate::Engine;
use crate::driver::Driver;
use crate::error::Result;
use crate::model::ConnectParams;

/// Connects with the driver for `params.config.engine`.
pub async fn connect(params: &ConnectParams) -> Result<Arc<dyn Driver>> {
    match params.config.engine {
        Engine::Sqlite => sqlite::connect(params).await,
        Engine::Postgres => postgres::connect(params).await,
        Engine::Mysql | Engine::Mariadb => mysql::connect(params).await,
        Engine::Mongodb => mongo::connect(params).await,
    }
}
