use std::collections::HashSet;

use cml_crypto::{Ed25519KeyHash, TransactionHash};
use petgraph::{
    graph::NodeIndex,
    prelude::StableDiGraph,
    visit::{Dfs, EdgeRef},
};
use serde::{Deserialize, Serialize};

use crate::entity_index::UnconfirmedRewardTxIndex;

/// This store mantains a directed graph where nodes represent validated harvest or gauge buffering
/// TXs that can be cosigned by the verifier. An edge from a node M to N indicates that the TX N has
/// spent the `buffer_wallet` output of M. None of these TXs have been confirmed on-chain, and so a
/// path of nodes from a source (no incoming edges) to a sink (no outgoing edges) represents a
/// single TX-chain.
///
/// This store is not persisted to disk due to the ephemeral nature of unconfirmed TXs under the
/// presence of dropped TXs, chain rollback and TX racing between separate bots.
///
/// This store prevents double-harvesting of rewards for any given user. Before adding a TX to the
/// graph, we will walk up the chain to ensure that a user is never referenced more than once.
///
/// Note that the first TX (represented by a source node) in a chain needs a `buffer_wallet` input
/// from a confirmed TX. The graph must be notified of changes to this through the use of
/// `confirm_tx()` and `rollback()` methods.
#[derive(Serialize, Deserialize)]
pub struct ChainedRewardTxGraph {
    gr: StableDiGraph<NodeData, ()>,
    /// Contains all users who have already confirmed to have harvested in the current epoch.
    last_confirmed_user_harvests: HashSet<Ed25519KeyHash>,
    /// The hash of the last-confirmed TX with `buffer_wallet` UTxO output (in position 0).
    last_confirmed_buffer_wallet_tx_hash: TransactionHash,
}

impl UnconfirmedRewardTxIndex for ChainedRewardTxGraph {
    /// Attempt to add a new TX to the store. If the TX spends a valid `buffer_wallet` input and
    /// does not perform double-harvesting, it will be added and `true` is returned.
    ///
    /// IMPORTANT: this TX must have already been validated for proper `buffer_wallet` and
    /// `harvest_order` inputs, reward-bot signature and proper withdrawal amounts as dictated by
    /// the `lp_indexer`.
    fn try_add_tx(
        &mut self,
        buffer_wallet_input_tx_hash: TransactionHash,
        tx_hash: TransactionHash,
        tx_user_creds: HashSet<Ed25519KeyHash>,
    ) -> bool {
        for cred in &tx_user_creds {
            if self.last_confirmed_user_harvests.contains(cred) {
                return false;
            }
        }

        let is_tx_spending_confirmed =
            buffer_wallet_input_tx_hash == self.last_confirmed_buffer_wallet_tx_hash;

        if is_tx_spending_confirmed {
            let data = NodeData {
                tx_hash,
                user_creds: tx_user_creds,
            };
            self.gr.add_node(data);
            true
        } else if let Some(parent_ix) =
            self.find_parent_chain_tx(&buffer_wallet_input_tx_hash, &tx_user_creds)
        {
            let data = NodeData {
                tx_hash,
                user_creds: tx_user_creds,
            };
            let child_ix = self.gr.add_node(data);
            self.gr.add_edge(parent_ix, child_ix, ());
            true
        } else {
            false
        }
    }

    fn rollback(
        &mut self,
        rolled_back_user_creds: HashSet<Ed25519KeyHash>,
        confirmed_buffer_wallet_tx_hash: TransactionHash,
    ) {
        // The rollback has resulted in a change to the last-confirmed `buffer_wallet` UTxO,
        // so we delete all unconfirmed TXs, since they all depended on a `buffer_wallet` instance
        // that no longer exists.
        //
        // Note: there's no risk of double-harvesting here. There's 2 possible cases to consider
        // after rollback:
        // 1. The reward-bot may be able to resubmit and confirm some of TXs in the graph before
        // it was cleared. But it won't be possible for the bot to obtain a cosignature for a
        // double-harvest TX afterwards because the verifier will only cosign a TX that spends
        // the last-confirmed `buffer_wallet`, which is now out of date. Nothing can happen until
        // the verifier re-syncs and we have the latest confirmed gauge-buffer/harvest TX.
        //
        // 2. If the reward-bot wasn't able to resubmit any TXs, it and the verifier will need
        // to form the next TX from the same last-confirmed inputs, hence there is no chance to
        // effect a double-harvest.
        self.gr.clear();
        assert!(self
            .last_confirmed_user_harvests
            .is_superset(&rolled_back_user_creds));
        self.last_confirmed_user_harvests = self
            .last_confirmed_user_harvests
            .difference(&rolled_back_user_creds)
            .cloned()
            .collect();
        self.last_confirmed_buffer_wallet_tx_hash = confirmed_buffer_wallet_tx_hash;
    }

    /// Confirms the TX with the given TX-hash. If the confirmed TX has been cosigned by the
    /// verifier (and so there is an associated entry in the graph), properly update the graph
    /// state.
    ///
    /// Otherwise the confirmed TX was either a gauge-buffer action, or it was cosigned by another
    /// verifier. For the latter case, we will need the current confirmed user harvests.
    fn confirm_tx(
        &mut self,
        tx_hash: TransactionHash,
        confirmed_user_harvests: &HashSet<Ed25519KeyHash>,
    ) -> bool {
        if let Some(ix) = self.gr.node_indices().find(|ix| {
            let node_data = self.gr.node_weight(*ix).unwrap();
            node_data.tx_hash == tx_hash
        }) {
            let children_nodes: Vec<_> = self
                .gr
                .edges_directed(ix, petgraph::Direction::Outgoing)
                .map(|e| e.target())
                .collect();

            // The confirmed TX must be a source node (no incoming edges).
            assert!(self
                .gr
                .edges_directed(ix, petgraph::Direction::Incoming)
                .next()
                .is_none());
            let node_data = self.gr.remove_node(ix).unwrap();
            self.last_confirmed_buffer_wallet_tx_hash = node_data.tx_hash;

            assert_eq!(node_data.user_creds, *confirmed_user_harvests,);

            for cred in node_data.user_creds {
                assert!(!self.last_confirmed_user_harvests.contains(&cred));
                self.last_confirmed_user_harvests.insert(cred);
            }

            // Now need to remove all conflicting sub-graphs
            let root_nodes_to_delete = self.gr.node_indices().filter(|n_ix| {
                !children_nodes.contains(n_ix)
                    && self
                        .gr
                        .edges_directed(*n_ix, petgraph::Direction::Incoming)
                        .next()
                        .is_none()
            });

            let mut nodes_to_delete = vec![];
            for root_ix in root_nodes_to_delete {
                let mut dfs = Dfs::new(&self.gr, root_ix);
                while let Some(n_ix) = dfs.next(&self.gr) {
                    nodes_to_delete.push(n_ix);
                }
            }

            for n_ix in nodes_to_delete {
                self.gr.remove_node(n_ix);
            }
            true
        } else {
            // Here we're confirming a TX that was signed by another verifier.
            self.gr.clear();
            self.last_confirmed_user_harvests = confirmed_user_harvests.clone();
            self.last_confirmed_buffer_wallet_tx_hash = tx_hash;
            false
        }
    }

    /// Once an epoch ends, all users are able to claim rewards for this epoch. Also, all
    /// unconfirmed TXs are no longer valid due to expiring TTL, so they can all be removed.
    fn notify_end_of_epoch(&mut self) {
        self.last_confirmed_user_harvests.clear();
        self.gr.clear();
    }
}

impl ChainedRewardTxGraph {
    pub fn new() -> Self {
        Self {
            gr: StableDiGraph::new(),
            last_confirmed_user_harvests: HashSet::new(),
            // zero hash is fine since we need to wait for the first confirmed TX
            last_confirmed_buffer_wallet_tx_hash: TransactionHash::from([0; 32]),
        }
    }

    pub fn is_unconfirmed_and_tracked(&self, tx_hash: TransactionHash) -> bool {
        self.gr
            .node_weights()
            .any(|node_data| node_data.tx_hash == tx_hash)
    }

    fn find_parent_chain_tx(
        &self,
        tx_hash: &TransactionHash,
        user_creds: &HashSet<Ed25519KeyHash>,
    ) -> Option<NodeIndex> {
        // Ensure we don't double-harvest against confirmed orders in this epoch.
        for cred in user_creds {
            if self.last_confirmed_user_harvests.contains(cred) {
                return None;
            }
        }
        let mut curr_ix = self.gr.node_indices().find(|ix| {
            let node_data = self.gr.node_weight(*ix).unwrap();
            node_data.tx_hash == *tx_hash
        });

        let parent_ix = curr_ix;

        while let Some(ix) = curr_ix {
            let data = self.gr.node_weight(ix).unwrap();
            for cred in &data.user_creds {
                if user_creds.contains(cred) {
                    return None;
                }
            }
            curr_ix = self
                .gr
                .edges_directed(ix, petgraph::Direction::Incoming)
                .next()
                .map(|edge_ref| edge_ref.source());
        }

        parent_ix
    }
}

#[derive(Serialize, Deserialize)]
struct NodeData {
    tx_hash: TransactionHash,
    user_creds: HashSet<Ed25519KeyHash>,
}

#[cfg(test)]
mod tests {

    use std::collections::HashSet;

    use cml_crypto::{Ed25519KeyHash, TransactionHash};
    use serde::{Deserialize, Serialize};
    use spectrum_offchain::tx_hash::CanonicalHash;

    use crate::entity_index::{chained_tx_graph::ChainedRewardTxGraph, UnconfirmedRewardTxIndex};

    #[test]
    fn test_full_tx_chain_confirmation() {
        let last_confirmed_user_harvests: HashSet<_> = (0_u8..10).map(gen_key_hash).collect();
        let tx_hashes: Vec<_> = (0_u8..20).map(gen_tx_hash).collect();
        let mut gr = ChainedRewardTxGraph::new();
        gr.confirm_tx(tx_hashes[0], &last_confirmed_user_harvests);
        assert_eq!(gr.last_confirmed_user_harvests.len(), 10);
        let key_hash = gen_key_hash(10);

        let new_confirmed_users: HashSet<_> = vec![key_hash].into_iter().collect();
        assert!(gr.try_add_tx(tx_hashes[0], tx_hashes[1], new_confirmed_users.clone()));

        gr.confirm_tx(tx_hashes[1], &new_confirmed_users);
        assert_eq!(gr.last_confirmed_buffer_wallet_tx_hash, tx_hashes[1]);
        assert_eq!(gr.last_confirmed_user_harvests.len(), 11);

        // Failed because it doesn't spend valid TX containing `buffer_wallet`.
        let new_confirmed_users: HashSet<_> = vec![gen_key_hash(11)].into_iter().collect();
        assert!(!gr.try_add_tx(tx_hashes[2], tx_hashes[3], new_confirmed_users));

        // Failed because user_key already harvested
        let new_confirmed_users: HashSet<_> = vec![gen_key_hash(0)].into_iter().collect();
        assert!(!gr.try_add_tx(tx_hashes[1], tx_hashes[2], new_confirmed_users,));

        let mut conf = vec![];
        // Add TX chain T_2, T_3, ..., T_19
        for i in 1..19 {
            let key_hash = gen_key_hash(i as u8 + 10);
            let new_confirmed_users: HashSet<_> = vec![key_hash].into_iter().collect();
            conf.push(new_confirmed_users.clone());
            assert!(gr.try_add_tx(tx_hashes[i], tx_hashes[i + 1], new_confirmed_users));
        }

        // Now confirm all TXs in the chain.
        for (i, confirmed_users) in (2..20).zip(&conf) {
            gr.confirm_tx(tx_hashes[i], confirmed_users);
            assert_eq!(gr.last_confirmed_buffer_wallet_tx_hash, tx_hashes[i]);
        }

        // Finally let's rollback each TX
        for (i, confirmed_users) in (1..20).zip(&conf).rev() {
            gr.rollback(confirmed_users.clone(), tx_hashes[i - 1]);
            assert_eq!(gr.last_confirmed_buffer_wallet_tx_hash, tx_hashes[i - 1]);
        }
    }

    #[test]
    fn test_graph_fork() {
        // In this test we have the following setting:
        //
        // Last confirmed TX:  T_0
        // -----------------------------------
        // Unconfirmed TXs:
        //
        //           T_1            T_7
        //           /             /   \
        //         T_2           T_8   T_9
        //        /   \
        //      T_3   T_6
        //     /   \
        //   T_4   T_5
        //
        // Sequence of actions:
        // 1. Confirm T_1, which leads to removal of T_7, T_8 and T_9.
        // 2. Confirm T_2.
        // 3. Confirm T_3, which leads to removal of T_6
        // 3. Confirm T_4, which leads to removal of T_5 (and so no more nodes in the unconfirmed graph)
        let last_confirmed_user_harvests: HashSet<_> = (0_u8..10).map(gen_key_hash).collect();
        let tx_hashes: Vec<_> = (0_u8..20).map(gen_tx_hash).collect();
        let mut gr = ChainedRewardTxGraph::new();
        gr.confirm_tx(tx_hashes[0], &last_confirmed_user_harvests);

        assert!(gr.try_add_tx(
            tx_hashes[0],
            tx_hashes[1],
            vec![gen_key_hash(10)].into_iter().collect()
        ));
        let mut i = 1;
        let key_hash_2 = gen_key_hash(10 + i);
        assert!(gr.try_add_tx(tx_hashes[1], tx_hashes[2], vec![key_hash_2].into_iter().collect()));
        i += 1;
        let key_hash_3 = gen_key_hash(10 + i);
        assert!(gr.try_add_tx(tx_hashes[2], tx_hashes[3], vec![key_hash_3].into_iter().collect()));
        assert!(gr.try_add_tx(tx_hashes[2], tx_hashes[6], vec![key_hash_3].into_iter().collect())); // can safely have same key_hash
        i += 1;
        let key_hash_4 = gen_key_hash(10 + i);
        assert!(gr.try_add_tx(tx_hashes[3], tx_hashes[4], vec![key_hash_4].into_iter().collect()));
        assert!(gr.try_add_tx(tx_hashes[3], tx_hashes[5], vec![key_hash_4].into_iter().collect()));

        // right subgraph
        i += 1;
        let key_hash_7 = gen_key_hash(10 + i);
        assert!(gr.try_add_tx(tx_hashes[0], tx_hashes[7], vec![key_hash_7].into_iter().collect()));
        i += 1;
        let key_hash_8 = gen_key_hash(10 + i);
        assert!(gr.try_add_tx(tx_hashes[7], tx_hashes[8], vec![key_hash_8].into_iter().collect()));
        assert!(gr.try_add_tx(tx_hashes[7], tx_hashes[9], vec![key_hash_8].into_iter().collect()));

        // Confirming T_1, which will remove T_7 and its children nodes.
        let user = vec![gen_key_hash(10)].into_iter().collect();
        assert!(gr.confirm_tx(tx_hashes[1], &user));

        assert!(!gr.last_confirmed_user_harvests.contains(&key_hash_7));
        assert!(!gr.last_confirmed_user_harvests.contains(&key_hash_8));
        assert!(!gr.is_unconfirmed_and_tracked(tx_hashes[7]));
        assert!(!gr.is_unconfirmed_and_tracked(tx_hashes[8]));
        assert!(!gr.is_unconfirmed_and_tracked(tx_hashes[9]));

        let user = vec![key_hash_2].into_iter().collect();
        assert!(gr.confirm_tx(tx_hashes[2], &user));

        let user = vec![key_hash_3].into_iter().collect();
        assert!(gr.confirm_tx(tx_hashes[3], &user));
        assert!(!gr.is_unconfirmed_and_tracked(tx_hashes[6]));

        let user = vec![key_hash_4].into_iter().collect();
        assert!(gr.confirm_tx(tx_hashes[4], &user));
        assert!(!gr.is_unconfirmed_and_tracked(tx_hashes[5]));
        assert_eq!(gr.gr.node_count(), 0);
    }

    fn gen_key_hash(seed_value: u8) -> Ed25519KeyHash {
        let bytes = [seed_value; 28];
        bytes.into()
    }

    fn gen_tx_hash(seed_value: u8) -> TransactionHash {
        let bytes = [seed_value; 32];
        TransactionHash::from(bytes)
    }
}
