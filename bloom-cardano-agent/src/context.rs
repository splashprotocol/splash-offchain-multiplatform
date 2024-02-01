use type_equalities::IsEqual;

use bloom_offchain::execution_engine::liquidity_book::ExecutionCap;
use bloom_offchain::execution_engine::types::Time;
use bloom_offchain_cardano::creds::RewardAddress;
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_offchain::data::Has;
use spectrum_offchain_cardano::data::pool::CFMMPoolRefScriptOutput;
use spectrum_offchain_cardano::data::ref_scripts::ReferenceOutputs;

#[derive(Debug, Clone)]
pub struct ExecutionContext {
    pub time: Time,
    pub execution_cap: ExecutionCap,
    pub refs: ReferenceOutputs,
    pub collateral: Collateral,
    pub reward_addr: RewardAddress,
}

impl Has<Time> for ExecutionContext {
    fn get_labeled<U: IsEqual<Time>>(&self) -> Time {
        self.time
    }
}

impl Has<ExecutionCap> for ExecutionContext {
    fn get_labeled<U: IsEqual<ExecutionCap>>(&self) -> ExecutionCap {
        self.execution_cap
    }
}

impl Has<Collateral> for ExecutionContext {
    fn get_labeled<U: IsEqual<Collateral>>(&self) -> Collateral {
        self.collateral.clone()
    }
}

impl Has<RewardAddress> for ExecutionContext {
    fn get_labeled<U: IsEqual<RewardAddress>>(&self) -> RewardAddress {
        self.reward_addr.clone()
    }
}

impl Has<CFMMPoolRefScriptOutput<1>> for ExecutionContext {
    fn get_labeled<U: IsEqual<CFMMPoolRefScriptOutput<1>>>(&self) -> CFMMPoolRefScriptOutput<1> {
        CFMMPoolRefScriptOutput(self.refs.pool_v1.clone())
    }
}

impl Has<CFMMPoolRefScriptOutput<2>> for ExecutionContext {
    fn get_labeled<U: IsEqual<CFMMPoolRefScriptOutput<2>>>(&self) -> CFMMPoolRefScriptOutput<2> {
        CFMMPoolRefScriptOutput(self.refs.pool_v2.clone())
    }
}

#[cfg(test)]
mod test {
    use cml_crypto::{blake2b256, PrivateKey, RawBytesEncoding};

    #[test]
    fn make_sig() {
        let preimage_hex = "F686D9AA9F784C7E3DC0027C57505444C0383776D159733776226ED200";
        let message_preimage = hex::decode(preimage_hex).unwrap();
        let message = blake2b256(&*message_preimage);
        let sk = PrivateKey::generate_ed25519();
        let pk = sk.to_public();
        let sig = sk.sign(&message);
        println!(
            "pk: {}, sig: {}, preimage: {}, msg: {}",
            pk.to_raw_hex(),
            sig.to_hex(),
            hex::encode(message_preimage),
            hex::encode(message)
        )
    }
}
