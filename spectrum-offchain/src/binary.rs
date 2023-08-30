use serde::Serialize;

pub fn prefixed_key<T: Serialize>(prefix: &str, id: &T) -> Vec<u8> {
    let mut key_bytes = bincode::serialize(prefix).unwrap();
    let id_bytes = bincode::serialize(&id).unwrap();
    key_bytes.extend_from_slice(&id_bytes);
    key_bytes
}

pub fn raw_prefixed_key(prefix: &str, bytes: &[u8]) -> Vec<u8> {
    let mut key_bytes = bincode::serialize(prefix).unwrap();
    key_bytes.extend_from_slice(bytes);
    key_bytes
}
