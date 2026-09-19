use super::MAX_PAYLOAD_BYTES;
use core::marker::PhantomData;
use serde::{Deserialize, Serialize};

/// Application payload codec preserving its concrete error at the owning adapter boundary.
pub trait PayloadCodec<'de> {
    /// Value decoded from caller-owned bytes.
    type Value;
    /// Codec-specific failure without diagnostic string normalization.
    type Error;
    /// Maximum encoded value length.
    const MAX_ENCODED_BYTES: usize;
    /// Encodes into caller-provided storage and returns bytes written.
    fn encode(value: &Self::Value, output: &mut [u8]) -> Result<usize, Self::Error>;
    /// Decodes with caller scratch used to verify the exact canonical encoding.
    fn decode(bytes: &'de [u8], scratch: &mut [u8]) -> Result<Self::Value, Self::Error>;
}

mod sealed {
    pub trait Value {}
    /// Seals supported scalar and borrowed Serde destinations.
    macro_rules! values { ($($t:ty),*) => { $(impl Value for $t {})* }; }
    values!(
        (),
        bool,
        u8,
        u16,
        u32,
        u64,
        u128,
        i8,
        i16,
        i32,
        i64,
        i128,
        f32,
        f64,
        char,
        &str,
        &[u8]
    );
    impl<T: Value, const N: usize> Value for [T; N] {}
    impl<A: Value, B: Value> Value for (A, B) {}
}
/// Sealed values whose Serde decoding does not allocate: fixed-width scalars,
/// borrowed strings/bytes, fixed arrays, and pairs recursively containing these.
/// Architecture-sized integers and owning collections deliberately lack this bound.
pub trait PortableValue<'de>: sealed::Value + Serialize + Deserialize<'de> {}
impl<'de, T: sealed::Value + Serialize + Deserialize<'de>> PortableValue<'de> for T {}

/// Canonical Postcard 1.x payload codec with a compile-time byte ceiling.
///
/// Postcard scalars use its specified varints (signed integers use ZigZag),
/// IEEE float bits are little-endian, and collection lengths are minimal varints.
/// `decode` rejects trailing bytes and non-minimal encodings by re-encoding into
/// caller scratch. Both input and output are bounded by `MAX` and the frame limit.
/// `MAX` must cover every value the generated route permits; exceeding it fails.
pub struct PostcardCodec<T, const MAX: usize>(
    #[doc = "Associates the codec with its value type."] PhantomData<T>,
);

/// Payload failure preserving the original Postcard codec error.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum PayloadError {
    /// Input or configured maximum exceeds the declared protocol limit.
    #[error("payload exceeds its declared encoded size limit")]
    Oversize,
    /// Caller storage cannot hold the bounded encoded representation.
    #[error("caller storage is too small for the payload")]
    Storage,
    /// Trailing or alternate encodings are forbidden.
    #[error("payload encoding is not canonical")]
    NonCanonical,
    /// Original codec error, retained for the hosted adapter to normalize.
    #[error("Postcard codec failed: {0}")]
    Codec(#[from] postcard::Error),
}
impl<'de, T: PortableValue<'de>, const MAX: usize> PayloadCodec<'de> for PostcardCodec<T, MAX> {
    type Value = T;
    type Error = PayloadError;
    const MAX_ENCODED_BYTES: usize = MAX;
    fn encode(value: &T, output: &mut [u8]) -> Result<usize, Self::Error> {
        if MAX > MAX_PAYLOAD_BYTES {
            return Err(PayloadError::Oversize);
        }
        let length = postcard::experimental::serialized_size(value).map_err(PayloadError::Codec)?;
        if length > MAX {
            return Err(PayloadError::Oversize);
        }
        let storage = output.get_mut(..length).ok_or(PayloadError::Storage)?;
        postcard::to_slice(value, storage)
            .map(|bytes| bytes.len())
            .map_err(PayloadError::Codec)
    }
    fn decode(bytes: &'de [u8], scratch: &mut [u8]) -> Result<T, Self::Error> {
        if MAX > MAX_PAYLOAD_BYTES || bytes.len() > MAX {
            return Err(PayloadError::Oversize);
        }
        let scratch = scratch
            .get_mut(..bytes.len())
            .ok_or(PayloadError::Storage)?;
        let (value, remaining) =
            postcard::take_from_bytes::<T>(bytes).map_err(PayloadError::Codec)?;
        if !remaining.is_empty() {
            return Err(PayloadError::NonCanonical);
        }
        let canonical = postcard::to_slice(&value, scratch).map_err(PayloadError::Codec)?;
        if canonical != bytes {
            return Err(PayloadError::NonCanonical);
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::error::Error as _;
    #[test]
    fn scalar_and_borrowed_payloads_are_canonical_without_owned_decode_storage() {
        let mut output = [0; 8];
        assert_eq!(PostcardCodec::<u32, 5>::encode(&300, &mut output), Ok(2));
        assert_eq!(&output[..2], &[0xac, 2]);
        assert_eq!(
            PostcardCodec::<u32, 5>::decode(&[0xac, 2], &mut output),
            Ok(300)
        );
        let input = [1, 42];
        let borrowed = PostcardCodec::<&[u8], 8>::decode(&input, &mut output).unwrap();
        assert_eq!(borrowed.as_ptr(), input[1..].as_ptr());
        assert_eq!(
            PostcardCodec::<&[u8], 8>::decode(&[0x81, 0, 42], &mut output),
            Err(PayloadError::NonCanonical)
        );
        assert_eq!(
            PostcardCodec::<u32, 5>::decode(&[0x80, 0], &mut output),
            Err(PayloadError::NonCanonical)
        );
        assert_eq!(
            PostcardCodec::<u32, 5>::decode(&[0, 0], &mut output),
            Err(PayloadError::NonCanonical)
        );
    }
    #[test]
    fn payload_bounds_precede_decode_and_caller_scratch_is_checked() {
        assert_eq!(
            PostcardCodec::<u32, 1>::decode(&[0xac, 2], &mut []),
            Err(PayloadError::Oversize)
        );
        assert_eq!(
            PostcardCodec::<u32, 5>::decode(&[0xac, 2], &mut [0]),
            Err(PayloadError::Storage)
        );
        assert_eq!(
            PostcardCodec::<u32, 1>::encode(&300, &mut [0; 8]),
            Err(PayloadError::Oversize)
        );
        assert_eq!(
            PostcardCodec::<u32, 5>::encode(&300, &mut [0]),
            Err(PayloadError::Storage)
        );
        assert_eq!(
            PostcardCodec::<u32, { MAX_PAYLOAD_BYTES + 1 }>::encode(&0, &mut []),
            Err(PayloadError::Oversize)
        );
        let error = PostcardCodec::<bool, 1>::decode(&[2], &mut [0]).unwrap_err();
        assert_eq!(
            error,
            PayloadError::Codec(postcard::Error::DeserializeBadBool)
        );
        assert!(error.source().unwrap().is::<postcard::Error>());
    }
}
