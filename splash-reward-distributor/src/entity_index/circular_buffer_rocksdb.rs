use std::{fmt::Display, sync::Arc};

use num::{
    range,
    traits::{WrappingAdd, WrappingSub},
    Bounded, Integer, ToPrimitive, Unsigned,
};
use rocksdb::{ColumnFamily, Transaction, TransactionDB};
use serde::{de::DeserializeOwned, Serialize};

const START_IX_KEY: &str = "s:";
const END_IX_KEY: &str = "e:";
const IX_PREFIX: &str = "i:";

fn index_key<T>(ix: T) -> Vec<u8>
where
    T: Unsigned + Integer + Into<u128> + Copy,
{
    let mut bytes = IX_PREFIX.as_bytes().to_vec();
    bytes.extend_from_slice(&to_be_bytes(ix));
    bytes
}

/// Initialise buffer if `START_IX_KEY` and `END_IX_KEY` don't exist in the store. If a new store is
/// initialised, return true, false otherwise.
pub fn buffer_init<Ix>(db: &Arc<TransactionDB>, cf: &ColumnFamily) -> bool
where
    Ix: Integer + Copy + Into<u128>,
{
    let tx = db.transaction();

    if tx.get_cf(cf, START_IX_KEY).unwrap().is_none() && tx.get_cf(cf, END_IX_KEY).unwrap().is_none() {
        tx.put_cf(cf, START_IX_KEY, to_be_bytes(Ix::zero())).unwrap();
        tx.put_cf(cf, END_IX_KEY, to_be_bytes(Ix::zero())).unwrap();
        tx.commit().unwrap();
        return true;
    }
    false
}

pub fn buffer_push_back<Ix, T>(t: &T, capacity: Ix, tx: &Transaction<TransactionDB>, cf: &ColumnFamily)
where
    Ix: Display + Unsigned + Integer + Into<u128> + Copy + FromBeBytes + Bounded + WrappingSub + WrappingAdd,
    T: Serialize,
{
    let start_ix_bytes = tx.get_cf(cf, START_IX_KEY.as_bytes()).unwrap().unwrap();
    let start_ix = Ix::from_be_bytes(&start_ix_bytes).unwrap();

    let end_ix_bytes = tx.get_cf(cf, END_IX_KEY.as_bytes()).unwrap().unwrap();
    let end_ix = Ix::from_be_bytes(&end_ix_bytes).unwrap();

    tx.put_cf(
        cf,
        END_IX_KEY.as_bytes(),
        to_be_bytes(end_ix.wrapping_add(&Ix::one())),
    )
    .unwrap();

    let end_ix_key = index_key(end_ix);
    tx.put_cf(cf, end_ix_key, rmp_serde::to_vec_named(t).unwrap())
        .unwrap();

    if buffer_full(start_ix, end_ix, capacity) {
        // Delete oldest element
        let start_ix_key = index_key(start_ix);
        tx.delete_cf(cf, start_ix_key).unwrap();
        tx.put_cf(
            cf,
            START_IX_KEY.as_bytes(),
            to_be_bytes(start_ix.wrapping_add(&Ix::one())),
        )
        .unwrap();
    }
}

pub fn buffer_read_back<Ix, T>(db: &Arc<TransactionDB>, cf: &ColumnFamily) -> Option<T>
where
    Ix: Unsigned
        + Integer
        + Into<u128>
        + Copy
        + FromBeBytes
        + Bounded
        + WrappingSub
        + WrappingAdd
        + WrappingSub,
    T: DeserializeOwned + Clone,
{
    let start_ix_bytes = db.get_cf(cf, START_IX_KEY.as_bytes()).unwrap().unwrap();
    let start_ix = Ix::from_be_bytes(&start_ix_bytes).unwrap();

    let end_ix_bytes = db.get_cf(cf, END_IX_KEY.as_bytes()).unwrap().unwrap();
    let end_ix = Ix::from_be_bytes(&end_ix_bytes).unwrap();

    if start_ix != end_ix {
        let start_ix_key = index_key(end_ix.wrapping_sub(&Ix::one()));
        db.get_cf(cf, start_ix_key).unwrap().map(|b| {
            let t: T = rmp_serde::from_slice(&b).unwrap();
            t
        })
    } else {
        None
    }
}

pub fn buffer_pop_back<Ix, T>(tx: &Transaction<TransactionDB>, cf: &ColumnFamily) -> Option<T>
where
    Ix: Unsigned
        + Integer
        + Into<u128>
        + Copy
        + FromBeBytes
        + Bounded
        + WrappingSub
        + WrappingAdd
        + WrappingSub,
    T: DeserializeOwned + Clone,
{
    let start_ix_bytes = tx.get_cf(cf, START_IX_KEY.as_bytes()).unwrap().unwrap();
    let start_ix = Ix::from_be_bytes(&start_ix_bytes).unwrap();

    let end_ix_bytes = tx.get_cf(cf, END_IX_KEY.as_bytes()).unwrap().unwrap();
    let end_ix = Ix::from_be_bytes(&end_ix_bytes).unwrap();

    if start_ix != end_ix {
        let new_end_ix = end_ix.wrapping_sub(&Ix::one());
        let ix_key = index_key(new_end_ix);
        let res = tx.get_cf(cf, &ix_key).unwrap().map(|b| {
            let t: T = rmp_serde::from_slice(&b).unwrap();
            t
        });
        tx.delete_cf(cf, &ix_key).unwrap();
        tx.put_cf(cf, END_IX_KEY, to_be_bytes(new_end_ix)).unwrap();
        res
    } else {
        None
    }
}

fn buffer_read_all<Ix, T>(db: &Arc<TransactionDB>, cf: &ColumnFamily) -> Vec<T>
where
    Ix: Unsigned
        + Integer
        + Into<u128>
        + Copy
        + FromBeBytes
        + Bounded
        + WrappingSub
        + WrappingAdd
        + ToPrimitive,
    T: DeserializeOwned + Clone,
{
    let tx = db.transaction();
    let start_ix_bytes = tx.get_cf(cf, START_IX_KEY.as_bytes()).unwrap().unwrap();
    let start_ix = Ix::from_be_bytes(&start_ix_bytes).unwrap();

    let end_ix_bytes = tx.get_cf(cf, END_IX_KEY.as_bytes()).unwrap().unwrap();
    let end_ix = Ix::from_be_bytes(&end_ix_bytes).unwrap();

    let mut res = vec![];
    let len = end_ix.wrapping_sub(&start_ix);
    if start_ix != end_ix {
        for i in range(Ix::zero(), len) {
            let ix_key = index_key(start_ix.wrapping_add(&i));
            let t = tx
                .get_cf(cf, ix_key)
                .unwrap()
                .map(|b| {
                    let t: T = rmp_serde::from_slice(&b).unwrap();
                    t
                })
                .unwrap();
            res.push(t);
        }
    }
    res
}

#[derive(Debug, PartialEq, Eq)]
struct BufferDebugInfo<Ix, T> {
    start_ix: Ix,
    end_ix: Ix,
    buffer_contents: Vec<T>,
}

fn buffer_debug<Ix, T>(db: &Arc<TransactionDB>, cf: &ColumnFamily) -> BufferDebugInfo<Ix, T>
where
    Ix: Unsigned
        + Integer
        + Into<u128>
        + Copy
        + FromBeBytes
        + Bounded
        + WrappingSub
        + WrappingAdd
        + ToPrimitive,
    T: DeserializeOwned + Clone,
{
    let tx = db.transaction();
    let start_ix_bytes = tx.get_cf(cf, START_IX_KEY.as_bytes()).unwrap().unwrap();
    let start_ix = Ix::from_be_bytes(&start_ix_bytes).unwrap();

    let end_ix_bytes = tx.get_cf(cf, END_IX_KEY.as_bytes()).unwrap().unwrap();
    let end_ix = Ix::from_be_bytes(&end_ix_bytes).unwrap();

    let mut buffer_contents = vec![];
    let len = end_ix.wrapping_sub(&start_ix);
    if start_ix != end_ix {
        for i in range(Ix::zero(), len) {
            let ix_key = index_key(start_ix.wrapping_add(&i));
            let t = tx
                .get_cf(cf, ix_key)
                .unwrap()
                .map(|b| {
                    let t: T = rmp_serde::from_slice(&b).unwrap();
                    t
                })
                .unwrap();
            buffer_contents.push(t);
        }
    }
    BufferDebugInfo {
        start_ix,
        end_ix,
        buffer_contents,
    }
}

fn buffer_full<Ix>(start_ix: Ix, end_ix: Ix, capacity: Ix) -> bool
where
    Ix: Unsigned + Integer + Bounded + WrappingSub,
{
    start_ix != end_ix && end_ix.wrapping_sub(&start_ix) == capacity
}

fn to_be_bytes<T>(value: T) -> Vec<u8>
where
    T: Integer + Into<u128> + Copy,
{
    match size_of::<T>() {
        1 => (value.into() as u8).to_be_bytes().to_vec(),
        2 => (value.into() as u16).to_be_bytes().to_vec(),
        4 => (value.into() as u32).to_be_bytes().to_vec(),
        8 => (value.into() as u64).to_be_bytes().to_vec(),
        16 => value.into().to_be_bytes().to_vec(),
        _ => panic!("Unsupported integer size"),
    }
}

/// Trait for types that can be created from big-endian bytes
pub trait FromBeBytes: Sized {
    fn from_be_bytes(bytes: &[u8]) -> Result<Self, &'static str>;
}

// Implement for all primitive integer types
macro_rules! impl_from_be_bytes {
    ($($t:ty),*) => {
        $(
            impl FromBeBytes for $t {
                fn from_be_bytes(bytes: &[u8]) -> Result<Self, &'static str> {
                    if bytes.len() != std::mem::size_of::<$t>() {
                        return Err("Invalid byte length");
                    }

                    let mut array = [0u8; std::mem::size_of::<$t>()];
                    array.copy_from_slice(bytes);
                    Ok(<$t>::from_be_bytes(array))
                }
            }
        )*
    };
}

impl_from_be_bytes!(u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize);

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rand::RngCore;
    use rocksdb::{Options, TransactionDB, TransactionDBOptions};

    use crate::entity_index::circular_buffer_rocksdb::{
        buffer_debug, buffer_init, buffer_pop_back, buffer_push_back, buffer_read_all, buffer_read_back,
        BufferDebugInfo,
    };

    const CF: &str = "cf";

    #[test]
    fn test_buffer_u8() {
        let db = spawn_db();
        const CAPACITY: u8 = 4;
        let cf = db.cf_handle(CF).unwrap();
        buffer_init::<u8>(&db, cf);

        let mut expected = BufferDebugInfo {
            start_ix: 0_u8,
            end_ix: 0,
            buffer_contents: vec![],
        };

        let tx = db.transaction();
        // Fill to capacity then delete
        for i in 0_u16..4 {
            buffer_push_back(&i, CAPACITY, &tx, cf);
            expected.end_ix += 1;
            expected.buffer_contents.push(i);
        }

        tx.commit().unwrap();

        for _ in 0..4 {
            let tx = db.transaction();
            assert_eq!(
                expected.buffer_contents.pop(),
                buffer_pop_back::<u8, u16>(&tx, cf)
            );
            tx.commit().unwrap();
            expected.end_ix = expected.end_ix.wrapping_sub(1);
            assert_eq!(buffer_debug::<u8, u16>(&db, cf), expected);
        }

        assert_eq!(buffer_read_back::<u8, u16>(&db, cf), None);

        for i in 0_u16..255 {
            let tx = db.transaction();
            buffer_push_back(&i, CAPACITY, &tx, cf);
            tx.commit().unwrap();
            if expected.buffer_contents.len() == CAPACITY as usize {
                expected.buffer_contents.remove(0);
                expected.start_ix = expected.start_ix.wrapping_add(1);
            }
            expected.buffer_contents.push(i);
            expected.end_ix = expected.end_ix.wrapping_add(1);
            assert_eq!(buffer_read_all::<u8, u16>(&db, cf), expected.buffer_contents);
            assert_eq!(buffer_read_back::<u8, u16>(&db, cf), Some(i));
        }

        // Start wrap-around
        let tx = db.transaction();
        buffer_push_back(&255_u16, CAPACITY, &tx, cf);
        tx.commit().unwrap();
        expected = BufferDebugInfo {
            start_ix: 252_u8,
            end_ix: 0_u8,
            buffer_contents: vec![252, 253, 254, 255],
        };

        assert_eq!(buffer_debug::<u8, u16>(&db, cf), expected);
        let tx = db.transaction();
        assert_eq!(buffer_pop_back::<u8, u16>(&tx, cf), Some(255));
        tx.commit().unwrap();
        expected = BufferDebugInfo {
            start_ix: 252_u8,
            end_ix: 255_u8,
            buffer_contents: vec![252, 253, 254],
        };
        assert_eq!(buffer_debug::<u8, u16>(&db, cf), expected);

        // Remove 254, 243, 252
        for _ in 0..3 {
            let tx = db.transaction();
            assert_eq!(
                buffer_pop_back::<u8, u16>(&tx, cf),
                expected.buffer_contents.pop()
            );
            tx.commit().unwrap();
            expected.end_ix -= 1;
            assert_eq!(buffer_debug::<u8, u16>(&db, cf), expected);
        }

        for i in 255_u16..2000 {
            let tx = db.transaction();
            buffer_push_back(&i, CAPACITY, &tx, cf);
            tx.commit().unwrap();
            if expected.buffer_contents.len() == CAPACITY as usize {
                expected.buffer_contents.remove(0);
                expected.start_ix = expected.start_ix.wrapping_add(1);
            }
            expected.buffer_contents.push(i);
            expected.end_ix = expected.end_ix.wrapping_add(1);
            assert_eq!(buffer_read_all::<u8, u16>(&db, cf), expected.buffer_contents);
            assert_eq!(buffer_read_back::<u8, u16>(&db, cf), Some(i));
        }
        println!("{:?}", expected);
    }

    fn spawn_db() -> Arc<TransactionDB> {
        let rnd = rand::thread_rng().next_u32();
        let db_path = format!("./tmp/{}", rnd);
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        let db_opts = TransactionDBOptions::default();
        Arc::new(TransactionDB::open_cf(&opts, &db_opts, db_path, [CF]).unwrap())
    }
}
