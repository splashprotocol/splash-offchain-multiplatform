use algebra_core::monoid::Monoid;
use bloom_offchain::execution_engine::liquidity_book::config::ExecutionConfig;
use bloom_offchain::execution_engine::liquidity_book::core::{
    ExecutionMeta, MakeInProgress, MatchmakingAttempt, MatchmakingRecipe, Next, TakeInProgress, Trans,
};
use bloom_offchain::execution_engine::liquidity_book::market_maker::{MakerBehavior, MarketMaker, SpotPrice};
use bloom_offchain::execution_engine::liquidity_book::market_taker::{MarketTaker, TakerBehaviour};
use bloom_offchain::execution_engine::liquidity_book::side::{OnSide, Side};
use bloom_offchain::execution_engine::liquidity_book::stashing_option::StashingOption;
use bloom_offchain::execution_engine::liquidity_book::state::{dummy_swap, try_optimized_swap, FillPreview};
use bloom_offchain::execution_engine::liquidity_book::types::AbsolutePrice;
use bloom_offchain::execution_engine::liquidity_book::{ExternalLBEvents, LBFeedback, LiquidityBook, TLB};
use either::Either;
use log::trace;
use spectrum_offchain::display::{display_option, display_tuple};
use spectrum_offchain::domain::{Has, Stable};
use spectrum_offchain::maker::Maker;
use std::collections::{HashMap, VecDeque};
use std::fmt::Display;
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

impl<Taker, Maker, P, U> LiquidityBook<Taker, Maker, ExecutionMeta> for Fifo<Taker, Maker, P, U>
where
    Taker: Stable + MarketTaker<U = U> + TakerBehaviour + Ord + Copy + Display,
    Maker: Stable + MarketMaker<U = U> + MakerBehavior + Copy + Display,
    U: Monoid + AddAssign + PartialOrd + Copy,
    P: Display,
{
    fn attempt(&mut self) -> Option<(MatchmakingRecipe<Taker, Maker>, ExecutionMeta)> {
        let mut optimized_matchmaking = true;
        loop {
            trace!(
                "{} Attempting to matchmake (optimized={})",
                self.pair,
                optimized_matchmaking
            );
            let mut batch: MatchmakingAttempt<Taker, Maker, U> = MatchmakingAttempt::empty();
            let mut meta = ExecutionMeta::empty();
            let mut max_attempts = self.state.queue_size();
            self.backup();
            while batch.execution_units_consumed() < self.conf.execution_cap.soft && batch.num_takes() < 17 {
                if let Some(spot_price) = self.spot_price() {
                    meta.add_price_point(spot_price);
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
                    trace!("{} No liquidity source is available", self.pair);
                }
                if max_attempts > 0 {
                    max_attempts -= 1;
                    continue;
                }
                break;
            }
            trace!("{} Raw batch: {}", self.pair, batch);
            match MatchmakingRecipe::try_from(batch, self.conf) {
                Ok(ex_recipe) => {
                    trace!("{} Successfully formed a batch {}", self.pair, ex_recipe);
                    return Some((ex_recipe, meta));
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
            return None;
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
    use bloom_offchain_cardano::orders::limit::LimitOrderValidation;
    use bounded_integer::BoundedU64;
    use cml_chain::auxdata::Metadata;
    use cml_chain::transaction::TransactionOutput;
    use cml_core::serialization::Deserialize;
    use cml_crypto::{Ed25519KeyHash, TransactionHash};
    use spectrum_cardano_lib::ex_units::ExUnits;
    use spectrum_cardano_lib::{OutputRef, Token};
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

    const RAW_INSTANT_ORDER: &str = "a300581d709e94e848482ffb2befe7fe6c2f5171c99086b53b3f73f535ff61ca0901821a002c4020a1581c77cb34f72da105bd0cab41c2a10e2fa2fe97a181e6771a62d0c9673ea14974657374546f6b656e1a05f5e100028201d81859011ad8799f4101d8799fd8799f581caf31ce038e8a8b297546db1a86b3973c32e49e191b3c76626046ed93ffd8799fd8799fd8799f581c1fc3c30bb2966c801399aa685012979d1f5048da659407e8371b8126ffffffffd8799f581c77cb34f72da105bd0cab41c2a10e2fa2fe97a181e6771a62d0c9673e4974657374546f6b656eff1a05f5e1001a000927c0d8799f581c093c6231a315f25d01e0388e42facc5617c315881fe46bc5d37d843145746f6b656effd8799f0001ff1a0007a120581c15772e8f1fdcf12d59636caf42522b7d6249ccb223253eb7e9b6d50900581caf31ce038e8a8b297546db1a86b3973c32e49e191b3c76626046ed93581c9b4baaee265f968742128891756ab9a5351b603d20043d8f4999164eff";
    const RAW_POOL_T2T: &str = "a300583930c876c435e1de1bd93ac71f0e9f956a844cd72493514d2740221bfea6b2f6abf60ccde92eae1a2f4fdf65f2eaf6208d872c6f0e597cc10b0701821a05f5e100a3581c093c6231a315f25d01e0388e42facc5617c315881fe46bc5d37d8431a145746f6b656e1a388d5e09581c63f947b8d9535bc4e4ce6919e3dc056547e8d30ada12f29aa5f826b8a1582027f988222b2a73069db8b1503ea79dfd2f9e217f6f8cafaf71be2db64b11165501581c77cb34f72da105bd0cab41c2a10e2fa2fe97a181e6771a62d0c9673ea14974657374546f6b656e1a0bebc200028201d818590130d8799fd8799f581c63f947b8d9535bc4e4ce6919e3dc056547e8d30ada12f29aa5f826b8582027f988222b2a73069db8b1503ea79dfd2f9e217f6f8cafaf71be2db64b111655ffd8799f581c77cb34f72da105bd0cab41c2a10e2fa2fe97a181e6771a62d0c9673e4974657374546f6b656effd8799f581c093c6231a315f25d01e0388e42facc5617c315881fe46bc5d37d843145746f6b656eff1b0000001efc22eee61a00393870581c15772e8f1fdcf12d59636caf42522b7d6249ccb223253eb7e9b6d5091b00000004af5c9bf91a001e8482581c78dab68b25456933968fd3f6566a2294f5254000d969d954b61366a6581c9a71deef6cf71cda44449e2f568bc21efc2dca1083447d65df51f545581cfd9c70b031d7fc94d9c2cf80053e81172e775045eeb74c8af1d78baeff";

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

    impl Has<LimitOrderValidation> for Context {
        fn select<U: IsEqual<LimitOrderValidation>>(&self) -> LimitOrderValidation {
            LimitOrderValidation {
                min_cost_per_ex_step: 0,
                min_fee_lovelace: 0,
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
}
