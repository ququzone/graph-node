use futures::compat::Future01CompatExt;
use graph::{
    anyhow::{bail, ensure},
    components::store::ChainStore as ChainStoreTrait,
    prelude::{
        anyhow::{self, anyhow, Context},
        hex,
        serde_json::{self, Value},
        web3::types::H256,
    },
    slog::Logger,
};
use graph_chain_ethereum::{EthereumAdapter, EthereumAdapterTrait};
use graph_store_postgres::ChainStore;
use json_structural_diff::{colorize as diff_to_string, JsonDiff};
use std::{
    io::{self, Write},
    sync::Arc,
};

pub async fn by_hash(
    hash: &str,
    chain_store: Arc<ChainStore>,
    ethereum_adapter: &EthereumAdapter,
    logger: &Logger,
) -> anyhow::Result<()> {
    // Create a BlockHash value to parse the input as a propper block hash
    let block_hash = {
        let hash = hash.trim_start_matches("0x");
        let hash = hex::decode(hash)
            .with_context(|| format!("Cannot parse H256 value from string `{}`", hash))?;
        H256::from_slice(&hash)
    };

    // Try to find a matching block from the store
    let cached_block = {
        let blocks = chain_store.blocks(&[block_hash])?;
        get_single_item("block", blocks)?
    };

    // Compare and report
    let comparison_result = {
        let result_set = compare_blocks(&[(block_hash, cached_block)], &ethereum_adapter, logger)
            .await
            .context("Failed to compare blocks")?;
        get_single_item("comparison", result_set)?
    };

    if let (hash, Some(diff)) = comparison_result {
        eprintln!("block {hash} diverges from cache:");
        eprintln!("{diff}");
        chain_store.delete_blocks(&[&hash])?;
    }
    Ok(())
}

pub async fn by_number(
    number: i32,
    chain_store: Arc<ChainStore>,
    ethereum_adapter: &EthereumAdapter,
    logger: &Logger,
) -> anyhow::Result<()> {
    let block_hashes = chain_store.block_hashes_by_block_number(number)?;
    let block_hash = get_single_item("block hash", block_hashes)?;

    // Try to find a matching block from the store
    let cached_blocks = chain_store.blocks(&[block_hash])?;
    let cached_block = get_single_item("block", cached_blocks)?;

    // Compare and report
    let comparison_result = {
        let result_set = compare_blocks(&[(block_hash, cached_block)], &ethereum_adapter, logger)
            .await
            .context("Failed to compare blocks")?;
        get_single_item("comparison", result_set)?
    };

    if let (hash, Some(diff)) = comparison_result {
        eprintln!("block {number} ({hash:?}) diverges from cache:");
        eprintln!("{diff}");
        chain_store.delete_blocks(&[&block_hash])?;
    }
    Ok(())
}

pub async fn by_range(
    chain_store: Arc<ChainStore>,
    ethereum_adapter: &EthereumAdapter,
    range: &str,
    logger: &Logger,
) -> anyhow::Result<()> {
    // Resolve a range of block numbers into a collection of blocks hashes
    let range = range.parse::<ranges::Range>()?;
    let cached_blocks = {
        let mut hashes_and_blocks: Vec<(H256, Value)> = Vec::new();
        let (min, max) = range.min_max()?;
        let max: i32 = match max {
            Some(x) => x,
            // When we have an open upper bound, we must check the number of the chain head block
            None => {
                let chain_head = chain_store.chain_head_ptr()?;
                match chain_head {
                    Some(block_ptr) => block_ptr.number,
                    None => {
                        anyhow::bail!("Could not find the chain head for {}", chain_store.chain)
                    }
                }
            }
        };
        // FIXME: This is not performant. We could fix this by hitting the database only once.
        for block_number in min..=max {
            let block_hashes = chain_store.block_hashes_by_block_number(block_number)?;
            let block_hash = get_single_item("block hash", block_hashes)?;

            // Try to find a matching block from the store
            let cached_blocks = chain_store.blocks(&[block_hash])?;
            let cached_block = get_single_item("block", cached_blocks)?;

            hashes_and_blocks.push((block_hash, cached_block))
        }
        hashes_and_blocks
    };

    // Compare and report
    let comparison_results = compare_blocks(cached_blocks.as_slice(), &ethereum_adapter, logger)
        .await
        .context("Failed to compare blocks")?;

    for comparison_result in comparison_results {
        if let (hash, Some(diff)) = comparison_result {
            eprintln!("block {hash} diverges from cache:");
            eprintln!("{diff}");
            chain_store.delete_blocks(&[&hash])?;
        }
    }
    Ok(())
}

pub fn truncate(chain_store: Arc<ChainStore>, skip_confirmation: bool) -> anyhow::Result<()> {
    if !skip_confirmation && !prompt_for_confirmation()? {
        println!("Aborting.");
        return Ok(());
    }

    chain_store
        .truncate_block_cache()
        .with_context(|| format!("Failed to truncate block cache for {}", chain_store.chain))
}

async fn compare_blocks(
    cached_blocks: &[(H256, Value)],
    ethereum_adapter: &EthereumAdapter,
    logger: &Logger,
) -> anyhow::Result<Vec<(H256, Option<String>)>> {
    let provider_blocks = fetch_provider_blocks(cached_blocks, ethereum_adapter, logger).await?;
    let pairs = cached_blocks.iter().zip(provider_blocks.iter());
    diff_blocks(pairs)
}

/// Request provider for fresh blocks from the input set
/// TODO: send renquests concurrently
async fn fetch_provider_blocks(
    cached_blocks: &[(H256, Value)],
    ethereum_adapter: &EthereumAdapter,
    logger: &Logger,
) -> anyhow::Result<Vec<Value>> {
    let mut provider_blocks = Vec::new();
    for (hash, _block) in cached_blocks {
        let provider_block = ethereum_adapter
            .block_by_hash(&logger, *hash)
            .compat()
            .await
            .context("failed to fetch block")?
            .ok_or_else(|| anyhow!("JRPC provider found no block with hash {hash}"))?;
        ensure!(
            provider_block.hash == Some(*hash),
            "Provider responded with a different block hash"
        );
        let provider_block_as_json = serde_json::to_value(provider_block)
            .context("failed to parse provider block as a JSON value")?;
        provider_blocks.push(provider_block_as_json);
    }
    anyhow::ensure!(
        cached_blocks.len() == provider_blocks.len(),
        "requested {} blocks from JRPC provider but got {} in response",
        cached_blocks.len(),
        provider_blocks.len()
    );
    Ok(provider_blocks)
}

/// Compare the block hashes from our cache against the ones received from the JRPC provider.
/// Returns a list of hashes diffs in text form, ready to be displayed to the user, in case the
/// blocks are different.
fn diff_blocks<'a, I>(pairs: I) -> anyhow::Result<Vec<(H256, Option<String>)>>
where
    I: Iterator<Item = (&'a (H256, Value), &'a Value)>,
{
    let mut comparison_results = Vec::new();
    for ((hash, cached_block), provider_block) in pairs {
        let provider_block = serde_json::to_value(provider_block)
            .context("failed to parse provider block as a JSON value")?;
        if cached_block != &provider_block {
            let diff_result = JsonDiff::diff(cached_block, &provider_block, false);
            // The diff result could potentially be a `Value::Null`, which is equivalent to not
            // being different at all.
            let json_diff = match diff_result.diff {
                None | Some(Value::Null) => None,
                Some(diff) => {
                    // Convert the JSON diff to a pretty-formatted text that will be displayed to
                    // the user
                    Some(diff_to_string(&diff, false))
                }
            };
            comparison_results.push((*hash, json_diff));
        }
    }
    Ok(comparison_results)
}

/// Asks users if they are certain about truncating the whole block cache.
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

/// Convenience function for extracting values from unary sets.
fn get_single_item<I, T>(name: &'static str, collection: I) -> anyhow::Result<T>
where
    I: IntoIterator<Item = T>,
{
    let mut iterator = collection.into_iter();
    match (iterator.next(), iterator.next()) {
        (Some(a), None) => Ok(a),
        (None, None) => bail!("Expected a single {name} but found none."),
        _ => bail!("Expected a single {name} but found multiple occurrences."),
    }
}

mod ranges {
    use graph::prelude::anyhow::{self, bail};
    use std::str::FromStr;

    pub(super) struct Range {
        pub(super) lower_bound: Option<i32>,
        pub(super) upper_bound: Option<i32>,
        pub(super) inclusive: bool,
    }

    impl Range {
        fn new(lower_bound: Option<i32>, upper_bound: Option<i32>, inclusive: bool) -> Self {
            Self {
                lower_bound,
                upper_bound,
                inclusive,
            }
        }

        pub(super) fn min_max(&self) -> anyhow::Result<(i32, Option<i32>)> {
            let min = match self.lower_bound {
                None | Some(0) => 1,
                Some(x) if x < 0 => anyhow::bail!("Negative block number"),
                Some(x) => x,
            };
            let inclusive = if self.inclusive { 1 } else { 0 };
            let max = self.upper_bound.map(|x| x + inclusive);
            Ok((min, max))
        }
    }

    impl FromStr for Range {
        type Err = anyhow::Error;

        fn from_str(s: &str) -> Result<Self, Self::Err> {
            const INCLUSIVE: &str = "..=";
            const EXCLUSIVE: &str = "..";
            if !s.contains(INCLUSIVE) && !s.contains(EXCLUSIVE) {
                bail!("Malformed range expression")
            }
            let (separator, inclusive) = if s.contains("..=") {
                (INCLUSIVE, true)
            } else {
                (EXCLUSIVE, false)
            };
            let split: Vec<&str> = s.split(separator).collect();
            let range = match split.as_slice() {
                // open upper bounds are always inclusive
                ["", ""] => Range::new(None, None, true),
                [start, ""] => {
                    let start: i32 = start.parse::<i32>()?;
                    // open upper bounds are always inclusive
                    Range::new(Some(start), None, true)
                }
                ["", end] => {
                    let end = end.parse::<i32>()?;
                    Range::new(None, Some(end), inclusive)
                }
                [start, end] => {
                    let start: i32 = start.parse::<i32>()?;
                    let end: i32 = end.parse::<i32>()?;
                    if start > end {
                        bail!("Invalid range")
                    }
                    Range::new(Some(start), Some(end), inclusive)
                }
                _ => bail!("Invalid range"),
            };
            Ok(range)
        }
    }
}
