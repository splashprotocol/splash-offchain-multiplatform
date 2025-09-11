use algebra_core::monoid::Monoid;
use bloom_offchain::execution_engine::liquidity_book::config::ExecutionConfig;
use bloom_offchain::execution_engine::liquidity_book::core::{
    ExecutionEvent, MakeInProgress, MatchmakingAttempt, MatchmakingRecipe, Next, TakeInProgress, Trans,
    UnsatisfiedFragment,
};
use bloom_offchain::execution_engine::liquidity_book::market_maker::{MakerBehavior, MarketMaker, SpotPrice};
use bloom_offchain::execution_engine::liquidity_book::market_taker::{MarketTaker, TakerBehaviour};
use bloom_offchain::execution_engine::liquidity_book::side::{OnSide, Side};
use bloom_offchain::execution_engine::liquidity_book::stashing_option::StashingOption;
use bloom_offchain::execution_engine::liquidity_book::state::{
    dummy_swap, try_optimized_swap, FillPreview, LiquidityBookSize,
};
use bloom_offchain::execution_engine::liquidity_book::types::AbsolutePrice;
use bloom_offchain::execution_engine::liquidity_book::{ExternalLBEvents, LBFeedback, LiquidityBook, TLB};
use either::Either;
use log::trace;
use spectrum_offchain::display::{display_option, display_tuple};
use spectrum_offchain::domain::{Has, Stable};
use spectrum_offchain::maker::Maker;
use std::collections::{HashMap, VecDeque};
use std::fmt::{Debug, Display};
use std::ops::AddAssign;

#[derive(Clone)]
struct FifoState<Taker: Stable, Maker: Stable> {
    queue: VecDeque<Taker::StableId>,
    takers: HashMap<Taker::StableId, Taker>,
    makers: HashMap<Maker::StableId, Maker>,
}

impl<Taker: Stable, Maker: Stable> FifoState<Taker, Maker> {
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            takers: HashMap::new(),
            makers: HashMap::new(),
        }
    }

    pub fn queue_size(&self) -> usize {
        self.queue.len()
    }

    pub fn takers_size(&self) -> usize {
        self.takers.len()
    }

    pub fn size(&self) -> LiquidityBookSize {
        LiquidityBookSize {
            num_active_takers: self.takers.len(),
            num_active_makers: self.makers.len(),
            num_idle_takers: 0,
            num_idle_makers: 0,
        }
    }

    pub fn append_taker(&mut self, taker: Taker) {
        self.queue.push_back(taker.stable_id());
        self.takers.insert(taker.stable_id(), taker);
    }

    pub fn prepend_taker(&mut self, taker: Taker) {
        self.queue.push_front(taker.stable_id());
        self.takers.insert(taker.stable_id(), taker);
    }

    pub fn remove_taker(&mut self, taker: Taker) {
        self.queue.retain(|id| *id != taker.stable_id());
        self.takers.remove(&taker.stable_id());
    }

    pub fn add_maker(&mut self, maker: Maker) {
        self.makers.insert(maker.stable_id(), maker);
    }

    pub fn remove_maker(&mut self, maker: Maker) {
        self.makers.remove(&maker.stable_id());
    }

    pub fn best_market_maker(&self) -> Option<&Maker>
    where
        Taker: MarketTaker,
        Maker: MarketMaker + Copy,
    {
        self.makers.values().max_by_key(|p| p.quality())
    }

    pub fn preselect_market_maker(
        &self,
        price: AbsolutePrice,
        demand: u64,
        side: Side,
        optimized: bool,
    ) -> Option<(Maker::StableId, FillPreview)>
    where
        Maker: MarketMaker,
    {
        let pools = self
            .makers
            .values()
            .filter(|pool| pool.is_active())
            .filter_map(|p| {
                if optimized {
                    try_optimized_swap(price, demand, side, p).or_else(|| dummy_swap(demand, side, p))
                } else {
                    dummy_swap(demand, side, p)
                }
            });
        match side {
            Side::Bid => pools.min_by_key(|(_, rp)| rp.price),
            Side::Ask => pools.max_by_key(|(_, rp)| rp.price),
        }
    }

    fn pick_maker_by_id(&mut self, pid: &Maker::StableId) -> Option<Maker> {
        self.makers.remove(pid)
    }

    pub fn pop_taker(&mut self) -> Option<Taker> {
        self.queue.pop_front().and_then(|id| self.takers.remove(&id))
    }
}

#[derive(Clone)]
pub struct Fifo<Taker: Stable, Maker: Stable, Pair, U> {
    state: FifoState<Taker, Maker>,
    backup: Option<FifoState<Taker, Maker>>,
    stash: Vec<Taker>,
    conf: ExecutionConfig<U>,
    pair: Pair,
}

impl<T, M, P, Ctx, U> Maker<P, Ctx> for Fifo<T, M, P, U>
where
    T: Stable,
    M: Stable,
    Ctx: Has<ExecutionConfig<U>>,
{
    fn make(key: P, ctx: &Ctx) -> Self {
        Self::new(ctx.select::<ExecutionConfig<U>>(), key)
    }
}

impl<Taker, Maker, P, U> LBFeedback<Taker, Maker> for Fifo<Taker, Maker, P, U>
where
    Taker: MarketTaker + Stable + Ord + Copy,
    Maker: MarketMaker + Stable + Copy,
{
    fn on_recipe_succeeded(&mut self) {
        self.backup = None;
    }

    fn on_recipe_failed(&mut self) {
        self.rollback(StashingOption::Unstash)
    }
}

impl<Taker: Stable, Maker: Stable, P, U> Fifo<Taker, Maker, P, U> {
    pub fn new(conf: ExecutionConfig<U>, pair: P) -> Self {
        Self {
            state: FifoState::new(),
            backup: None,
            stash: vec![],
            conf,
            pair,
        }
    }

    fn spot_price(&self) -> Option<SpotPrice>
    where
        Taker: MarketTaker,
        Maker: MarketMaker + Copy,
    {
        self.state.best_market_maker().map(|mm| mm.static_price())
    }

    fn backup(&mut self)
    where
        Taker: Clone,
        Maker: Clone,
    {
        self.backup.replace(self.state.clone());
    }

    fn rollback(&mut self, stashing_opt: StashingOption<Taker>)
    where
        Taker: Copy,
    {
        match stashing_opt {
            StashingOption::Stash(mut to_stash) => {
                if let Some(mut backup) = self.backup.take() {
                    for taker in &to_stash {
                        backup.remove_taker(*taker);
                    }
                    self.state = backup;
                    self.stash.append(&mut to_stash);
                };
            }
            StashingOption::Unstash => {
                if let Some(mut backup) = self.backup.take() {
                    for taker in self.stash.drain(..) {
                        backup.append_taker(taker);
                    }
                    self.state = backup;
                };
            }
        }
    }
}

impl<Taker, Maker, P, U> Fifo<Taker, Maker, P, U>
where
    Taker: MarketTaker<U = U> + Stable + Ord + Copy + Display,
    Maker: MarketMaker + Stable + Copy,
    U: PartialOrd,
{
    fn on_take<Any>(&mut self, tx: Next<Taker, Any>) {
        if let Next::Succ(next) = tx {
            self.state.prepend_taker(next);
        }
    }

    fn on_make<Any>(&mut self, tx: Next<Maker, Any>) {
        if let Next::Succ(next) = tx {
            self.state.add_maker(next);
        }
    }
}

impl<Taker, Maker, P, U> LiquidityBook<Taker, Maker, Vec<ExecutionEvent>> for Fifo<Taker, Maker, P, U>
where
    Taker: Stable + MarketTaker<U = U> + TakerBehaviour + Ord + Copy + Display + Debug,
    <Taker as TakerBehaviour>::Mode: UnsatisfiedFragment<Taker>,
    Maker: Stable + MarketMaker<U = U> + MakerBehavior + Copy + Display,
    U: Monoid + AddAssign + PartialOrd + Copy,
    P: Display,
{
    fn attempt(&mut self) -> (Option<MatchmakingRecipe<Taker, Maker>>, Vec<ExecutionEvent>) {
        let mut optimized_matchmaking = true;
        loop {
            trace!(
                "{} Attempting to matchmake (optimized={})",
                self.pair,
                optimized_matchmaking
            );
            let mut batch: MatchmakingAttempt<Taker, Maker, U> = MatchmakingAttempt::empty();
            let mut events = vec![];
            let pre_attempt_size = self.state.size();
            events.push(ExecutionEvent::LiquidityBookSizePreAttempt(pre_attempt_size));
            let mut max_attempts = self.state.queue_size();
            self.backup();
            while batch.execution_units_consumed() < self.conf.execution_cap.soft && batch.num_takes() < 17 {
                if let Some(spot_price) = self.spot_price() {
                    events.push(ExecutionEvent::SpotPrice(spot_price));
                    trace!("{} spot_price: {}", self.pair, spot_price,);
                    if let Some(target_taker) = self.state.pop_taker() {
                        trace!("Selected taker: {}", target_taker);
                        let target_side = target_taker.side();
                        let target_price = target_side.wrap(target_taker.price());
                        let maybe_price_maker = self.state.preselect_market_maker(
                            target_taker.price(),
                            target_taker.input(),
                            target_side,
                            optimized_matchmaking,
                        );
                        trace!(
                            "{} P_target: {}, P_amm: {}",
                            self.pair,
                            target_price.unwrap(),
                            display_option(&maybe_price_maker.map(|(id, fp)| display_tuple((id, fp.price))))
                        );
                        match maybe_price_maker {
                            Some((maker_sid, FillPreview { price, input }))
                                if target_price.overlaps(price) =>
                            {
                                if let Some(maker) = self.state.pick_maker_by_id(&maker_sid) {
                                    trace!("Taker {} matched with {}", target_taker, maker);
                                    let (take, make) =
                                        execute_with_maker(target_taker, maker, target_side.wrap(input));
                                    batch.add_make(make);
                                    batch.add_take(take);
                                    self.on_take(take.result);
                                    self.on_make(make.result);
                                    continue;
                                }
                            }
                            _ => {
                                trace!("Failed to match taker {}", target_taker);
                                self.state.append_taker(target_taker);
                            }
                        }
                    } else {
                        break;
                    }
                } else {
                    events.push(ExecutionEvent::SpotPriceNotAvailable);
                    trace!("{} No liquidity source is available", self.pair);
                }
                if max_attempts > 0 {
                    max_attempts -= 1;
                    continue;
                }
                break;
            }
            trace!("{} Raw batch: {}", self.pair, batch);
            let post_attempt_size = self.state.size();
            events.push(ExecutionEvent::LiquidityBookSizePostAttempt(post_attempt_size));
            match MatchmakingRecipe::try_from(batch, self.conf) {
                Ok(ex_recipe) => {
                    trace!("{} Successfully formed a batch {}", self.pair, ex_recipe);
                    return (Some(ex_recipe), events);
                }
                Err(None) => {
                    trace!("{} Matchmaking attempt failed in fifo", self.pair);
                    self.rollback(StashingOption::Unstash);
                }
                Err(Some(Either::Left(unsatisfied_takers))) => {
                    trace!(
                        "{} Matchmaking attempt failed due to taker limits, retrying",
                        self.pair
                    );
                    self.rollback(StashingOption::Stash(unsatisfied_takers));
                    continue;
                }
                Err(Some(Either::Right(_downgrade_required))) => {
                    trace!(
                        "{} Matchmaking attempt failed due to high execution complexity, retrying",
                        self.pair
                    );
                    self.rollback(StashingOption::Stash(vec![]));
                    optimized_matchmaking = false;
                    continue;
                }
            }
            return (None, events);
        }
    }
}

fn execute_with_maker<Taker, Maker>(
    target_taker: Taker,
    maker: Maker,
    chunk_size: OnSide<u64>,
) -> (TakeInProgress<Taker>, MakeInProgress<Maker>)
where
    Taker: MarketTaker + TakerBehaviour + Copy,
    Maker: MarketMaker + MakerBehavior + Copy,
{
    let next_maker = maker.swap(chunk_size);
    let make = Trans::new(maker, next_maker);
    let trade_output = make.loss().map(|val| val.unwrap()).unwrap_or(0);
    let next_taker = target_taker.with_applied_trade(chunk_size.unwrap(), trade_output);
    let take = Trans::new(target_taker, next_taker);
    (take, make)
}

impl<Taker: Stable, Maker: Stable, Pair, U> ExternalLBEvents<Taker, Maker> for Fifo<Taker, Maker, Pair, U> {
    fn advance_clocks(&mut self, _: u64) {}

    fn update_taker(&mut self, tk: Taker) {
        self.state.append_taker(tk)
    }

    fn remove_taker(&mut self, tk: Taker) {
        self.state.remove_taker(tk)
    }

    fn update_maker(&mut self, mk: Maker) {
        self.state.add_maker(mk)
    }

    fn remove_maker(&mut self, mk: Maker) {
        self.state.remove_maker(mk)
    }
}

#[cfg(test)]
mod tests {
    use crate::fifo::{execute_with_maker, Fifo};
    use crate::snek_protocol_deployment::{SnekDeployedValidators, SnekProtocolScriptHashes};
    use bloom_offchain::execution_engine::liquidity_book::config::{ExecutionCap, ExecutionConfig};
    use bloom_offchain::execution_engine::liquidity_book::market_maker::MarketMaker;
    use bloom_offchain::execution_engine::liquidity_book::market_taker::MarketTaker;
    use bloom_offchain::execution_engine::liquidity_book::side::OnSide;
    use bloom_offchain::execution_engine::liquidity_book::{ExternalLBEvents, LiquidityBook};
    use bloom_offchain_cardano::orders::adhoc::{AdhocFeeStructure, AdhocOrder};
    use bloom_offchain_cardano::orders::instant::{InstantOrder, InstantOrderValidation};
    use bounded_integer::BoundedU64;
    use cml_chain::auxdata::Metadata;
    use cml_chain::transaction::TransactionOutput;
    use cml_core::serialization::Deserialize;
    use cml_crypto::{Ed25519KeyHash, TransactionHash};
    use spectrum_cardano_lib::ex_units::ExUnits;
    use spectrum_cardano_lib::{OutputRef, TaggedAmount, Token};
    use spectrum_offchain::data::small_vec::SmallVec;
    use spectrum_offchain::domain::Has;
    use spectrum_offchain::ledger::TryFromLedger;
    use spectrum_offchain_cardano::creds::OperatorCred;
    use spectrum_offchain_cardano::data::pair::PairId;
    use spectrum_offchain_cardano::data::pool::PoolValidation;
    use spectrum_offchain_cardano::data::quadratic_pool::QuadraticPool;
    use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
    use spectrum_offchain_cardano::deployment::ProtocolValidator::{
        DegenQuadraticPoolV1, DegenQuadraticPoolV1T2T, InstantOrderV1,
    };
    use spectrum_offchain_cardano::handler_context::{
        AddedPaymentDestinations, AllowedAdditionalPaymentDestinations, ConsumedIdentifiers, ConsumedInputs,
        Mints, ProducedIdentifiers,
    };
    use type_equalities::IsEqual;

    const RAW_INSTANT_ORDER: &str = "a300583911d9143ac63473b17a215d1b7484dfb6ac6b4a0005beb0e26a6ca02c96d817a67d082624525f7ba3f4211b0557c3587e88d2809725d6f80902011a00a1be40028201d81858e7d8798a4101d8799fd8799f581c3f4165e2ea0a4dc6f7bcbbd23f824187c80e5490902e097fc3049c72ffd8799fd8799fd8799f581cd817a67d082624525f7ba3f4211b0557c3587e88d2809725d6f80902ffffffffd879824040d87982581cba62b8967f992e24449511bac5edbd13078f2202d44cdf5dc0ef29cf4f54686520536f7274696e6720486174d879821a0029e7f71a007a12001a0010c8e01a0016e360581cedbf33f5d6e083970648e39175c49ec1c093df76b6e6a0f1473e47761b000001992a6604a8581c3f4165e2ea0a4dc6f7bcbbd23f824187c80e5490902e097fc3049c72";
    const RAW_POOL_T2T: &str = "a300583931905ab869961b094f1b8197278cfe15b45cbe49fa8f32c6b014f85a2db2f6abf60ccde92eae1a2f4fdf65f2eaf6208d872c6f0e597cc10b0701821a1731ec54a2581c63f947b8d9535bc4e4ce6919e3dc056547e8d30ada12f29aa5f826b8a158200102f70f384481964784240c06f025d0d53e027ba7817e1910f09b25cae2f5b501581cba62b8967f992e24449511bac5edbd13078f2202d44cdf5dc0ef29cfa14f54686520536f7274696e67204861741a3450142d028201d81858edd87989d87982581c63f947b8d9535bc4e4ce6919e3dc056547e8d30ada12f29aa5f826b858200102f70f384481964784240c06f025d0d53e027ba7817e1910f09b25cae2f5b5d879824040d87982581cba62b8967f992e24449511bac5edbd13078f2202d44cdf5dc0ef29cf4f54686520536f7274696e67204861741b0000001c871b063f1a0026d61e581cedbf33f5d6e083970648e39175c49ec1c093df76b6e6a0f1473e47761b000000043c4abc40581c8807fbe6e36b1c35ad6f36f0993e2fc67ab6f2db06041cfa3a53c04a581c30c1003aa7dec834e0d0a78db547ba8840e58060725dbfae352f0d64";

    struct Context {
        oref: OutputRef,
        instant_order: DeployedScriptInfo<{ InstantOrderV1 as u8 }>,
        degen_pool_n2t: DeployedScriptInfo<{ DegenQuadraticPoolV1 as u8 }>,
        degen_pool_t2t: DeployedScriptInfo<{ DegenQuadraticPoolV1T2T as u8 }>,
        cred: OperatorCred,
        consumed_inputs: ConsumedInputs,
        consumed_identifiers: ConsumedIdentifiers<Token>,
        produced_identifiers: ProducedIdentifiers<Token>,
    }

    impl Has<OutputRef> for Context {
        fn select<U: IsEqual<OutputRef>>(&self) -> OutputRef {
            self.oref
        }
    }

    impl Has<ConsumedIdentifiers<Token>> for Context {
        fn select<U: IsEqual<ConsumedIdentifiers<Token>>>(&self) -> ConsumedIdentifiers<Token> {
            self.consumed_identifiers
        }
    }

    impl Has<ProducedIdentifiers<Token>> for Context {
        fn select<U: IsEqual<ProducedIdentifiers<Token>>>(&self) -> ProducedIdentifiers<Token> {
            self.produced_identifiers
        }
    }

    impl Has<InstantOrderValidation> for Context {
        fn select<U: IsEqual<InstantOrderValidation>>(&self) -> InstantOrderValidation {
            InstantOrderValidation {
                min_lovelace: 1500000,
                min_fee_lovelace: 500000,
                min_execution_budget_lovelace: 500000,
            }
        }
    }

    impl Has<ConsumedInputs> for Context {
        fn select<U: IsEqual<ConsumedInputs>>(&self) -> ConsumedInputs {
            self.consumed_inputs
        }
    }

    impl Has<OperatorCred> for Context {
        fn select<U: IsEqual<OperatorCred>>(&self) -> OperatorCred {
            self.cred
        }
    }

    impl Has<DeployedScriptInfo<{ InstantOrderV1 as u8 }>> for Context {
        fn select<U: IsEqual<DeployedScriptInfo<{ InstantOrderV1 as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ InstantOrderV1 as u8 }> {
            self.instant_order
        }
    }

    impl Has<DeployedScriptInfo<{ DegenQuadraticPoolV1 as u8 }>> for Context {
        fn select<U: IsEqual<DeployedScriptInfo<{ DegenQuadraticPoolV1 as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ DegenQuadraticPoolV1 as u8 }> {
            self.degen_pool_n2t
        }
    }

    impl Has<DeployedScriptInfo<{ DegenQuadraticPoolV1T2T as u8 }>> for Context {
        fn select<U: IsEqual<DeployedScriptInfo<{ DegenQuadraticPoolV1T2T as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ DegenQuadraticPoolV1T2T as u8 }> {
            self.degen_pool_t2t
        }
    }

    impl Has<PoolValidation> for Context {
        fn select<U: IsEqual<PoolValidation>>(&self) -> PoolValidation {
            PoolValidation {
                min_n2t_lovelace: 1_000_000,
                min_t2t_lovelace: 1_000_000,
            }
        }
    }

    impl Has<Option<Mints>> for Context {
        fn select<U: IsEqual<Option<Mints>>>(&self) -> Option<Mints> {
            None
        }
    }

    impl Has<Option<Metadata>> for Context {
        fn select<U: IsEqual<Option<Metadata>>>(&self) -> Option<Metadata> {
            None
        }
    }

    impl Has<AddedPaymentDestinations> for Context {
        fn select<U: IsEqual<AddedPaymentDestinations>>(&self) -> AddedPaymentDestinations {
            AddedPaymentDestinations(SmallVec::default())
        }
    }

    impl Has<AllowedAdditionalPaymentDestinations> for Context {
        fn select<U: IsEqual<AllowedAdditionalPaymentDestinations>>(
            &self,
        ) -> AllowedAdditionalPaymentDestinations {
            AllowedAdditionalPaymentDestinations(SmallVec::default())
        }
    }

    impl Has<AdhocFeeStructure> for Context {
        fn select<U: IsEqual<AdhocFeeStructure>>(&self) -> AdhocFeeStructure {
            AdhocFeeStructure {
                relative_fee_percent: BoundedU64::new(1).unwrap(),
            }
        }
    }

    #[test]
    fn partial_fill_order_from_pool() {
        println!("123123");

        const TX: &str = "b72f29953347030c5051bdba801d2c5dfdebc9574dfb28e139d0ef131033aee6";
        const IX: u64 = 0;
        let oref = OutputRef::new(TransactionHash::from_hex(TX).unwrap(), IX);
        const TX_BC: &str = "17bac1341d063c58193ef501048650ec9d9e5a85384219c828fbf54a88f30197";
        const IX_BC: u64 = 1;
        let oref_bc = OutputRef::new(TransactionHash::from_hex(TX_BC).unwrap(), IX_BC);
        let raw_deployment = std::fs::read_to_string("/Users/aleksandr/IdeaProjects/spectrum-offchain-multiplatform/snek-cardano-agent/resources/preprod.deployment.json").expect("Cannot load deployment file");
        let deployment: SnekDeployedValidators =
            serde_json::from_str(&raw_deployment).expect("Invalid deployment file");
        let scripts = SnekProtocolScriptHashes::from(&deployment);
        let ctx = Context {
            oref,
            instant_order: scripts.instant_order,
            degen_pool_n2t: scripts.degen_fn_pool_v1,
            degen_pool_t2t: scripts.degen_fn_pool_v1_t2t,
            cred: OperatorCred(
                Ed25519KeyHash::from_hex("15772e8f1fdcf12d59636caf42522b7d6249ccb223253eb7e9b6d509").unwrap(),
            ),
            consumed_inputs: SmallVec::new(vec![oref_bc].into_iter()).into(),
            consumed_identifiers: SmallVec::new(vec![].into_iter()).into(),
            produced_identifiers: Default::default(),
        };

        let bearer = TransactionOutput::from_cbor_bytes(&*hex::decode(RAW_INSTANT_ORDER).unwrap()).unwrap();
        let ask_fr = AdhocOrder::try_from_ledger(&bearer, &ctx).unwrap();

        let raw_pool = TransactionOutput::from_cbor_bytes(&*hex::decode(RAW_POOL_T2T).unwrap()).unwrap();
        let pool = QuadraticPool::try_from_ledger(&raw_pool, &ctx).unwrap();

        let mut fifo = Fifo::new(
            ExecutionConfig {
                execution_cap: ExecutionCap {
                    soft: ExUnits {
                        mem: 5000000,
                        steps: 4000000000,
                    },
                    hard: ExUnits {
                        mem: 14000000,
                        steps: 10000000000,
                    },
                },
                o2o_allowed: false,
                base_step_budget: 600000.into(),
            },
            PairId::canonical(ask_fr.0.input_asset, ask_fr.0.output_asset),
        );

        fifo.update_maker(pool);

        fifo.update_taker(ask_fr);

        let res = fifo.attempt();

        let real_price_in_pool = pool.real_price(OnSide::Ask(ask_fr.input()));

        let (t, m) = execute_with_maker(ask_fr, pool, OnSide::Bid(ask_fr.input()));

        println!("t: {}", t);
        println!("m: {}", m);

        assert_eq!(m.gain().unwrap().unwrap(), t.removed_input());
        assert_eq!(m.loss().unwrap().unwrap(), t.added_output());

        assert_eq!(1, 2);
    }

    #[test]
    fn test_unsatisfied_fragments_check() {
        use spectrum_offchain::domain::Stable;

        // Setup context similar to partial_fill_order_from_pool test
        const TX: &str = "b72f29953347030c5051bdba801d2c5dfdebc9574dfb28e139d0ef131033aee6";
        const IX: u64 = 0;
        let oref = OutputRef::new(TransactionHash::from_hex(TX).unwrap(), IX);
        const TX_BC: &str = "17bac1341d063c58193ef501048650ec9d9e5a85384219c828fbf54a88f30197";
        const IX_BC: u64 = 1;
        let oref_bc = OutputRef::new(TransactionHash::from_hex(TX_BC).unwrap(), IX_BC);
        let raw_deployment = std::fs::read_to_string("/Users/aleksandr/IdeaProjects/spectrum-offchain-multiplatform/snek-cardano-agent/resources/preprod.deployment.json").expect("Cannot load deployment file");
        let deployment: SnekDeployedValidators =
            serde_json::from_str(&raw_deployment).expect("Invalid deployment file");
        let scripts = SnekProtocolScriptHashes::from(&deployment);
        let ctx = Context {
            oref,
            instant_order: scripts.instant_order,
            degen_pool_n2t: scripts.degen_fn_pool_v1,
            degen_pool_t2t: scripts.degen_fn_pool_v1_t2t,
            cred: OperatorCred(
                Ed25519KeyHash::from_hex("edbf33f5d6e083970648e39175c49ec1c093df76b6e6a0f1473e4776").unwrap(),
            ),
            consumed_inputs: SmallVec::new(vec![oref_bc].into_iter()).into(),
            consumed_identifiers: SmallVec::new(vec![].into_iter()).into(),
            produced_identifiers: Default::default(),
        };

        // Get the unsatisfied order (RAW_INSTANT_ORDER)
        let bearer = TransactionOutput::from_cbor_bytes(&*hex::decode(RAW_INSTANT_ORDER).unwrap()).unwrap();
        let unsatisfied_order = AdhocOrder::try_from_ledger(&bearer, &ctx).unwrap();
        let instant_order = InstantOrder::try_from_ledger(&bearer, &ctx).unwrap();

        // Get the pool (RAW_POOL_T2T)
        let raw_pool = TransactionOutput::from_cbor_bytes(&*hex::decode(RAW_POOL_T2T).unwrap()).unwrap();
        let mut parsed_pool = QuadraticPool::try_from_ledger(&raw_pool, &ctx).unwrap();
        parsed_pool.reserves_x = TaggedAmount::new(141988115);
        parsed_pool.reserves_y = TaggedAmount::new(947688745);
        let pool = parsed_pool.clone();

        // Create a correct order by cloning the unsatisfied order and modifying it
        // We'll create a "correct" order by modifying it to ensure it will be satisfied
        let correct_order = AdhocOrder::new(
            InstantOrder {
                beacon: Token::from([0; 60]),
                ..instant_order
            },
            0,
        );

        // Create FIFO with the pool
        let mut fifo = Fifo::new(
            ExecutionConfig {
                execution_cap: ExecutionCap {
                    soft: ExUnits {
                        mem: 5000000,
                        steps: 4000000000,
                    },
                    hard: ExUnits {
                        mem: 14000000,
                        steps: 10000000000,
                    },
                },
                o2o_allowed: false,
                base_step_budget: 600000.into(),
            },
            PairId::canonical(unsatisfied_order.0.input_asset, unsatisfied_order.0.output_asset),
        );

        // Add the pool as maker
        fifo.update_maker(pool);

        // Add both orders as takers
        fifo.update_taker(unsatisfied_order.clone());
        fifo.update_taker(correct_order.clone());

        // Call attempt() on the FIFO
        let (recipe, _) = fifo.attempt();

        // Verify that we got a recipe
        assert!(recipe.is_some(), "Attempt should produce a recipe");

        let recipe = recipe.unwrap();

        // Verify that the order in the queue is the unsatisfied order
        let stash_len = fifo.stash.len();
        let stash_order_id = fifo.stash.pop().unwrap().0;
        assert_eq!(
            stash_len, 1,
            "The FIFO should have exactly one order in the stash"
        );
        assert_eq!(
            stash_order_id.stable_id(),
            unsatisfied_order.stable_id(),
            "The order in the queue should be the unsatisfied order"
        );
    }
}
