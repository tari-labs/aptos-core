// Copyright © Aptos Foundation
// SPDX-License-Identifier: Apache-2.0

use crate::{
    committer::ActionCommitter, executor::ActionExecutor, generator::ActionGenerator,
    utils::BasicProofReader, StateKeyHash,
};
use aptos_config::config::{RocksdbConfigs, StorageDirPaths};
use aptos_crypto::hash::SPARSE_MERKLE_PLACEHOLDER_HASH;
use aptos_db::state_merkle_db::StateMerkleDb;
use aptos_logger::info;
use aptos_scratchpad::SparseMerkleTree;
use aptos_types::state_store::{
    state_key::StateKey, state_storage_usage::StateStorageUsage, state_value::StateValue,
};
use std::{
    sync::{
        mpsc::{channel, Receiver, Sender},
        Arc,
    },
    thread::sleep,
    time::Duration,
};
use tokio::task::spawn_blocking;
pub enum Action {
    Read(StateKeyHash),
    Write(StateKey, Option<StateValue>),
}
#[derive(Clone, Copy)]
pub struct ActionConfig {
    // The number of read and write in each batch
    pub count: usize,
    // per million TODO: add read into the batch
    pub read_ratio: u32,
    // per million
    pub delete_ratio: u32,
    // largest write statekey generated to keep track of all keys generated
    pub last_state_key_ind: usize,
}

#[derive(Clone, Copy)]
pub enum ExecutionMode {
    AST,
    StatusQuo,
}

pub struct CommitMessage {
    // The updates to be applied to the state tree
    pub updates: Vec<(StateKey, Option<StateValue>)>,
    pub smt: Option<SparseMerkleTree<StateValue>>,
}

impl CommitMessage {
    pub fn new(
        updates: Vec<(StateKey, Option<StateValue>)>,
        smt: Option<SparseMerkleTree<StateValue>>,
    ) -> Self {
        Self { updates, smt }
    }
}

pub struct PipelineConfig {
    batch_size: usize,
    total_input_size: usize,
    db_path: String,
    execution_mode: ExecutionMode,
}

impl PipelineConfig {
    pub fn new(
        batch_size: usize,
        total_input_size: usize,
        db_path: String,
        execution_mode: ExecutionMode,
    ) -> Self {
        Self {
            batch_size,
            total_input_size,
            db_path,
            execution_mode,
        }
    }
}

pub struct Pipeline {
    config: PipelineConfig,
    sender: Sender<ActionConfig>,
    generator: ActionGenerator,
    executor: ActionExecutor,
    committer: ActionCommitter,
}

impl Pipeline {
    pub fn create_empty_smt() -> SparseMerkleTree<StateValue> {
        SparseMerkleTree::<StateValue>::new(
            *SPARSE_MERKLE_PLACEHOLDER_HASH,
            StateStorageUsage::new_untracked(),
        )
    }

    pub fn new(config: PipelineConfig) -> Self {
        // setup the channel between pipeline and genearator
        let (updates_sender, updates_receiver): (Sender<ActionConfig>, Receiver<ActionConfig>) =
            channel();

        // setup the channel between generate and executor
        let (action_sender, action_receiver): (Sender<Vec<Action>>, Receiver<Vec<Action>>) =
            channel();
        let generator = ActionGenerator::new(updates_receiver, action_sender);
        // setup the channel betwen the executor and committer
        let (committer_sender, committer_receiver): (
            Sender<CommitMessage>,
            Receiver<CommitMessage>,
        ) = channel();
        let state_merkle_db = Arc::new(
            StateMerkleDb::new(
                &StorageDirPaths::from_path(&config.db_path),
                RocksdbConfigs::default(),
                false,
                1000000usize,
            )
            .unwrap(),
        );
        let base_smt = Pipeline::create_empty_smt();
        //TODO(bowu): This is not a good proximation for the status quo since the the proofs are async fetched from the DB
        let proof_reader = BasicProofReader::new();

        let executor = match config.execution_mode {
            ExecutionMode::AST => ActionExecutor::new(
                config.execution_mode,
                proof_reader,
                base_smt.clone(),
                action_receiver,
                committer_sender,
            ),
            ExecutionMode::StatusQuo => ActionExecutor::new(
                config.execution_mode,
                proof_reader,
                base_smt.clone(),
                action_receiver,
                committer_sender,
            ),
        };

        let committer = ActionCommitter::new(state_merkle_db, committer_receiver, Some(base_smt));

        Self {
            config,
            sender: updates_sender,
            generator,
            executor,
            committer,
        }
    }

    pub fn run(&mut self) {
        let action_config = ActionConfig {
            count: self.config.batch_size,
            read_ratio: 0,
            delete_ratio: 0,
            last_state_key_ind: 0,
        };

        spawn_blocking(|| {
            self.generator.run();
        });

        spawn_blocking(|| {
            self.executor.run();
        });
        spawn_blocking(|| {
            self.committer.run();
        });

        let mut input_count = 0;

        loop {
            info!("total input count: {}", input_count);
            if input_count >= self.config.total_input_size {
                break;
            }
            self.sender.send(action_config).unwrap();
            sleep(Duration::from_secs(1));
            input_count += self.config.batch_size;
        }
    }
}
