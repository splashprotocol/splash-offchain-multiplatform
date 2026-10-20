//! Read-only check that a local Mainnet node serves the block containing the test pools.
use cardano_chain_sync::event_source::unpack_valid_transactions_multi_era;
use cml_core::serialization::{Deserialize, Serialize};
use cml_multi_era::MultiEraBlock;
use either::Either;
use pallas_network::miniprotocols::chainsync::{self, BlockContent, NextResponse};
use pallas_network::miniprotocols::{handshake, Point, PROTOCOL_N2C_CHAIN_SYNC, PROTOCOL_N2C_HANDSHAKE};
use pallas_network::multiplexer::{Bearer, Plexer};
use spectrum_cardano_lib::hash::{hash_block_header_canonical_multi_era, hash_transaction_canonical};

const BEFORE_SLOT: u64 = 200_049_102;
const BEFORE_HASH: &str = "6ff6c466583b6c9d799899a4c20bf548a998bc005f267a56de83d4608a26e0b1";
const EXPECTED_HASH: &str = "5f873f330d66856c87811678594f9b9f5c95c95f59f8bcd9c1a4950c3190074a";
const EXPECTED_TX: &str = "73db4c6e5199221b67460cfe806ae33e04699d435dfae59ecebc3d44b420d5d6";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let socket = args.next().ok_or("pass the node socket path")?;
    let before_slot: u64 = args.next().map(|s| s.parse()).transpose()?.unwrap_or(BEFORE_SLOT);
    let before_hash = args.next().unwrap_or_else(|| BEFORE_HASH.to_owned());
    let expected_hash = args.next().unwrap_or_else(|| EXPECTED_HASH.to_owned());
    let expected_tx = args.next().unwrap_or_else(|| EXPECTED_TX.to_owned());
    let bearer = Bearer::connect_unix(socket).await?;
    let mut plexer = Plexer::new(bearer);
    let hs = plexer.subscribe_client(PROTOCOL_N2C_HANDSHAKE);
    let cs = plexer.subscribe_client(PROTOCOL_N2C_CHAIN_SYNC);
    let running = plexer.spawn();
    let mut handshaker = handshake::Client::new(hs);
    let confirmation = handshaker
        .handshake(handshake::n2c::VersionTable::v10_and_above(764_824_073))
        .await?;
    if !matches!(confirmation, handshake::Confirmation::Accepted(_, _)) {
        return Err("Mainnet node handshake rejected".into());
    }
    let mut sync = chainsync::Client::new(cs);
    let point = Point::Specific(before_slot, hex::decode(before_hash)?.into());
    let (intersection, tip) = sync.find_intersect(vec![point]).await?;
    println!("intersection: {:?}, node tip: {:?}", intersection, tip);
    if intersection.is_none() {
        return Err("previous block not on node chain".into());
    }
    for _ in 0..3 {
        let response =
            tokio::time::timeout(std::time::Duration::from_secs(15), sync.request_next()).await??;
        match response {
            NextResponse::RollForward(BlockContent(raw), _) => {
                let block = MultiEraBlock::from_cbor_bytes(&raw[2..])?;
                let hash = hash_block_header_canonical_multi_era(&block.header()).to_hex();
                if hash != expected_hash {
                    return Err(format!("unexpected next block: {hash}").into());
                }
                let mut found = false;
                for (tx, _, _, _) in unpack_valid_transactions_multi_era(block) {
                    if let Either::Right(tx) = tx {
                        if hash_transaction_canonical(&tx.body).to_hex() == expected_tx {
                            found = true;
                            for (index, output) in tx.body.outputs.iter().take(3).enumerate() {
                                println!(
                                    "node pool output #{index}: {}",
                                    hex::encode(output.to_cbor_bytes())
                                );
                            }
                        }
                    }
                }
                if !found {
                    return Err("pool-creation transaction missing from node block".into());
                }
                println!("Mainnet node served pool-creation block {hash} and transaction {expected_tx}");
                running.abort().await;
                return Ok(());
            }
            NextResponse::RollBackward(point, _) => {
                println!("ChainSync intersection acknowledgement: {point:?}");
            }
            NextResponse::Await => return Err("node is waiting at the previous block".into()),
        }
    }
    Err("no roll forward after ChainSync intersection".into())
}
