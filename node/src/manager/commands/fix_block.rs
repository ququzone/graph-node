use graph::components::store::ChainStore as ChainStoreTrait;
use graph::prelude::{anyhow, StoreError};
use graph_store_postgres::ChainStore;
use std::sync::Arc;

pub async fn by_hash(chain_store: Arc<ChainStore>, hash: &str) -> anyhow::Result<()> {
    todo!()
}

pub async fn by_number(chain_store: Arc<ChainStore>, number: i32) -> anyhow::Result<()> {
    todo!()
}

pub async fn by_range(chain_store: Arc<ChainStore>, range: &str) -> anyhow::Result<()> {
    todo!()
}
