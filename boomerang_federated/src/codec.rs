/// Error returned by payload codec adapters.
#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[cfg(feature = "serde-json-codec")]
    #[error("serde JSON codec error: {0}")]
    SerdeJson(#[from] serde_json::Error),

    #[error("{0}")]
    Message(String),
}

impl CodecError {
    pub fn message(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }
}

/// Encodes typed payload values into protocol bytes.
pub trait PayloadEncoder<T>: Send + Sync + 'static {
    fn encode(&self, value: &T) -> Result<Vec<u8>, CodecError>;
}

/// Decodes protocol bytes into typed payload values.
pub trait PayloadDecoder<T>: Send + Sync + 'static {
    fn decode(&self, bytes: &[u8]) -> Result<T, CodecError>;
}

/// Marker trait for adapters that can both encode and decode a payload type.
pub trait PayloadCodec<T>: PayloadEncoder<T> + PayloadDecoder<T> {}

impl<T, C> PayloadCodec<T> for C where C: PayloadEncoder<T> + PayloadDecoder<T> {}

/// JSON-backed serde adapter for first-slice tests and simple integrations.
#[cfg(feature = "serde-json-codec")]
#[derive(Debug, Clone, Copy, Default)]
pub struct SerdeJsonCodec;

#[cfg(feature = "serde-json-codec")]
impl<T> PayloadEncoder<T> for SerdeJsonCodec
where
    T: serde::Serialize,
{
    fn encode(&self, value: &T) -> Result<Vec<u8>, CodecError> {
        serde_json::to_vec(value).map_err(CodecError::from)
    }
}

#[cfg(feature = "serde-json-codec")]
impl<T> PayloadDecoder<T> for SerdeJsonCodec
where
    T: serde::de::DeserializeOwned,
{
    fn decode(&self, bytes: &[u8]) -> Result<T, CodecError> {
        serde_json::from_slice(bytes).map_err(CodecError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "serde-json-codec")]
    #[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    struct Payload {
        value: u32,
    }

    #[cfg(feature = "serde-json-codec")]
    #[test]
    fn serde_json_codec_round_trips_payloads() {
        let codec = SerdeJsonCodec;
        let encoded = codec.encode(&Payload { value: 42 }).unwrap();
        let decoded: Payload = codec.decode(&encoded).unwrap();

        assert_eq!(decoded, Payload { value: 42 });
    }
}
