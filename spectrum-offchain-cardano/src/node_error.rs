use std::collections::HashSet;

use circular_buffer::CircularBuffer;
use cml_crypto::{RawBytesEncoding, TransactionHash};
use tailcall::tailcall;

use spectrum_cardano_lib::OutputRef;

const PREFIX_LEN: usize = 3;
const PREFIX: [u8; PREFIX_LEN] = [130u8, 88, 32];
const TX_ID_LEN: usize = 32;
const TX_IX_LEN: usize = 1;
const SEGMENT_LEN: usize = TX_ID_LEN + TX_IX_LEN;

pub fn transcribe_bad_inputs_error(error_response: Vec<u8>) -> HashSet<OutputRef> {
    #[tailcall]
    fn go(
        mut prefix: CircularBuffer<PREFIX_LEN, u8>,
        mut acc: Vec<OutputRef>,
        mut rem: Box<dyn Iterator<Item = u8>>,
    ) -> Vec<OutputRef> {
        match rem.next() {
            None => acc,
            Some(byte) => {
                prefix.push_back(byte);
                if prefix == PREFIX {
                    #[tailcall]
                    fn read_segment(
                        mut acc: Vec<u8>,
                        mut rem: impl Iterator<Item = u8>,
                    ) -> (Vec<u8>, impl Iterator<Item = u8>) {
                        match rem.next() {
                            None => (acc, rem),
                            Some(byte) => {
                                acc.push(byte);
                                if acc.len() < SEGMENT_LEN {
                                    read_segment(acc, rem)
                                } else {
                                    (acc, rem)
                                }
                            }
                        }
                    }
                    let (segment, rem) = read_segment(vec![], rem);
                    match (segment.get(0..TX_ID_LEN), segment.get(TX_ID_LEN)) {
                        (Some(tx_hash), Some(ix)) => {
                            let tx_id = TransactionHash::from_raw_bytes(tx_hash).unwrap();
                            let tx_ix = *ix as u64;
                            let oref = OutputRef::new(tx_id, tx_ix);
                            acc.push(oref);
                            go(prefix, acc, Box::new(rem))
                        }
                        _ => acc
                    }
                } else {
                    go(prefix, acc, rem)
                }
            }
        }
    }
    let prefix_buff = CircularBuffer::<3, u8>::new();
    HashSet::from_iter(go(prefix_buff, vec![], Box::new(error_response.into_iter())))
}

#[cfg(test)]
mod test {
    use crate::node_error::transcribe_bad_inputs_error;
    use cml_crypto::{RawBytesEncoding, TransactionHash};
    use spectrum_cardano_lib::OutputRef;
    use std::collections::HashSet;

    const ERR: &str = "82028182059f820082018207830000000100028200820282018207820181820382018258200faddf00919ef15d38ac07684199e69be95a003a15f757bf77701072b050c1f500820082028201830500821a06760d80a1581cfd10da3e6a578708c877e14b6aaeda8dc3a36f666a346eec52a30b3aa14974657374746f6b656e1a0001fbd08200820282018200838258200faddf00919ef15d38ac07684199e69be95a003a15f757bf77701072b050c1f5008258205f85cf7db4713466bc8d9d32a84b5b6bfd2f34a76b5f8cf5a5cb04b4d6d6f0380082582096eb39b8d909373c8275c611fae63792f5e3d0a67c1eee5b3afb91fdcddc859100ff";
    #[test]
    fn read_raw_node_response() {
        let err = hex::decode(ERR).unwrap();
        let missing_inputs = transcribe_bad_inputs_error(err);
        let expected_inputs = vec![
            (
                "96eb39b8d909373c8275c611fae63792f5e3d0a67c1eee5b3afb91fdcddc8591",
                0,
            ),
            (
                "5f85cf7db4713466bc8d9d32a84b5b6bfd2f34a76b5f8cf5a5cb04b4d6d6f038",
                0,
            ),
            (
                "0faddf00919ef15d38ac07684199e69be95a003a15f757bf77701072b050c1f5",
                0,
            ),
        ]
        .into_iter()
        .map(|(tx_id, ix)| {
            OutputRef::new(
                TransactionHash::from_raw_bytes(&*hex::decode(tx_id).unwrap()).unwrap(),
                ix,
            )
        });
        assert_eq!(missing_inputs, HashSet::from_iter(expected_inputs))
    }
}
