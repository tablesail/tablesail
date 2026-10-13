use std::sync::Arc;

use crate::driver::Driver;
use crate::error::Result;
use crate::model::ConnectParams;

pub async fn connect(params: &ConnectParams) -> Result<Arc<dyn Driver>> {
    todo!("mysql driver: connect to {}", params.host)
}
