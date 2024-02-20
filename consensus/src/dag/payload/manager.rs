use super::{
    payload_fetcher::PayloadRequester,
    store::{DagPayloadStore, DagPayloadStoreError},
};
use crate::dag::{dag_store::DagStore, types::DagPayload, CertifiedNode};
use anyhow::bail;
use aptos_collections::BoundedVecDeque;
use aptos_consensus_types::{
    common::Payload,
    dag_payload::{DecoupledPayload, PayloadDigest},
};
use aptos_logger::{debug, error};
use aptos_types::transaction::SignedTransaction;
use dashmap::DashMap;
use futures::{future::BoxFuture, FutureExt};
use std::{ops::DerefMut, sync::Arc};
use tokio::sync::oneshot;

pub trait TDagPayloadResolver: Send + Sync {
    fn get_payload_if_exists(&self, node: &CertifiedNode) -> Option<Arc<DecoupledPayload>>;
    fn add_payload(&self, payload: DecoupledPayload) -> anyhow::Result<()>;
}

pub struct DagPayloadManager {
    dag_store: Arc<DagStore>,
    payload_store: Arc<DagPayloadStore>,
    requester: PayloadRequester,
    waiters: DashMap<PayloadDigest, BoundedVecDeque<oneshot::Sender<Vec<SignedTransaction>>>>,
}

impl DagPayloadManager {
    pub fn new(
        dag_store: Arc<DagStore>,
        payload_store: Arc<DagPayloadStore>,
        requester: PayloadRequester,
    ) -> Self {
        Self {
            dag_store,
            payload_store,
            requester,
            waiters: DashMap::new(),
        }
    }

    pub fn insert_payload(&self, node_payload: DecoupledPayload) -> anyhow::Result<()> {
        // Insert payload into store
        // Cancel fetch request
        // Notify waiters
        let info = node_payload.info();
        let digest = *info.digest();
        let payload = node_payload.payload().clone();
        self.payload_store.insert(node_payload)?;
        if let Err(e) = self.requester.cancel(info) {
            debug!("cannot send cancel {:?}", e);
        }
        if let Some((_, waiters)) = self.waiters.remove(&digest) {
            for tx in waiters.into_iter() {
                let Payload::DirectMempool(txns) = &payload else {
                    unreachable!("other payloads are not supported");
                };
                if let Err(e) = tx.send(txns.clone()) {
                    debug!("unable to send: {:?}", e);
                }
            }
        }

        Ok(())
    }

    fn retrieve_payload(
        self: Arc<Self>,
        node: &CertifiedNode,
    ) -> anyhow::Result<BoxFuture<Result<Vec<SignedTransaction>, oneshot::error::RecvError>>> {
        debug!("retrieving payload for node {}", node.id());
        let (tx, rx) = oneshot::channel();
        let DagPayload::Decoupled(info) = node.payload() else {
            unreachable!("payload manager is only for decouple DAG payload")
        };
        self.waiters
            .entry(*info.digest())
            .or_insert_with(|| BoundedVecDeque::new(1))
            .deref_mut()
            .push_back(tx);
        match self.payload_store.get(info.id(), info.digest()) {
            Ok(payload) => {
                let Payload::DirectMempool(txns) = payload.payload() else {
                    unreachable!("other payloads are not supported");
                };
                debug!("payload available {}", payload.id());
                if let Some(tx) = self
                    .waiters
                    .remove(info.digest())
                    .expect("must exist")
                    .1
                    .pop_front()
                {
                    tx.send(txns.clone()).ok();
                }
                Ok(async move { rx.await }.boxed())
            },
            Err(DagPayloadStoreError::Missing(_)) => {
                debug!("payload missing {}", info.id());
                let responders = node.parents_metadata().map(|m| *m.author()).collect();
                let request_rx = self.requester.request(info.clone(), responders)?;
                let me = self.clone();
                let fut = async move {
                    let node_payload = request_rx.await?;
                    let Payload::DirectMempool(txns) = node_payload.payload() else {
                        unreachable!("other payloads are not supported");
                    };
                    if let Some(tx) = me
                        .waiters
                        .remove(info.digest())
                        .expect("must exist")
                        .1
                        .pop_front()
                    {
                        tx.send(txns.clone()).ok();
                    }
                    rx.await
                };
                Ok(fut.boxed())
            },
            Err(_) => {
                error!("unable to send request fetch {}", info.id());
                bail!("error fetching");
            },
        }
    }

    fn prefetch_payload(&self, node: &CertifiedNode) {
        let DagPayload::Decoupled(info) = node.payload() else {
            unreachable!("payload manager is only for decouple DAG payload")
        };
        match self.payload_store.get(info.id(), info.digest()) {
            Ok(_) => {},
            Err(DagPayloadStoreError::Missing(_)) => {
                debug!("prefetch payload missing {}", node.id());
                let responders = node.parents_metadata().map(|m| *m.author()).collect();
                self.requester.request(info.clone(), responders).ok();
            },
            Err(err) => {
                error!("unable to send request prefetch {:?}, {}", err, node.id());
            },
        }
    }
}

impl TDagPayloadResolver for DagPayloadManager {
    fn get_payload_if_exists(&self, node: &CertifiedNode) -> Option<Arc<DecoupledPayload>> {
        let DagPayload::Decoupled(info) = node.payload() else {
            unreachable!("payload manager is only for decouple DAG payload")
        };
        self.payload_store.get(info.id(), info.digest()).ok()
    }

    fn add_payload(&self, payload: DecoupledPayload) -> anyhow::Result<()> {
        self.insert_payload(payload)
    }
}
