use crate::entity::EvolvingCardanoEntity;
use bloom_offchain::execution_engine::liquidity_book::market_taker::MarketTaker;
use either::Either;
use spectrum_offchain::data::ior::Ior;
use spectrum_offchain::domain::event::{Channel, Transition};

pub trait ConditionalValidation<const I: u8> {
    fn cond(&self) -> bool;
    fn is_valid(&self) -> bool;
}

impl<const I: u8, T: ConditionalValidation<I>> ConditionalValidation<I> for Ior<T, T> {
    fn cond(&self) -> bool {
        match self {
            Ior::Left(left) => left.cond(),
            Ior::Right(right) => right.cond(),
            Ior::Both(left, right) => left.cond() || right.cond(),
        }
    }

    fn is_valid(&self) -> bool {
        match self {
            Ior::Left(left) => left.is_valid(),
            Ior::Right(right) => right.is_valid(),
            Ior::Both(left, right) => left.is_valid() && right.is_valid(),
        }
    }
}

impl<const I: u8, T: ConditionalValidation<I>> ConditionalValidation<I> for Transition<T> {
    fn cond(&self) -> bool {
        match self {
            Transition::Forward(ior) => ior.cond(),
            Transition::Backward(ior) => ior.cond(),
        }
    }

    fn is_valid(&self) -> bool {
        match self {
            Transition::Forward(ior) => ior.is_valid(),
            Transition::Backward(ior) => ior.is_valid(),
        }
    }
}

impl<const I: u8, T: ConditionalValidation<I>, C> ConditionalValidation<I> for Channel<T, C> {
    fn cond(&self) -> bool {
        match self {
            Channel::Ledger(confirmed, _) => confirmed.0.cond(),
            Channel::Mempool(unconfirmed) => unconfirmed.0.cond(),
            Channel::LocalTxSubmit(predicted) => predicted.0.cond(),
        }
    }

    fn is_valid(&self) -> bool {
        match self {
            Channel::Ledger(confirmed, _) => confirmed.0.is_valid(),
            Channel::Mempool(unconfirmed) => unconfirmed.0.is_valid(),
            Channel::LocalTxSubmit(predicted) => predicted.0.is_valid(),
        }
    }
}

#[repr(u8)]
pub enum Validations {
    HypedLaunch,
}

const HYPED_LAUNCH_CAP_LOVELACE: u64 = 50_000_000;

impl ConditionalValidation<{ Validations::HypedLaunch as u8 }> for EvolvingCardanoEntity {
    fn cond(&self) -> bool {
        match self.0 .0 {
            Either::Left(_) => false,
            Either::Right(p) => p.entity.capped,
        }
    }

    fn is_valid(&self) -> bool {
        match self.0 .0 {
            Either::Left(o) => o.entity.input() <= HYPED_LAUNCH_CAP_LOVELACE,
            Either::Right(_) => true,
        }
    }
}
