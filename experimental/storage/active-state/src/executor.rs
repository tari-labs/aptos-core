// Copyright © Aptos Foundation
// SPDX-License-Identifier: Apache-2.0

// Support AST smt updates and status-quo updates
// Assume all read can be find in the active state tree
// To simuate the status-quo, the smt is rebuilt from scratch after x batches

use crate::{
    metrics::UPDATE_CNT,
    pipeline::{Action, CommitMessage, ExecutionMode},
    utils::BasicProofReader,
    ActiveState,
};
use aptos_crypto::hash::CryptoHash;
use aptos_logger::info;
use aptos_scratchpad::SparseMerkleTree;
use aptos_types::state_store::state_value::StateValue;
use std::sync::mpsc::{Receiver, Sender};

pub struct ActionExecutor {
    mode: ExecutionMode,
    proof_reader: BasicProofReader,
    current_smt: SparseMerkleTree<StateValue>,
    active_state: Option<ActiveState>,
    receiver: Receiver<Vec<Action>>,
    committer_sender: Sender<CommitMessage>,
}

impl ActionExecutor {
    pub fn new(
        mode: ExecutionMode,
        proof_reader: BasicProofReader,
        current_smt: SparseMerkleTree<StateValue>,
        receiver: Receiver<Vec<Action>>,
        committer_sender: Sender<CommitMessage>,
    ) -> Self {
        match mode {
            ExecutionMode::AST => {
                let active_state = ActiveState::new(current_smt.clone(), 1000);
                Self {
                    mode,
                    proof_reader,
                    current_smt,
                    active_state: Some(active_state),
                    receiver,
                    committer_sender,
                }
            },
            ExecutionMode::StatusQuo => Self {
                mode,
                proof_reader,
                current_smt,
                active_state: None,
                receiver,
                committer_sender,
            },
        }
    }

    pub fn run(&mut self) {
        loop {
            let actions = self.receiver.recv().expect("Failure in receiving actions");
            let mut updates = Vec::new();
            for action in actions.into_iter() {
                match action {
                    Action::Read(state_key_hash) => {
                        unimplemented!();
                    },
                    Action::Write(state_key, state_value_opt) => {
                        updates.push((state_key, state_value_opt));
                    },
                }
            }
            let update_cnt = updates.len();
            match self.mode {
                ExecutionMode::AST => {
                    self.active_state
                        .as_mut()
                        .unwrap()
                        .batch_put_value_set(updates)
                        .unwrap();
                    // nothing to be done for now
                    let commit_msg = CommitMessage::new(Vec::new(), None);
                    self.committer_sender.send(commit_msg).unwrap();
                },
                ExecutionMode::StatusQuo => {
                    let new_smt = self
                        .current_smt
                        .batch_update(
                            updates
                                .iter()
                                .map(|(k, v)| (k.hash(), v.as_ref()))
                                .collect(),
                            &self.proof_reader,
                        )
                        .unwrap();
                    let commit_msg = CommitMessage::new(updates, Some(new_smt));
                    self.committer_sender.send(commit_msg).unwrap();
                },
            };
            info!("Update count: {}", update_cnt);
            UPDATE_CNT.inc_by(update_cnt as f64);
        }
    }
}
