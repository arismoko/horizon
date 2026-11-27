//! Per-position connected-texture group receive path.
use crate::game::ClientGame;
use sunrise_protocol::{
    Error,
    connection_groups::{self, Section},
    packets::play::Clientbound as C,
};

#[path = "native_connection_groups/receive.rs"]
mod receive;
pub use receive::ReceivedGroup;
pub(in crate::game) use receive::{GroupUpdates, block_groups, check_group_updates, column_groups};

fn payload<'a>(packet: &'a C, channel: &str) -> Option<&'a [u8]> {
    match packet {
        C::CustomPayload(packet) if packet.channel == channel => Some(&packet.data),
        _ => None,
    }
}

#[cfg(feature = "client")]
impl ClientGame<'_> {
    /// True means this whole bundle was routed. Ordinary bundles still use the
    /// usual dispatch, with the same packet observations in their original order.
    pub fn route_connection_group_bundle(
        &mut self,
        members: &[C],
        negotiated: bool,
    ) -> sunrise_gameplay_common::game::Result<bool> {
        let [first, rest @ ..] = members else {
            return Ok(false);
        };
        if members
            .iter()
            .any(|event| payload(event, connection_groups::CHANNEL).is_some())
        {
            let result = (|| {
                if !negotiated {
                    return Err(Error::new("connection groups: channel was not negotiated"));
                }
                let sections = rest
                    .iter()
                    .map(|event| {
                        let data = payload(event, connection_groups::CHANNEL).ok_or_else(|| {
                            Error::new("connection groups: unexpected packet in bundle")
                        })?;
                        Section::decode(data)
                    })
                    .collect::<sunrise_protocol::Result<Vec<_>>>()?;
                if sections.is_empty() {
                    return Err(Error::new("connection groups: missing companion packet"));
                }
                self.observe_client_packet(first);
                self.apply_connection_group_packet(first, &sections)?;
                for packet in rest {
                    self.observe_client_packet(packet);
                }
                Ok(())
            })();
            if let Err(error) = result {
                self.refuse_remote_input(error)?;
            }
            return Ok(true);
        }
        // A scoped pair has one vanilla packet in its first run and group
        // sections in the following runs, all in one vanilla outer bundle.
        let channel = sunrise_protocol::viewed_levels::CHANNEL;
        if !rest.is_empty()
            && payload(first, channel).is_some()
            && let Some(runs) = members
                .iter()
                .map(|packet| payload(packet, channel).map(|data| (packet, data)))
                .collect::<Option<Vec<_>>>()
        {
            if let Err(error) = self.route_viewed_bundle(&runs, negotiated) {
                self.refuse_remote_input(error)?;
            }
            return Ok(true);
        }
        Ok(false)
    }

    fn apply_connection_group_packet(
        &mut self,
        packet: &C,
        sections: &[Section],
    ) -> sunrise_protocol::Result<()> {
        let limit = self.connection_group_limit();
        if let C::LevelChunkWithLight(packet) = packet {
            if !self.remote_chunk_in_range(packet.x, packet.z) {
                return Err(Error::new("connection groups: column outside the view"));
            }
            let format = self
                .client
                .remote
                .as_ref()
                .ok_or_else(|| Error::new("connection groups: no remote level"))?
                .chunk_format
                .clone();
            let mut chunk = format.column(packet).map_err(Error::new)?;
            column_groups(&mut chunk, &self.common.world, sections, limit)?;
            self.remote_chunk(packet, Some((&format, Ok(chunk))));
        } else {
            let mut groups = block_groups(&self.common.world, packet, sections)?;
            self.check_active_group_updates(&groups, limit)?;
            match packet {
                C::BlockUpdate(packet) => {
                    self.server_block(packet.pos, packet.state, groups.remove(&packet.pos))
                }
                C::SectionBlocksUpdate(packet) => {
                    self.apply_section_blocks_update_packet(packet, groups)
                }
                _ => {
                    return Err(Error::new(
                        "connection groups: companion is not a block update",
                    ));
                }
            }
        }
        Ok(())
    }
}
