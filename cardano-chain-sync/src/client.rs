use std::marker::PhantomData;
use std::path::Path;
use std::sync::Arc;

use cml_core::serialization::Deserialize;
use cml_core::Slot;
use cml_crypto::BlockHeaderHash;
use log::debug;
use pallas_network::miniprotocols::chainsync::{BlockContent, NextResponse, State};
use pallas_network::miniprotocols::handshake::RefuseReason;
use pallas_network::miniprotocols::{chainsync, handshake, PROTOCOL_N2C_CHAIN_SYNC, PROTOCOL_N2C_HANDSHAKE};
use pallas_network::multiplexer;
use pallas_network::multiplexer::{Bearer, RunningPlexer};
use tokio::sync::Mutex;

use crate::cache::LedgerCache;
use crate::data::ChainUpgrade;

pub struct ChainSyncClient<Block> {
    plexer: RunningPlexer,
    chain_sync: chainsync::N2CClient,
    block: PhantomData<Block>,
}

impl<Block> ChainSyncClient<Block> {
    #[cfg(not(target_os = "windows"))]
    pub async fn init<'a, Cache>(
        cache: Arc<Mutex<Cache>>,
        path: impl AsRef<Path>,
        magic: u64,
        starting_point: Point,
    ) -> Result<Self, Error>
    where
        Cache: LedgerCache,
    {
        let bearer = Bearer::connect_unix(path).await.map_err(Error::ConnectFailure)?;

        let mut mplex = multiplexer::Plexer::new(bearer);

        let hs_channel = mplex.subscribe_client(PROTOCOL_N2C_HANDSHAKE);
        let cs_channel = mplex.subscribe_client(PROTOCOL_N2C_CHAIN_SYNC);

        let plexer = mplex.spawn();

        let versions = handshake::n2c::VersionTable::v10_and_above(magic);
        let mut client = handshake::Client::new(hs_channel);

        let handshake = client
            .handshake(versions)
            .await
            .map_err(Error::HandshakeProtocol)?;

        if let handshake::Confirmation::Rejected(reason) = handshake {
            return Err(Error::HandshakeRefused(reason));
        }

        let mut cs_client = chainsync::Client::new(cs_channel);

        let best_point = cache.lock().await.get_tip().await.unwrap_or(starting_point);

        debug!("Using {:?} as a starting point", best_point);

        if let (None, _) = cs_client
            .find_intersect(vec![best_point.into()])
            .await
            .map_err(Error::ChainSyncProtocol)?
        {
            return Err(Error::IntersectionNotFound);
        }

        Ok(Self {
            plexer,
            chain_sync: cs_client,
            block: PhantomData::default(),
        })
    }

    pub async fn try_pull_next(&mut self) -> Option<ChainUpgrade<Block>>
    where
        Block: Deserialize,
    {
        let response = match self.chain_sync.state() {
            State::MustReply => self.chain_sync.recv_while_can_await().await,
            _ => self.chain_sync.request_next().await,
        };
        match response {
            Ok(NextResponse::RollForward(BlockContent(raw), _)) => {
                let original_bytes = raw[BLK_START..].to_vec();
                match Block::from_cbor_bytes(&original_bytes) {
                    Ok(blk) => Some(ChainUpgrade::RollForward {
                        blk,
                        blk_bytes: original_bytes,
                        replayed: false,
                    }),
                    Err(err) => panic!(
                        "Block deserialization failed: {}, bytes: {}",
                        err,
                        hex::encode(original_bytes)
                    ),
                }
            }
            Ok(NextResponse::RollBackward(pt, _)) => Some(ChainUpgrade::RollBackward(pt.into())),
            _ => None,
        }
    }

    pub async fn close(self) {
        self.plexer.abort().await
    }
}

const BLK_START: usize = 2;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("error connecting bearer")]
    ConnectFailure(#[source] tokio::io::Error),

    #[error("handshake protocol error")]
    HandshakeProtocol(handshake::Error),

    #[error("chain-sync protocol error")]
    ChainSyncProtocol(chainsync::ClientError),

    #[error("handshake version not accepted")]
    HandshakeRefused(RefuseReason),

    #[error("intersection not found")]
    IntersectionNotFound,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Point {
    Origin,
    Specific(u64, BlockHeaderHash),
}

impl Point {
    pub fn get_slot(&self) -> Slot {
        match self {
            Point::Origin => 0,
            Point::Specific(s, _) => *s,
        }
    }
}

impl From<Point> for pallas_network::miniprotocols::Point {
    fn from(value: Point) -> Self {
        match value {
            Point::Origin => pallas_network::miniprotocols::Point::Origin,
            Point::Specific(tip, pt) => {
                pallas_network::miniprotocols::Point::Specific(tip, <[u8; 32]>::from(pt).into())
            }
        }
    }
}

impl Into<Point> for pallas_network::miniprotocols::Point {
    fn into(self) -> Point {
        match self {
            pallas_network::miniprotocols::Point::Origin => Point::Origin,
            pallas_network::miniprotocols::Point::Specific(tip, pt) => {
                Point::Specific(tip, <[u8; 32]>::try_from(pt).unwrap().into())
            }
        }
    }
}
