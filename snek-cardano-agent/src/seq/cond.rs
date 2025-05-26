use crate::entity::EvolvingCardanoEntity;
use bloom_offchain::execution_engine::liquidity_book::market_taker::MarketTaker;
use cml_core::Slot;
use either::Either;
use spectrum_offchain::data::ior::Ior;
use spectrum_offchain::domain::event::{Channel, Transition};

#[derive(Debug, Copy, Clone)]
pub struct Id<const I: u8>;

pub trait ConditionalValidation<const I: u8, Cx> {
    fn cond(&self, id: Id<I>) -> bool;
    fn is_valid(&self, id: Id<I>, cx: Cx) -> bool;
}

impl<const I: u8, T: ConditionalValidation<I, Cx>, Cx: Copy> ConditionalValidation<I, Cx> for Ior<T, T> {
    fn cond(&self, id: Id<I>) -> bool {
        match self {
            Ior::Left(left) => left.cond(id),
            Ior::Right(right) => right.cond(id),
            Ior::Both(left, right) => left.cond(id) || right.cond(id),
        }
    }

    fn is_valid(&self, id: Id<I>, cx: Cx) -> bool {
        match self {
            Ior::Left(left) => left.is_valid(id, cx),
            Ior::Right(right) => right.is_valid(id, cx),
            Ior::Both(left, right) => left.is_valid(id, cx) || right.is_valid(id, cx),
        }
    }
}

impl<const I: u8, T: ConditionalValidation<I, Cx>, Cx: Copy> ConditionalValidation<I, Cx> for Transition<T> {
    fn cond(&self, id: Id<I>) -> bool {
        match self {
            Transition::Forward(ior) => ior.cond(id),
            Transition::Backward(ior) => ior.cond(id),
        }
    }

    fn is_valid(&self, id: Id<I>, cx: Cx) -> bool {
        match self {
            Transition::Forward(ior) => ior.is_valid(id, cx),
            Transition::Backward(ior) => ior.is_valid(id, cx),
        }
    }
}

impl<const I: u8, T: ConditionalValidation<I, Cx>, C, Cx> ConditionalValidation<I, Cx> for Channel<T, C> {
    fn cond(&self, id: Id<I>) -> bool {
        match self {
            Channel::Ledger(confirmed, _) => confirmed.0.cond(id),
            Channel::Mempool(unconfirmed) => unconfirmed.0.cond(id),
            Channel::LocalTxSubmit(predicted) => predicted.0.cond(id),
        }
    }

    fn is_valid(&self, id: Id<I>, cx: Cx) -> bool {
        match self {
            Channel::Ledger(confirmed, _) => confirmed.0.is_valid(id, cx),
            Channel::Mempool(unconfirmed) => unconfirmed.0.is_valid(id, cx),
            Channel::LocalTxSubmit(predicted) => predicted.0.is_valid(id, cx),
        }
    }
}

#[repr(u8)]
pub enum Validations {
    HypedLaunch,
    CancellationLock,
}

const HYPED_LAUNCH_CAP_LOVELACE: u64 = 50_000_000;

impl<Cx> ConditionalValidation<{ Validations::HypedLaunch as u8 }, Cx> for EvolvingCardanoEntity {
    fn cond(&self, _: Id<{ Validations::HypedLaunch as u8 }>) -> bool {
        match self.0 .0 {
            Either::Left(_) => false,
            Either::Right(p) => p.entity.capped,
        }
    }

    fn is_valid(&self, _: Id<{ Validations::HypedLaunch as u8 }>, _: Cx) -> bool {
        match self.0 .0 {
            Either::Left(o) => o.entity.input() <= HYPED_LAUNCH_CAP_LOVELACE,
            Either::Right(_) => true,
        }
    }
}

const SAFETY_DELAY: Slot = 20;

impl ConditionalValidation<{ Validations::CancellationLock as u8 }, Slot> for EvolvingCardanoEntity {
    fn cond(&self, _: Id<{ Validations::CancellationLock as u8 }>) -> bool {
        true
    }

    fn is_valid(&self, _: Id<{ Validations::CancellationLock as u8 }>, session_end: Slot) -> bool {
        match self.0 .0 {
            Either::Left(o) => o.entity.0.cancellation_after > session_end + SAFETY_DELAY,
            Either::Right(_) => true,
        }
    }
}
