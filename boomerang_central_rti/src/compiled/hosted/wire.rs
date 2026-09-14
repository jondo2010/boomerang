//! Conversion at the admitted typed channel boundary; serialization belongs to the portable codec.
use super::*;
use boomerang_federated::wire::{Message, Session};
use boomerang_runtime::image::RtiRouteImage;

/// Borrows the compiler's original member and route tables for exact wire admission.
pub type WireContract<'a> =
    boomerang_federated::wire::Contract<'a, FederateIndex, RtiRouteIndex, RtiRouteImage<'a>>;
/// One direction-neutral canonical session for a channel bound to a compiled member.
pub(super) type WireSession<'a, 'image> =
    Session<'a, 'image, FederateIndex, RtiRouteIndex, RtiRouteImage<'image>>;

/// Owns an admitted upstream record, retaining its original typed route domain.
pub(super) fn request_from(
    message: Message<'_, RtiRouteIndex>,
) -> Result<RtiRequest, CentralRtiError> {
    let Message::Request(request) = message else {
        return Err(HostedError::Direction.into());
    };
    request.try_map(
        Ok::<_, CentralRtiError>,
        |bytes| bytes.to_vec(),
        |text| (*text).to_owned(),
    )
}
/// Owns an admitted downstream record; upstream traffic is rejected before dispatch.
pub(super) fn reply_from(message: Message<'_, RtiRouteIndex>) -> Result<RtiReply, CentralRtiError> {
    let Message::Reply(reply) = message else {
        return Err(HostedError::Direction.into());
    };
    reply.try_map(
        Ok::<_, CentralRtiError>,
        |bytes| bytes.to_vec(),
        |text| (*text).to_owned(),
    )
}
/// Classifies admission without granting permission to reorder frames.
pub(super) fn class(message: &Message<'_, RtiRouteIndex>) -> Class {
    if matches!(
        message,
        Message::Request(canonical::Request::Payload { .. })
            | Message::Reply(canonical::Reply::Payload { .. })
    ) {
        Class::Payload
    } else {
        Class::Coordination
    }
}
/// Encodes into fixed maximum scratch before retaining only the complete frame.
pub(super) fn encode(
    session: &mut WireSession<'_, '_>,
    message: &Message<'_, RtiRouteIndex>,
) -> Result<Vec<u8>, CentralRtiError> {
    let mut bytes = vec![0; MAX_FRAME_BYTES];
    let count = session
        .encode(message, &mut bytes)
        .map_err(HostedError::from)?;
    bytes.truncate(count);
    Ok(bytes)
}
