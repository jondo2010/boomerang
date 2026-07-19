//! Deferred inbound endpoint bindings resolved after runtime aliases exist.

use crate::{
    assembly::{ConnectionLoweringArtifacts, RuntimeAssemblyContext},
    runtime, AssemblyActionKey, AssemblyError, AssemblyReactorKey,
};

pub(crate) type FederatedInboundEndpointFactory = dyn FnOnce(
    &RuntimeAssemblyContext,
    &mut boomerang_federated::FederatedRuntimeConnections,
) -> Result<(), AssemblyError>;

impl ConnectionLoweringArtifacts {
    pub(crate) fn add_federated_inbound_endpoint<T>(
        &mut self,
        endpoint: boomerang_federated::EndpointId,
        target_partition: AssemblyReactorKey,
        target_federate: boomerang_federated::FederateId,
        target_action_key: AssemblyActionKey,
        decoder: Box<dyn boomerang_federated::PayloadDecoder<T>>,
    ) where
        T: runtime::ReactorData,
    {
        self.federated_inbound_endpoint_factories.push(Box::new(
            move |runtime_assembly, connections| {
                let (enclave_ref, runtime_action_key) = runtime_assembly
                    .aliases
                    .action_aliases
                    .get(target_action_key)
                    .ok_or_else(|| {
                        AssemblyError::InternalError(format!(
                            "missing runtime action alias for federated endpoint {endpoint}"
                        ))
                    })?
                    .clone();
                let expected_enclave_ref =
                    &runtime_assembly.aliases.enclave_aliases[target_partition];
                if &enclave_ref != expected_enclave_ref {
                    return Err(AssemblyError::InternalError(format!(
                        "federated endpoint {endpoint} resolved to wrong target enclave"
                    )));
                }

                let enclave_key = enclave_ref.enclave_key();
                let enclave = runtime_assembly.enclave(&enclave_ref);
                let context = enclave.create_send_context(enclave_key);
                let action_ref = enclave.create_async_action_ref(runtime_action_key);
                connections
                    .register_inbound(&target_federate, endpoint, context, action_ref, decoder)
                    .map_err(|error| AssemblyError::UnsupportedFederationTopology {
                        what: error.to_string(),
                    })
            },
        ));
    }
}
