#[derive(PartialEq, Clone, serde::Deserialize, serde::Serialize, Debug)]
pub struct AuthedIntent {
    pub intent: Vec<u8>,
    pub prefix: Vec<u8>,
    pub postfix: Vec<u8>,
    pub signature: Vec<u8>,
    pub credential: [u8; 32],
}

impl AuthedIntent {
    pub fn encode(&self) -> Vec<u8> {
        let mut encoded = Vec::new();

        // Encode intent with length prefix
        encoded.extend((self.intent.len() as u32).to_be_bytes());
        encoded.extend(&self.intent);

        // Encode prefix with length prefix
        encoded.extend((self.prefix.len() as u32).to_be_bytes());
        encoded.extend(&self.prefix);

        // Encode postfix with length prefix
        encoded.extend((self.postfix.len() as u32).to_be_bytes());
        encoded.extend(&self.postfix);

        // Encode signature with length prefix
        encoded.extend((self.signature.len() as u32).to_be_bytes());
        encoded.extend(&self.signature);

        // Encode credential (fixed size, no length prefix needed)
        encoded.extend(&self.credential);

        encoded
    }

    pub fn decode(encoded: &[u8]) -> Result<AuthedIntent, String> {
        let mut cursor = 0;

        // Decode intent
        let intent_len = Self::read_u32(encoded, &mut cursor)?;
        let intent = Self::read_vec(encoded, intent_len, &mut cursor)?;

        // Decode prefix
        let prefix_len = Self::read_u32(encoded, &mut cursor)?;
        let prefix = Self::read_vec(encoded, prefix_len, &mut cursor)?;

        // Decode postfix
        let postfix_len = Self::read_u32(encoded, &mut cursor)?;
        let postfix = Self::read_vec(encoded, postfix_len, &mut cursor)?;

        // Decode signature
        let signature_len = Self::read_u32(encoded, &mut cursor)?;
        let signature = Self::read_vec(encoded, signature_len, &mut cursor)?;

        // Decode credential
        if cursor + 32 > encoded.len() {
            return Err("Invalid data: incomplete credential".to_string());
        }
        let mut credential = [0u8; 32];
        credential.copy_from_slice(&encoded[cursor..cursor + 32]);
        cursor += 32;

        Ok(AuthedIntent {
            intent,
            prefix,
            postfix,
            signature,
            credential,
        })
    }

    fn read_u32(data: &[u8], cursor: &mut usize) -> Result<u32, String> {
        if *cursor + 4 > data.len() {
            return Err("Invalid data: unexpected end of input".to_string());
        }
        let value = u32::from_be_bytes(data[*cursor..*cursor + 4].try_into().unwrap());
        *cursor += 4;
        Ok(value)
    }

    fn read_vec(data: &[u8], len: u32, cursor: &mut usize) -> Result<Vec<u8>, String> {
        let len = len as usize;
        if *cursor + len > data.len() {
            return Err("Invalid data: unexpected end of input".to_string());
        }
        let value = data[*cursor..*cursor + len].to_vec();
        *cursor += len;
        Ok(value)
    }
}

impl TryFrom<&[u8]> for AuthedIntent {
    type Error = String;
    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        Self::decode(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_authed_intent() {
        let authed_intent = AuthedIntent {
            intent: vec![1, 2, 3, 4],
            prefix: vec![5, 6, 7],
            postfix: vec![8, 9],
            signature: vec![10, 11, 12, 13, 14],
            credential: [15; 32],
        };

        // Encode
        let encoded = authed_intent.encode();

        // Decode
        let decoded = AuthedIntent::decode(&encoded).expect("Decoding failed");

        // Assert that the encoded and decoded data match
        assert_eq!(authed_intent, decoded);
    }

    #[test]
    fn test_decode_invalid_data() {
        // Incomplete data
        let invalid_encoded: Vec<u8> = vec![0, 0, 0, 4, 1, 2]; // Truncated data
        let result = AuthedIntent::decode(&invalid_encoded);

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Invalid data: unexpected end of input");

        // Incomplete credential
        let incomplete_credential: Vec<u8> = vec![
            0, 0, 0, 4, 1, 2, 3, 4, // Intent
            0, 0, 0, 3, 5, 6, 7, // Prefix
            0, 0, 0, 2, 8, 9, // Postfix
            0, 0, 0, 5, 10, 11, 12, 13, 14, // Signature
            15, 15, 15, // Incomplete credential
        ];
        let result = AuthedIntent::decode(&incomplete_credential);

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Invalid data: incomplete credential");
    }

    #[test]
    fn test_roundtrip_with_empty_fields() {
        let authed_intent = AuthedIntent {
            intent: vec![],
            prefix: vec![],
            postfix: vec![],
            signature: vec![],
            credential: [0; 32],
        };

        // Encode
        let encoded = authed_intent.encode();

        // Decode
        let decoded = AuthedIntent::decode(&encoded).expect("Decoding failed");

        // Assert that the encoded and decoded data match
        assert_eq!(authed_intent, decoded);
    }
}
