use cml_chain::block::Block;
use cml_core::serialization::Deserialize;
use pallas_network::miniprotocols::chainsync::{BlockContent, NextResponse};
use pallas_network::miniprotocols::handshake::RefuseReason;
use pallas_network::miniprotocols::{chainsync, handshake, PROTOCOL_N2C_CHAIN_SYNC, PROTOCOL_N2C_HANDSHAKE};
use pallas_network::multiplexer;
use pallas_network::multiplexer::Bearer;
use tokio::task::JoinHandle;

use crate::data::ChainUpgrade;

pub struct ChainSyncClient {
    mplex_handle: JoinHandle<Result<(), multiplexer::Error>>,
    chain_sync: chainsync::N2CClient,
}

impl ChainSyncClient {
    #[cfg(not(target_os = "windows"))]
    pub async fn init<'a>(conf: ChainSyncConf<'a>) -> Result<Self, Error> {
        let bearer = Bearer::connect_unix(conf.path)
            .await
            .map_err(Error::ConnectFailure)?;

        let mut mplex = multiplexer::Plexer::new(bearer);

        let hs_channel = mplex.subscribe_client(PROTOCOL_N2C_HANDSHAKE);
        let cs_channel = mplex.subscribe_client(PROTOCOL_N2C_CHAIN_SYNC);

        let mplex_handle = tokio::spawn(async move { mplex.run().await });

        let versions = handshake::n2c::VersionTable::v10_and_above(conf.magic);
        let mut client = handshake::Client::new(hs_channel);

        let handshake = client
            .handshake(versions)
            .await
            .map_err(Error::HandshakeProtocol)?;

        if let handshake::Confirmation::Rejected(reason) = handshake {
            return Err(Error::HandshakeRefused(reason));
        }

        let mut cs_client = chainsync::Client::new(cs_channel);

        if let (None, _) = cs_client
            .find_intersect(vec![conf.starting_point.into()])
            .await
            .map_err(Error::ChainSyncProtocol)?
        {
            return Err(Error::IntersectionNotFound);
        }

        Ok(Self {
            mplex_handle,
            chain_sync: cs_client,
        })
    }

    pub async fn try_pull_next(&mut self) -> Option<ChainUpgrade> {
        match self.chain_sync.request_next().await {
            Ok(NextResponse::RollForward(BlockContent(raw), _)) => {
                let blk = Block::from_cbor_bytes(&raw[BLK_START..]).expect("Block deserialization failed");
                Some(ChainUpgrade::RollForward(blk))
            }
            Ok(NextResponse::RollBackward(pt, _)) => Some(ChainUpgrade::RollBackward(pt)),
            _ => None,
        }
    }

    pub fn close(self) {
        self.mplex_handle.abort()
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

#[derive(serde::Deserialize)]
pub enum RawPoint {
    Origin,
    Specific(u64, String),
}

#[derive(serde::Deserialize, derive_more::Into, derive_more::From)]
#[serde(try_from = "RawPoint")]
pub struct Point(pallas_network::miniprotocols::Point);

impl TryFrom<RawPoint> for Point {
    type Error = String;
    fn try_from(value: RawPoint) -> Result<Self, Self::Error> {
        match value {
            RawPoint::Origin => Ok(Point::from(pallas_network::miniprotocols::Point::Origin)),
            RawPoint::Specific(tip, raw) => hex::decode(raw)
                .map_err(|_| "Invalid HEX point".to_string())
                .map(|pt| Point::from(pallas_network::miniprotocols::Point::Specific(tip, pt))),
        }
    }
}

#[derive(serde::Deserialize)]
pub struct ChainSyncConf<'a> {
    pub path: &'a str,
    pub magic: u64,
    pub starting_point: Point,
}
