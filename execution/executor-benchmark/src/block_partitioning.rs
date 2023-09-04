// Copyright © Aptos Foundation

use crate::{metrics::TIMER, pipeline::ExecuteBlockMessage};
use aptos_block_partitioner::{BlockPartitioner, PartitionerConfig};
use aptos_crypto::HashValue;
use aptos_logger::info;
use aptos_types::{
    block_executor::partitioner::{ExecutableBlock, ExecutableTransactions},
    transaction::Transaction,
};
use std::time::Instant;
use aptos_transaction_orderer::batch_orderer_with_window::SequentialDynamicWindowOrderer;
use aptos_transaction_orderer::block_orderer::{BatchedBlockOrdererWithWindow, BlockOrderer};
use aptos_types::transaction::analyzed_transaction::AnalyzedTransaction;

pub(crate) struct BlockPartitioningStage {
    num_executor_shards: usize,
    num_blocks_processed: usize,
    maybe_partitioner: Option<Box<dyn BlockPartitioner>>,
}

impl BlockPartitioningStage {
    pub fn new(num_shards: usize, partitioner_config: PartitionerConfig) -> Self {
        let maybe_partitioner = if num_shards <= 1 {
            None
        } else {
            let partitioner = partitioner_config.build();
            Some(partitioner)
        };

        Self {
            num_executor_shards: num_shards,
            num_blocks_processed: 0,
            maybe_partitioner,
        }
    }

    pub fn process(&mut self, mut txns: Vec<Transaction>) -> ExecuteBlockMessage {
        let current_block_start_time = Instant::now();
        info!(
            "In iteration {}, received {:?} transactions.",
            self.num_blocks_processed,
            txns.len()
        );
        let block_id = HashValue::random();
        let block: ExecutableBlock = match &self.maybe_partitioner {
            // None => (block_id, txns).into(),
            None => {
                let n_txns = txns.len();
                let last_txn = txns.pop().unwrap();
                let analyzed_transactions = txns.into_iter().map(|t| t.into()).collect();
                let block_orderer = BatchedBlockOrdererWithWindow::new(
                    SequentialDynamicWindowOrderer::default(),
                    n_txns,
                    1000,
                );

                let mut ordered_txns: Vec<Transaction> = vec![];
                block_orderer
                    .order_transactions(
                        analyzed_transactions,
                        |txns: Vec<AnalyzedTransaction>| -> Result<(), ()> {
                            ordered_txns.extend(txns.into_iter().map(|t| t.into()));
                            Ok(())
                        }
                    )
                    .unwrap();
                ordered_txns.push(last_txn);
                (block_id, ordered_txns).into()
            },
            Some(partitioner) => {
                let last_txn = txns.pop().unwrap();
                let analyzed_transactions = txns.into_iter().map(|t| t.into()).collect();
                let timer = TIMER.with_label_values(&["partition"]).start_timer();
                timer.stop_and_record();
                let mut partitioned_txns =
                    partitioner.partition(analyzed_transactions, self.num_executor_shards);
                partitioned_txns.add_checkpoint_txn(last_txn);
                ExecutableBlock::new(block_id, ExecutableTransactions::Sharded(partitioned_txns))
            },
        };
        self.num_blocks_processed += 1;
        ExecuteBlockMessage {
            current_block_start_time,
            partition_time: Instant::now().duration_since(current_block_start_time),
            block,
        }
    }
}
