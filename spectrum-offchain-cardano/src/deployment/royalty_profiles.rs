//! Additive deployment routing for protected pool validators.
//!
//! Protected scripts cannot replace their legacy entries: pool UTxOs remain
//! addressed by the validator hash they were created with. This registry
//! resolves royalty bundles and ordinary FeeSwitch pool validators by hash.

use cml_crypto::ScriptHash;

use super::{
    DeployedValidator, DeployedValidatorErased, DeployedValidatorRef, DeployedValidators, ProtocolDeployment,
};

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum RoyaltyPoolKind {
    LegacyV1,
    LegacyV1LedgerFixed,
    LegacyV2,
    ProtectedV1,
    ProtectedV2,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum ProtectedFeeSwitchKind {
    Regular,
    BiDir,
}

/// The reference scripts that must be used with a particular royalty pool
/// generation.  Keeping these together prevents a pool validator from being
/// spent with an order/DAO/withdrawal script from another deployment.
#[derive(Clone)]
pub struct RoyaltyPoolScriptBundle {
    pub pool: DeployedValidatorRef,
    pub deposit: DeployedValidatorRef,
    pub redeem: DeployedValidatorRef,
    pub dao: DeployedValidatorRef,
    pub dao_request: DeployedValidatorRef,
    pub withdraw: DeployedValidatorRef,
    pub withdraw_request: DeployedValidatorRef,
}

#[derive(Clone)]
pub struct RoutedRoyaltyPool {
    pub kind: RoyaltyPoolKind,
    pub scripts: RoyaltyPoolScriptBundle,
}

/// Owns the legacy royalty bundles and optional protected royalty and
/// FeeSwitch validators. A legacy deployment JSON keeps its old behaviour.
#[derive(Clone)]
pub struct RoyaltyPoolProfiles {
    legacy_v1: RoyaltyPoolScriptBundle,
    legacy_v1_ledger_fixed: RoyaltyPoolScriptBundle,
    legacy_v2: RoyaltyPoolScriptBundle,
    protected_v1: Option<RoyaltyPoolScriptBundle>,
    protected_v2: Option<RoyaltyPoolScriptBundle>,
    protected_fee_switch: Option<(DeployedValidatorRef, DeployedValidatorRef)>,
}

/// Pulled (transaction-builder ready) counterpart of [`RoyaltyPoolProfiles`].
/// Unlike the discovery profile, it owns the exact reference UTxO selected for
/// each validator and therefore can safely be used while constructing a spend.
#[derive(Clone)]
pub struct RoyaltyPoolExecutionBundle {
    pub pool: DeployedValidatorErased,
    pub deposit: DeployedValidatorErased,
    pub redeem: DeployedValidatorErased,
    pub dao: DeployedValidatorErased,
    pub dao_request: DeployedValidatorErased,
    pub withdraw: DeployedValidatorErased,
    pub withdraw_request: DeployedValidatorErased,
}

#[derive(Clone)]
pub struct RoyaltyPoolExecutionProfiles {
    legacy_v1: RoyaltyPoolExecutionBundle,
    legacy_v1_ledger_fixed: RoyaltyPoolExecutionBundle,
    legacy_v2: RoyaltyPoolExecutionBundle,
    protected_v1: Option<RoyaltyPoolExecutionBundle>,
    protected_v2: Option<RoyaltyPoolExecutionBundle>,
    protected_fee_switch: Option<(DeployedValidatorErased, DeployedValidatorErased)>,
}

impl RoyaltyPoolExecutionProfiles {
    pub fn legacy_only(legacy: &ProtocolDeployment) -> Self {
        Self {
            legacy_v1: execution_v1_bundle(legacy, false),
            legacy_v1_ledger_fixed: execution_v1_bundle(legacy, true),
            legacy_v2: execution_v2_bundle(legacy),
            protected_v1: None,
            protected_v2: None,
            protected_fee_switch: None,
        }
    }

    pub fn with_protected(legacy: &ProtocolDeployment, protected: &ProtocolDeployment) -> Self {
        let mut profiles = Self::legacy_only(legacy);
        profiles.protected_v1 = Some(execution_v1_bundle(protected, true));
        profiles.protected_v2 = Some(execution_v2_bundle(protected));
        profiles
    }

    pub fn with_protected_fee_switch(
        mut self,
        regular: DeployedValidatorErased,
        bi_dir: DeployedValidatorErased,
    ) -> Self {
        self.protected_fee_switch = Some((regular, bi_dir));
        self
    }

    pub fn resolve_fee_switch(
        &self,
        kind: ProtectedFeeSwitchKind,
    ) -> Option<&DeployedValidatorErased> {
        let (regular, bi_dir) = self.protected_fee_switch.as_ref()?;
        Some(match kind {
            ProtectedFeeSwitchKind::Regular => regular,
            ProtectedFeeSwitchKind::BiDir => bi_dir,
        })
    }

    pub fn resolve(&self, kind: RoyaltyPoolKind) -> Option<&RoyaltyPoolExecutionBundle> {
        match kind {
            RoyaltyPoolKind::LegacyV1 => Some(&self.legacy_v1),
            RoyaltyPoolKind::LegacyV1LedgerFixed => Some(&self.legacy_v1_ledger_fixed),
            RoyaltyPoolKind::LegacyV2 => Some(&self.legacy_v2),
            RoyaltyPoolKind::ProtectedV1 => self.protected_v1.as_ref(),
            RoyaltyPoolKind::ProtectedV2 => self.protected_v2.as_ref(),
        }
    }
}

impl RoyaltyPoolProfiles {
    pub fn legacy_only(legacy: &DeployedValidators) -> Self {
        Self {
            legacy_v1: v1_bundle(legacy, false),
            legacy_v1_ledger_fixed: v1_bundle(legacy, true),
            legacy_v2: v2_bundle(legacy),
            protected_v1: None,
            protected_v2: None,
            protected_fee_switch: None,
        }
    }

    /// The protected deployment JSON has the new V1 pool in
    /// `royaltyPoolLedgerFixed` and the new V2 pool in `royaltyPoolV2`.
    /// Legacy entries are still read from `legacy` and never overwritten.
    pub fn with_protected(legacy: &DeployedValidators, protected: &DeployedValidators) -> Self {
        let mut profiles = Self::legacy_only(legacy);
        profiles.protected_v1 = Some(v1_bundle(protected, true));
        profiles.protected_v2 = Some(v2_bundle(protected));
        profiles
    }

    pub fn with_protected_fee_switch(mut self, deployment: &DeployedValidators) -> Self {
        self.protected_fee_switch = deployment.protected_fee_switch.as_ref().map(|protected| {
            (protected.pool.clone(), protected.pool_bi_dir.clone())
        });
        self
    }

    pub fn resolve_fee_switch(&self, hash: &ScriptHash) -> Option<ProtectedFeeSwitchKind> {
        let (regular, bi_dir) = self.protected_fee_switch.as_ref()?;
        if regular.hash == *hash {
            Some(ProtectedFeeSwitchKind::Regular)
        } else if bi_dir.hash == *hash {
            Some(ProtectedFeeSwitchKind::BiDir)
        } else {
            None
        }
    }

    pub fn fee_switch_ref(&self, kind: ProtectedFeeSwitchKind) -> Option<&DeployedValidatorRef> {
        let (regular, bi_dir) = self.protected_fee_switch.as_ref()?;
        Some(match kind {
            ProtectedFeeSwitchKind::Regular => regular,
            ProtectedFeeSwitchKind::BiDir => bi_dir,
        })
    }

    pub fn resolve(&self, hash: &ScriptHash) -> Option<RoutedRoyaltyPool> {
        let candidates = [
            (RoyaltyPoolKind::LegacyV1, &self.legacy_v1),
            (RoyaltyPoolKind::LegacyV1LedgerFixed, &self.legacy_v1_ledger_fixed),
            (RoyaltyPoolKind::LegacyV2, &self.legacy_v2),
        ];
        for (kind, scripts) in candidates {
            if scripts.pool.hash == *hash {
                return Some(RoutedRoyaltyPool {
                    kind,
                    scripts: scripts.clone(),
                });
            }
        }
        for (kind, scripts) in [
            (RoyaltyPoolKind::ProtectedV1, self.protected_v1.as_ref()),
            (RoyaltyPoolKind::ProtectedV2, self.protected_v2.as_ref()),
        ] {
            if let Some(scripts) = scripts.filter(|scripts| scripts.pool.hash == *hash) {
                return Some(RoutedRoyaltyPool {
                    kind,
                    scripts: scripts.clone(),
                });
            }
        }
        None
    }

    pub fn resolve_deposit(&self, hash: &ScriptHash) -> Option<RoyaltyPoolKind> {
        self.all()
            .find_map(|(kind, bundle)| (bundle.deposit.hash == *hash).then_some(kind))
    }

    pub fn resolve_redeem(&self, hash: &ScriptHash) -> Option<RoyaltyPoolKind> {
        self.all()
            .find_map(|(kind, bundle)| (bundle.redeem.hash == *hash).then_some(kind))
    }

    pub fn resolve_dao_request(&self, hash: &ScriptHash) -> Option<RoyaltyPoolKind> {
        self.all()
            .find_map(|(kind, bundle)| (bundle.dao_request.hash == *hash).then_some(kind))
    }

    pub fn resolve_withdraw_request(&self, hash: &ScriptHash) -> Option<RoyaltyPoolKind> {
        self.all()
            .find_map(|(kind, bundle)| (bundle.withdraw_request.hash == *hash).then_some(kind))
    }

    fn all(&self) -> impl Iterator<Item = (RoyaltyPoolKind, &RoyaltyPoolScriptBundle)> {
        [
            (RoyaltyPoolKind::LegacyV1, Some(&self.legacy_v1)),
            (
                RoyaltyPoolKind::LegacyV1LedgerFixed,
                Some(&self.legacy_v1_ledger_fixed),
            ),
            (RoyaltyPoolKind::LegacyV2, Some(&self.legacy_v2)),
            (RoyaltyPoolKind::ProtectedV1, self.protected_v1.as_ref()),
            (RoyaltyPoolKind::ProtectedV2, self.protected_v2.as_ref()),
        ]
        .into_iter()
        .filter_map(|(kind, bundle)| bundle.map(|bundle| (kind, bundle)))
    }
}

fn v1_bundle(deployment: &DeployedValidators, ledger_fixed: bool) -> RoyaltyPoolScriptBundle {
    RoyaltyPoolScriptBundle {
        pool: if ledger_fixed {
            deployment.royalty_pool_ledger_fixed.clone()
        } else {
            deployment.royalty_pool.clone()
        },
        deposit: deployment.royalty_pool_deposit.clone(),
        redeem: deployment.royalty_pool_redeem.clone(),
        dao: deployment.royalty_pool_dao_contract.clone(),
        dao_request: deployment.royalty_pool_dao_request.clone(),
        withdraw: if ledger_fixed {
            deployment.royalty_pool_withdraw_contract_ledger_fixed.clone()
        } else {
            deployment.royalty_pool_withdraw_contract.clone()
        },
        withdraw_request: deployment.royalty_pool_withdraw_request.clone(),
    }
}

fn v2_bundle(deployment: &DeployedValidators) -> RoyaltyPoolScriptBundle {
    RoyaltyPoolScriptBundle {
        pool: deployment.royalty_pool_v2.clone(),
        deposit: deployment.royalty_pool_deposit_v2.clone(),
        redeem: deployment.royalty_pool_redeem_v2.clone(),
        dao: deployment.royalty_pool_dao_contract_v2.clone(),
        dao_request: deployment.royalty_pool_v2_dao_request.clone(),
        withdraw: deployment.royalty_pool_withdraw_contract_v2.clone(),
        withdraw_request: deployment.royalty_pool_v2_withdraw_request.clone(),
    }
}

fn erased<const TYP: u8>(validator: &DeployedValidator<TYP>) -> DeployedValidatorErased {
    validator.clone().erased()
}

fn execution_v1_bundle(deployment: &ProtocolDeployment, ledger_fixed: bool) -> RoyaltyPoolExecutionBundle {
    RoyaltyPoolExecutionBundle {
        pool: if ledger_fixed {
            erased(&deployment.royalty_pool_ledger_fixed)
        } else {
            erased(&deployment.royalty_pool)
        },
        deposit: erased(&deployment.royalty_pool_deposit),
        redeem: erased(&deployment.royalty_pool_redeem),
        dao: erased(&deployment.royalty_pool_dao),
        dao_request: erased(&deployment.royalty_pool_dao_request),
        withdraw: if ledger_fixed {
            erased(&deployment.royalty_pool_withdraw_ledger_fixed)
        } else {
            erased(&deployment.royalty_pool_withdraw)
        },
        withdraw_request: erased(&deployment.royalty_pool_royalty_withdraw_request),
    }
}

fn execution_v2_bundle(deployment: &ProtocolDeployment) -> RoyaltyPoolExecutionBundle {
    RoyaltyPoolExecutionBundle {
        pool: erased(&deployment.royalty_pool_v2),
        deposit: erased(&deployment.royalty_pool_deposit_v2),
        redeem: erased(&deployment.royalty_pool_redeem_v2),
        dao: erased(&deployment.royalty_pool_v2_dao),
        dao_request: erased(&deployment.royalty_pool_v2_dao_request),
        withdraw: erased(&deployment.royalty_pool_withdraw_v2),
        withdraw_request: erased(&deployment.royalty_pool_v2_royalty_withdraw_request),
    }
}

#[cfg(test)]
mod tests {
    use super::{RoyaltyPoolKind, RoyaltyPoolProfiles};
    use crate::deployment::DeployedValidators;
    use cml_crypto::ScriptHash;

    fn read(path: &str) -> DeployedValidators {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn single_mainnet_config_routes_protected_and_legacy_pool_hashes() {
        let deployment = read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../bloom-cardano-agent/resources/mainnet.deployment.json"
        ));
        let protected = deployment.protected_royalty_profile().unwrap();
        let profiles = RoyaltyPoolProfiles::with_protected(&deployment, &protected)
            .with_protected_fee_switch(&deployment);
        assert_eq!(profiles.resolve(&deployment.royalty_pool.hash).unwrap().kind, RoyaltyPoolKind::LegacyV1);
        assert_eq!(profiles.resolve(&protected.royalty_pool_ledger_fixed.hash).unwrap().kind, RoyaltyPoolKind::ProtectedV1);
        assert_eq!(profiles.resolve(&protected.royalty_pool_v2.hash).unwrap().kind, RoyaltyPoolKind::ProtectedV2);
        let fee_switch = deployment.protected_fee_switch.as_ref().unwrap();
        assert_eq!(profiles.resolve_fee_switch(&fee_switch.pool.hash), Some(super::ProtectedFeeSwitchKind::Regular));
        assert_eq!(profiles.resolve_fee_switch(&fee_switch.pool_bi_dir.hash), Some(super::ProtectedFeeSwitchKind::BiDir));
        assert_eq!(profiles.resolve_fee_switch(&deployment.const_fn_pool_fee_switch.hash), None);
    }

    #[test]
    fn legacy_and_protected_profiles_coexist_and_keep_their_reference_bundles() {
        let legacy = read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../bloom-cardano-agent/resources/preprod.deployment.json"
        ));
        let protected = read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../bloom-cardano-agent/resources/preprod-royalty-2026-10-05.deployment.json"
        ));
        let profiles = RoyaltyPoolProfiles::with_protected(&legacy, &protected);

        let legacy_v1 = profiles.resolve(&legacy.royalty_pool.hash).unwrap();
        assert_eq!(legacy_v1.kind, RoyaltyPoolKind::LegacyV1);
        assert_eq!(
            legacy_v1.scripts.pool.reference_utxo.tx_hash,
            legacy.royalty_pool.reference_utxo.tx_hash
        );

        let legacy_v2 = profiles.resolve(&legacy.royalty_pool_v2.hash).unwrap();
        assert_eq!(legacy_v2.kind, RoyaltyPoolKind::LegacyV2);
        assert_eq!(
            legacy_v2.scripts.withdraw.hash,
            legacy.royalty_pool_withdraw_contract_v2.hash
        );

        let protected_v1 = profiles
            .resolve(&protected.royalty_pool_ledger_fixed.hash)
            .unwrap();
        assert_eq!(protected_v1.kind, RoyaltyPoolKind::ProtectedV1);
        assert_eq!(
            protected_v1.scripts.dao.hash,
            protected.royalty_pool_dao_contract.hash
        );
        assert_eq!(
            protected_v1.scripts.withdraw.hash,
            protected.royalty_pool_withdraw_contract_ledger_fixed.hash
        );

        let protected_v2 = profiles.resolve(&protected.royalty_pool_v2.hash).unwrap();
        assert_eq!(protected_v2.kind, RoyaltyPoolKind::ProtectedV2);
        assert_eq!(
            protected_v2.scripts.deposit.hash,
            protected.royalty_pool_deposit_v2.hash
        );
        assert_eq!(
            protected_v2.scripts.redeem.hash,
            protected.royalty_pool_redeem_v2.hash
        );

        // Deposit/redeem and withdrawal order validators are shared between
        // deployments. Both protected DAO request validators have distinct
        // hashes because each one commits to its corresponding protected DAO
        // policy. For shared hashes, execution uses the pool's kind to select
        // the matching reference bundle.
        assert_eq!(
            profiles.resolve_deposit(&protected.royalty_pool_deposit.hash),
            Some(RoyaltyPoolKind::LegacyV1)
        );
        assert_eq!(
            profiles.resolve_redeem(&protected.royalty_pool_redeem_v2.hash),
            Some(RoyaltyPoolKind::LegacyV2)
        );
        assert_eq!(
            profiles.resolve_dao_request(&protected.royalty_pool_dao_request.hash),
            Some(RoyaltyPoolKind::ProtectedV1)
        );
        assert_eq!(
            profiles.resolve_dao_request(&protected.royalty_pool_v2_dao_request.hash),
            Some(RoyaltyPoolKind::ProtectedV2)
        );
        assert_eq!(
            profiles.resolve_withdraw_request(&protected.royalty_pool_withdraw_request.hash),
            Some(RoyaltyPoolKind::LegacyV1)
        );
        assert_eq!(
            profiles.resolve_withdraw_request(&protected.royalty_pool_v2_withdraw_request.hash),
            Some(RoyaltyPoolKind::LegacyV2)
        );

        assert!(profiles.resolve(&ScriptHash::from([0_u8; 28])).is_none());
    }
}
