use crate::config::crypto_container::CryptoContainerInitError::{
    CipheredContainerParsingError, CipheredContainerProcessingError, CommonSecretRecoveryFailed,
    ShamirPasswordsParsingError,
};
use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::Engine;
use cml_crypto::blake2b224;
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::RsaPrivateKey;
use sharks::{Share, Sharks};
use std::io;
use std::io::Write;

pub struct CryptoContainer {
    key: RsaPrivateKey,
}

pub enum CryptoContainerInitError {
    ShamirPasswordsParsingError,
    CipheredContainerParsingError,
    CipheredContainerProcessingError,
    CommonSecretRecoveryFailed,
    Error,
}

impl CryptoContainer {
    pub fn read() -> Result<Self, CryptoContainerInitError> {
        let mut first_password = String::new();
        io::stdout().write("Enter first key".as_ref()).unwrap();
        io::stdin()
            .read_line(&mut first_password)
            .map_err(|_| ShamirPasswordsParsingError)?;
        let mut second_password = String::new();
        io::stdout().write("Enter second key".as_ref()).unwrap();
        io::stdin()
            .read_line(&mut second_password)
            .map_err(|_| ShamirPasswordsParsingError)?;

        let shares: Vec<Share> = vec![first_password.trim(), second_password.trim()]
            .into_iter()
            .map(|s| {
                let decoded_share = hex::decode(s).unwrap();
                Share::try_from(decoded_share.as_ref()).map_err(|_| CommonSecretRecoveryFailed)
            })
            .collect::<Result<_, _>>()?;

        let sharks = Sharks(2);
        let restored_raw_aes_key = sharks
            .recover(shares.iter().collect::<Vec<_>>())
            .map_err(|_| CommonSecretRecoveryFailed)?;
        let raw_nonce = blake2b224(restored_raw_aes_key.as_ref());
        let aes_key = Key::<Aes256Gcm>::from_slice(restored_raw_aes_key.as_ref());
        let aes_cipher = Aes256Gcm::new(aes_key);
        let nonce = Nonce::from_slice(raw_nonce.as_ref());
        io::stdout().write("Enter container".as_ref()).unwrap();
        let mut container_buffer = String::new();
        io::stdin()
            .read_line(&mut second_password)
            .map_err(|_| ShamirPasswordsParsingError)?;
        let raw_container = aes_cipher
            .decrypt(nonce, container_buffer.as_ref())
            .map_err(|_| CipheredContainerProcessingError)?;
        let container = CryptoContainer {
            key: RsaPrivateKey::from_pkcs1_pem(std::str::from_utf8(&raw_container).expect("invalid UTF-8"))
                .unwrap(),
        };
        Ok(container)
    }
}
