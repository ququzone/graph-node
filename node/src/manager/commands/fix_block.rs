use graph::blockchain::BlockHash;
use graph::{
    components::store::ChainStore as ChainStoreTrait,
    prelude::{
        anyhow::{self, Context},
        StoreError,
    },
};
use graph_store_postgres::ChainStore;
use std::convert::TryFrom;
use std::io::{self, Write};
use std::sync::Arc;

pub async fn by_hash(chain_store: Arc<ChainStore>, hash: &str) -> anyhow::Result<()> {
    let block_hash = BlockHash::try_from(hash)?;
    todo!()
}

pub async fn by_number(chain_store: Arc<ChainStore>, number: i32) -> anyhow::Result<()> {
    let block_hashes = chain_store.block_hashes_by_block_number(number)?;

    // Try to resolve block number into a single block hash.
    let block_hash = match block_hashes.as_slice() {
        [] => anyhow::bail!("Found no block with number {}", number),
        [hash] => hash,
        _ => anyhow::bail!(
            "Found multiple blocks for the same number. Please specify a block hash instead."
        ),
    };
    todo!()
}

pub async fn by_range(chain_store: Arc<ChainStore>, range: &str) -> anyhow::Result<()> {
    todo!()
}

pub fn truncate(chain_store: Arc<ChainStore>) -> anyhow::Result<()> {
    if !prompt_for_confirmation()? {
        println!("Aborting.");
        return Ok(());
    }

    chain_store
        .truncate_block_cache()
        .with_context(|| format!("Failed to truncate block cache for {}", chain_store.chain))
}

fn prompt_for_confirmation() -> anyhow::Result<bool> {
    print!("This will delete all cached blocks.\nProceed? [y/N] ");
    io::stdout().flush()?;

    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    answer.make_ascii_lowercase();

    match answer.trim() {
        "y" | "yes" => Ok(true),
        _ => Ok(false),
    }
}
