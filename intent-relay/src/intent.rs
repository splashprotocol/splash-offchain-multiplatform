#[derive(Clone, serde::Deserialize, serde::Serialize, Debug)]
pub struct AuthedIntent {
    pub intent: Vec<u8>,
    pub prefix: Vec<u8>,
    pub postfix: Vec<u8>,
    pub signature: Vec<u8>,
    pub credential: [u8; 32],
}
