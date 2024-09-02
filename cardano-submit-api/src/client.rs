use std::marker::PhantomData;
use std::path::Path;

use cml_core::serialization::Serialize;
use cml_crypto::blake2b256;
use log::{trace, warn};
use pallas_network::miniprotocols::handshake::RefuseReason;
use pallas_network::miniprotocols::localtxsubmission::cardano_node_errors::ApplyTxError;
use pallas_network::miniprotocols::localtxsubmission::{EraTx, NodeErrorDecoder, Response};
use pallas_network::miniprotocols::{
    handshake, localtxsubmission, PROTOCOL_N2C_HANDSHAKE, PROTOCOL_N2C_TX_SUBMISSION,
};
use pallas_network::multiplexer;
use pallas_network::multiplexer::{Bearer, RunningPlexer};

pub struct LocalTxSubmissionClient<'a, const ERA: u16, Tx> {
    plexer: RunningPlexer,
    tx_submission: localtxsubmission::Client<'a, NodeErrorDecoder>,
    tx: PhantomData<Tx>,
}

impl<'a, const ERA: u16, Tx> LocalTxSubmissionClient<'a, ERA, Tx> {
    #[cfg(not(target_os = "windows"))]
    pub async fn init(path: impl AsRef<Path>, magic: u64) -> Result<Self, Error> {
        let bearer = Bearer::connect_unix(path).await.map_err(Error::ConnectFailure)?;

        let mut mplex = multiplexer::Plexer::new(bearer);

        let hs_channel = mplex.subscribe_client(PROTOCOL_N2C_HANDSHAKE);
        let ts_channel = mplex.subscribe_client(PROTOCOL_N2C_TX_SUBMISSION);

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

        let ts_client = localtxsubmission::Client::new(ts_channel, NodeErrorDecoder::default());

        Ok(Self {
            plexer,
            tx_submission: ts_client,
            tx: PhantomData::default(),
        })
    }

    pub async fn submit_tx(&mut self, tx: Tx) -> Result<Response<Vec<ApplyTxError>>, Error>
    where
        Tx: Serialize,
    {
        let tx_bytes = tx.to_cbor_bytes();
        let hash = hex::encode(&blake2b256(&tx_bytes)[0..8]);
        trace!("[{}] Going to submit TX", hash);
        let result = self
            .tx_submission
            .submit_tx(EraTx(ERA, tx_bytes))
            .await
            .map_err(Error::TxSubmissionProtocol);
        trace!("[{}] Submit attempt finished", hash);
        result
    }

    pub async fn close(self) {
        self.plexer.abort().await
    }

    pub fn unsafe_reset(&mut self) {
        warn!("Unsafe client reset was requested, but it is no longer supported!");
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("error connecting bearer")]
    ConnectFailure(#[source] tokio::io::Error),

    #[error("handshake protocol error")]
    HandshakeProtocol(handshake::Error),

    #[error("chain-sync protocol error")]
    TxSubmissionProtocol(#[source] localtxsubmission::Error),

    #[error("handshake version not accepted")]
    HandshakeRefused(RefuseReason),
}

#[cfg(test)]
mod tests {
    use cml_chain::transaction::Transaction;
    use cml_core::serialization::{Deserialize, Serialize};

    #[test]
    fn parse_tx() {
        let tx = Transaction::from_cbor_bytes(&*hex::decode(TX).unwrap()).unwrap();
        println!("{}", hex::encode(tx.witness_set.to_cbor_bytes()));
    }
    const TX: &str = "84a800848258201397e09845b49b01e59ea8048b89a2a892828b9927289b126c7368a1b83e64f5008258201569c7922fefa715d129c00ddad49a65505166af049de0f7c5e23d1eb6253138008258201a2548ca9a2d9fb35a16c3da5c760b7704002c14339844647415f2e9bb5e59a50082582036405bb823e2950c617d319a0c2f8b7c7cb3c7af9d398bc5d252a7ef8074b5e0000184a200583900719bee424a97b58b3dca88fe5da6feac6494aa7226f975f3506c5b257846f6bb07f5b2825885e4502679e699b4e60a0c4609a46bc35454cd01821a001ba51ca1581cd9bb181e3bb7993a343d6cdc7c1b96080243ff935ae18808432c0a44a14a4e455720534352495054195c8fa200583900719bee424a97b58b3dca88fe5da6feac6494aa7226f975f3506c5b257846f6bb07f5b2825885e4502679e699b4e60a0c4609a46bc35454cd01821a001c98a8a1581cd9bb181e3bb7993a343d6cdc7c1b96080243ff935ae18808432c0a44a14a4e455720534352495054195c99a3005839309dee0659686c3ab807895c929e3284c11222affd710b09be690f924db2f6abf60ccde92eae1a2f4fdf65f2eaf6208d872c6f0e597cc10b0701821b0000000289087f10a3581c6e917b8b965078a39804a6313e5be73535612421acd70aa83f0ec200a15820712764b20fe9746c5e6c0a576ffd0fb8a3ca136773a8cb8db02fc3fb0937b6e71b7fffffff9b8b1550581cd8eb52caf3289a2880288b23141ce3d2a7025dcf76f26fd5659add06a15820ddae4822e3183bcd3f1527cbcd723b3e57f5dd4774587f2e53c1d7052813575801581cd9bb181e3bb7993a343d6cdc7c1b96080243ff935ae18808432c0a44a14a4e4557205343524950541a0f8c756e028201d818590109d8798bd87982581cd8eb52caf3289a2880288b23141ce3d2a7025dcf76f26fd5659add065820ddae4822e3183bcd3f1527cbcd723b3e57f5dd4774587f2e53c1d70528135758d879824040d87982581cd9bb181e3bb7993a343d6cdc7c1b96080243ff935ae18808432c0a444a4e455720534352495054d87982581c6e917b8b965078a39804a6313e5be73535612421acd70aa83f0ec2005820712764b20fe9746c5e6c0a576ffd0fb8a3ca136773a8cb8db02fc3fb0937b6e71a000182b818641913880081d87981d87a81581ce5619f78d716cc24ea924e4412d83353ae390edd9905ecc6d5c85c2b00581c75c4570eb625ae881b32a34c52b159f6f3f3f2c7aaabf5bac4688133a200583900719bee424a97b58b3dca88fe5da6feac6494aa7226f975f3506c5b257846f6bb07f5b2825885e4502679e699b4e60a0c4609a46bc35454cd01821a001c98a8a1581cd9bb181e3bb7993a343d6cdc7c1b96080243ff935ae18808432c0a44a14a4e455720534352495054195c93021a0009c45405a1581df096f5c1bee23481335ff4aece32fe1dfa1aa40a944a66d2d6edc9a9a5000b58209ee08108ac764127bf5a614ca78b48452621d7eca49693db1a569bcc7dcd3ba60d81825820204619c402f17d83be5cdf7728514053a948f46ab44cf193957c720ba1b53328000e81581c5cb2c968e5d1c7197a6ce7615967310a375545d9bc65063a964335b21283825820a85dbebcf7a9c27a2548191e6516b12140ec744c142248355ddeef8652ab071f00825820155b2bf8fefd246f89d6ed25b446a0e92e8ab38400ecf81ee68bfd3179e6f4120082582077fc5b1688a79dd5b9bdba5981b7403b519690dda4f1471fffa78151bbefe0f501a20081825820d39f6e2677e23ac35da0a65bef78ae5cfdf34eb57c4059d4d7f31aa9ccdb8eff5840e1b6cd46c1a0a48130cb96e76e6ba7b158a7c541b8d8419b35c324ff072e029f9e6792e1265223a73694549c38f7d62a6068a703ec8ffeacd97632c8a2c362010585840000d87a80821a000298101a03dfd240840001d87a80821a000298101a03dfd240840002d879820202821a000a60401a1017df80840003d87a80821a000298101a03dfd24084030080821a0012ebc01a1b1ebfc0f5f6";
}
