//! Assembly-scoped payload codec registration for cross-Federate connections.

use std::{
    any::{type_name, Any, TypeId},
    collections::HashMap,
    sync::Arc,
};

use crate::{runtime, Assembly, AssemblyError, AssemblyPortKey};

pub(crate) type FederatedCodecPair<T> = (
    Box<dyn boomerang_federated::PayloadEncoder<T>>,
    Box<dyn boomerang_federated::PayloadDecoder<T>>,
);

type FederatedCodecEntry = dyn Any + Send + Sync;

#[derive(Default)]
pub(crate) struct FederatedCodecRegistry {
    entries: HashMap<TypeId, Box<FederatedCodecEntry>>,
}

struct FederatedCodecRegistration<T: runtime::ReactorData> {
    encoder_factory: Box<dyn Fn() -> Box<dyn boomerang_federated::PayloadEncoder<T>> + Send + Sync>,
    decoder_factory: Box<dyn Fn() -> Box<dyn boomerang_federated::PayloadDecoder<T>> + Send + Sync>,
}

struct FederatedEncoderAdapter<C> {
    codec: Arc<C>,
}

impl<T, C> boomerang_federated::PayloadEncoder<T> for FederatedEncoderAdapter<C>
where
    T: runtime::ReactorData,
    C: boomerang_federated::PayloadEncoder<T> + Send + Sync + 'static,
{
    fn encode(&self, value: &T) -> Result<Vec<u8>, boomerang_federated::CodecError> {
        self.codec.encode(value)
    }
}

struct FederatedDecoderAdapter<C> {
    codec: Arc<C>,
}

impl<T, C> boomerang_federated::PayloadDecoder<T> for FederatedDecoderAdapter<C>
where
    T: runtime::ReactorData,
    C: boomerang_federated::PayloadDecoder<T> + Send + Sync + 'static,
{
    fn decode(&self, bytes: &[u8]) -> Result<T, boomerang_federated::CodecError> {
        self.codec.decode(bytes)
    }
}

impl Assembly {
    pub fn register_federated_codec<T, C>(&mut self, codec: C) -> Result<(), AssemblyError>
    where
        T: runtime::ReactorData,
        C: boomerang_federated::PayloadEncoder<T>
            + boomerang_federated::PayloadDecoder<T>
            + Send
            + Sync
            + 'static,
    {
        let type_id = TypeId::of::<T>();
        if self.federated_codecs.entries.contains_key(&type_id) {
            return Err(AssemblyError::UnsupportedFederationTopology {
                what: format!(
                    "federated codec for payload type '{}' is already registered",
                    type_name::<T>()
                ),
            });
        }

        let codec = Arc::new(codec);
        let encoder_codec = Arc::clone(&codec);
        let decoder_codec = Arc::clone(&codec);
        self.federated_codecs.entries.insert(
            type_id,
            Box::new(FederatedCodecRegistration::<T> {
                encoder_factory: Box::new(move || {
                    Box::new(FederatedEncoderAdapter {
                        codec: Arc::clone(&encoder_codec),
                    })
                }),
                decoder_factory: Box::new(move || {
                    Box::new(FederatedDecoderAdapter {
                        codec: Arc::clone(&decoder_codec),
                    })
                }),
            }),
        );
        Ok(())
    }

    pub(crate) fn federated_codec_for<T>(
        &self,
        source_key: AssemblyPortKey,
        target_key: AssemblyPortKey,
    ) -> Result<FederatedCodecPair<T>, AssemblyError>
    where
        T: runtime::ReactorData,
    {
        let source_fqn = self.fqn_for(source_key, false)?;
        let target_fqn = self.fqn_for(target_key, false)?;
        let entry = self
            .federated_codecs
            .entries
            .get(&TypeId::of::<T>())
            .ok_or_else(|| AssemblyError::UnsupportedFederationTopology {
                what: format!(
                    "cross-federate connection '{}' -> '{}' requires a federated codec for payload type '{}'; register one on Assembly with register_federated_codec::<T, _>(...)",
                    source_fqn,
                    target_fqn,
                    type_name::<T>(),
                ),
            })?;
        let registration = entry
            .downcast_ref::<FederatedCodecRegistration<T>>()
            .ok_or_else(|| {
                AssemblyError::InternalError(format!(
                    "federated codec registry type mismatch for payload type '{}'",
                    type_name::<T>()
                ))
            })?;
        Ok((
            (registration.encoder_factory)(),
            (registration.decoder_factory)(),
        ))
    }
}
