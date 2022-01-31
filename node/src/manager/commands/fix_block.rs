use graph::blockchain::BlockHash;
use graph::{
    components::store::ChainStore as ChainStoreTrait,
    prelude::{
        anyhow::{self, Context},
        StoreError,
    },
};
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

pub fn truncate(chain_store: Arc<ChainStore>) -> anyhow::Result<()> {
    chain_store
        .truncate_block_cache()
        .with_context(|| format!("Failed to truncate block cache for {}", chain_store.chain))
}
