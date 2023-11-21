// Copyright © Aptos Foundation
// SPDX-License-Identifier: Apache-2.0
use super::{publishing::publish_util::Package, ReliableTransactionSubmitter};
use crate::{TransactionGenerator, TransactionGeneratorCreator};
use aptos_infallible::RwLock;
use aptos_sdk::{
    move_types::account_address::AccountAddress,
    transaction_builder::{aptos_stdlib, TransactionFactory},
    types::{chain_id::ChainId, transaction::SignedTransaction, LocalAccount},
};
use move_core_types::{
    ident_str,
    language_storage::{ModuleId, TypeTag},
};
use aptos_types::transaction::TransactionPayload;
use rand::{
    distributions::{Distribution, Standard},
    prelude::SliceRandom,
    rngs::StdRng,
    Rng, RngCore, SeedableRng,
};
use std::{
    cmp::{max, min},
    sync::Arc,
};

pub struct TournamentTransactionGenerator {
    rng: StdRng,
    num_tournaments: u64,
    txn_factory: TransactionFactory,
    admin_accounts: Arc<RwLock<Vec<LocalAccount>>>,
    player_accounts: Arc<RwLock<Vec<LocalAccount>>>,
}

impl TournamentTransactionGenerator {
    pub fn new(
        mut rng: StdRng,
        txn_factory: TransactionFactory,
        num_tournaments: u64,
        admin_accounts: Arc<RwLock<Vec<LocalAccount>>>,
        player_accounts: Arc<RwLock<Vec<LocalAccount>>>,
    ) -> Self {
        player_accounts.write().shuffle(&mut rng);
        Self {
            rng,
            txn_factory,
            num_tournaments,
            admin_accounts,
            player_accounts
        }
    }
}

impl TransactionGenerator for TournamentTransactionGenerator {
    fn generate_transactions(
        &mut self,
        account: &LocalAccount,
        num_to_create: usize,
    ) -> Vec<SignedTransaction> {

    }
}


pub struct TournamentTransactionGeneratorCreator {
    txn_factory: TransactionFactory,
    num_tournaments: u64,
    admin_accounts: Arc<RwLock<Vec<LocalAccount>>>,
    player_accounts: Arc<RwLock<Vec<LocalAccount>>>,
}


impl TournamentTransactionGeneratorCreator {
    pub async fn new(
        txn_factory: TransactionFactory,
        num_tournaments: u64,
        all_accounts: &mut [LocalAccount],
        txn_executor: &dyn ReliableTransactionSubmitter,
    ) -> Self {
        // Split accounts into admins and players.
        let admin_accounts = Arc::new(RwLock::new(all_accounts.iter().cloned().take(num_tournaments).collect()));
        let player_accounts = Arc::new(RwLock::new(all_accounts.iter().cloned().skip(num_tournaments).collect()));
        
        // Setup tournament for each of the admin accounts.
        let setup_tournament_txns = admin_accounts.iter().map(|admin_account| admin_account.sign_with_transaction_builder(txn_factory.payload(
            TransactionPayload::EntryFunction(EntryFunction::new(
                ModuleId::new(
                    AccountAddress::from_hex_literal("0x0d17edeafc6393d340df999ca4ca9b33bf35f19ad4d16fbf49e57eaa3da09421")?,
                    ident_str!("rps_utils").to_owned(),
                ),
                ident_str!("setup_tournament").to_owned(),
                vec![],
                vec![],
            ))
        )));

        txn_executor
            .execute_transactions(&setup_tournament_txns)
            .await
            .unwrap();
        
        
        Self {
            txn_factory,
            num_tournaments,
            admin_accounts,
            player_accounts
        }
    }
}

impl TransactionGeneratorCreator for TournamentTransactionGeneratorCreator {
    fn create_transaction_generator(&self) -> Box<dyn TransactionGenerator> {
        let rng = StdRng::from_entropy();
        
        // Create tournaments for each admin
        Box::new(TournamentTransactionGenerator::new(
            rng,
            self.txn_factory.clone(),
            self.num_tournaments,
            self.admin_accounts.clone(),
            self.player_accounts.clone()
        ))
    }
}