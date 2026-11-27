//! Host-owned play and container packet construction; the protocol owns codecs and hashes.
use mod_api::{ContainerClickRequest, PacketBuildError, PlayPacketRequest, ServerboundPlayPacket};
use std::cell::RefCell;
use sunrise_protocol::{
    ids,
    item::{Hashed, Stack},
    packets::play::{self, ContainerClick, Serverbound, pack_move_player_flags},
    registry::{Builtin, Dynamic, SyncedNames},
};

fn encode_serverbound(packet: Serverbound) -> Result<ServerboundPlayPacket, PacketBuildError> {
    let name = usize::try_from(packet.id())
        .ok()
        .and_then(|id| ids::play::serverbound::NAMES.get(id).copied())
        .ok_or_else(|| out_of_range(packet.name(), "id"))?;
    let id =
        mod_api::Identifier::from_parts("minecraft", name).map_err(|_| out_of_range(name, "id"))?;
    let body = packet
        .try_encode_payload()
        .map_err(|_| out_of_range(name, "payload"))?;
    Ok(ServerboundPlayPacket::new(id, body))
}

fn out_of_range(packet: &str, field: impl Into<String>) -> PacketBuildError {
    PacketBuildError::OutOfRange {
        packet: packet.into(),
        field: field.into(),
    }
}

fn non_finite(packet: &str, field: &str) -> PacketBuildError {
    PacketBuildError::NonFinite {
        packet: packet.into(),
        field: field.into(),
    }
}

fn check_ordinal(packet: &str, field: &str, value: i32, max: i32) -> Result<(), PacketBuildError> {
    if (0..=max).contains(&value) {
        Ok(())
    } else {
        Err(PacketBuildError::UnknownAction {
            packet: packet.into(),
            field: field.into(),
        })
    }
}

/// Vanilla `BlockPos.asLong`: signed 26-bit X/Z and signed 12-bit Y.
fn check_block_pos(packet: &str, [x, y, z]: [i32; 3]) -> Result<(), PacketBuildError> {
    if !(-33_554_432..=33_554_431).contains(&x)
        || !(-33_554_432..=33_554_431).contains(&z)
        || !(-2048..=2047).contains(&y)
    {
        return Err(out_of_range(packet, "position"));
    }
    Ok(())
}

/// Vanilla `ServerGamePacketListenerImpl.containsInvalidValues`: rejects NaN coordinates;
/// position infinities are written unchanged and clamped by the server.
fn check_move_pos(packet: &str, pos: [f64; 3]) -> Result<(), PacketBuildError> {
    if pos.iter().any(|v| v.is_nan()) {
        return Err(non_finite(packet, "position"));
    }
    Ok(())
}

fn check_finite_angle(packet: &str, field: &str, value: f32) -> Result<(), PacketBuildError> {
    if !value.is_finite() {
        return Err(non_finite(packet, field));
    }
    Ok(())
}

/// Encode a typed client play-packet request with the protocol's vanilla serverbound encoder.
pub fn encode_play_packet(
    request: &PlayPacketRequest,
) -> Result<ServerboundPlayPacket, PacketBuildError> {
    let packet = match request {
        PlayPacketRequest::UseItem {
            hand,
            sequence,
            yaw,
            pitch,
        } => {
            check_ordinal("use_item", "hand", *hand, 1)?;
            Serverbound::UseItem(play::UseItem {
                hand: *hand,
                sequence: *sequence,
                yaw: *yaw,
                pitch: *pitch,
            })
        }
        PlayPacketRequest::UseItemOn {
            hand,
            pos,
            face,
            cursor,
            inside,
            world_border,
            sequence,
        } => {
            let p = "use_item_on";
            check_ordinal(p, "hand", *hand, 1)?;
            check_block_pos(p, *pos)?;
            check_ordinal(p, "face", *face, 5)?;
            if !cursor.iter().all(|v| v.is_finite()) {
                return Err(non_finite(p, "cursor"));
            }
            Serverbound::UseItemOn(play::UseItemOn {
                hand: *hand,
                pos: *pos,
                face: *face,
                cursor: *cursor,
                inside: *inside,
                world_border: *world_border,
                sequence: *sequence,
            })
        }
        PlayPacketRequest::PlayerAction {
            action,
            pos,
            direction,
            sequence,
        } => {
            let p = "player_action";
            check_ordinal(p, "action", *action, 8)?;
            check_block_pos(p, *pos)?;
            check_ordinal(p, "direction", *direction, 5)?;
            Serverbound::PlayerAction(play::PlayerAction {
                action: *action,
                pos: *pos,
                direction: *direction as u8,
                sequence: *sequence,
            })
        }
        PlayPacketRequest::Interact {
            entity,
            hand,
            location,
            sneaking,
        } => {
            check_ordinal("interact", "hand", *hand, 1)?;
            Serverbound::Interact(play::Interact {
                id: *entity,
                hand: *hand,
                location: *location,
                secondary: *sneaking,
            })
        }
        PlayPacketRequest::Attack { entity } => Serverbound::Attack(play::Attack { id: *entity }),
        PlayPacketRequest::PickItemFromBlock { pos, include_data } => {
            check_block_pos("pick_item_from_block", *pos)?;
            Serverbound::PickItemFromBlock(play::PickItemFromBlock {
                pos: *pos,
                include_data: *include_data,
            })
        }
        PlayPacketRequest::PickItemFromEntity {
            entity,
            include_data,
        } => Serverbound::PickItemFromEntity(play::PickItemFromEntity {
            id: *entity,
            include_data: *include_data,
        }),
        PlayPacketRequest::SignUpdate { pos, front, lines } => {
            let p = "sign_update";
            check_block_pos(p, *pos)?;
            let packet = Serverbound::SignUpdate(play::SignUpdate {
                pos: *pos,
                lines: lines.clone(),
                front: *front,
            });
            // The protocol's own validation finds the oversize line, once; no other field of
            // this packet can fail it.
            if let Some(i) = packet
                .validate()
                .err()
                .and_then(|error| error.oversize_sign_line())
            {
                return Err(PacketBuildError::SignLineTooLong {
                    packet: p.into(),
                    field: format!("lines[{i}]"),
                });
            }
            packet
        }
        PlayPacketRequest::MovePlayerPos {
            pos,
            on_ground,
            horizontal_collision,
        } => {
            check_move_pos("move_player_pos", *pos)?;
            Serverbound::MovePlayerPos(play::MovePlayerPos {
                x: pos[0],
                y: pos[1],
                z: pos[2],
                flags: pack_move_player_flags(*on_ground, *horizontal_collision),
            })
        }
        PlayPacketRequest::MovePlayerPosRot {
            pos,
            yaw,
            pitch,
            on_ground,
            horizontal_collision,
        } => {
            let p = "move_player_pos_rot";
            check_move_pos(p, *pos)?;
            check_finite_angle(p, "yaw", *yaw)?;
            check_finite_angle(p, "pitch", *pitch)?;
            Serverbound::MovePlayerPosRot(play::MovePlayerPosRot {
                x: pos[0],
                y: pos[1],
                z: pos[2],
                yaw: *yaw,
                pitch: *pitch,
                flags: pack_move_player_flags(*on_ground, *horizontal_collision),
            })
        }
        PlayPacketRequest::MovePlayerRot {
            yaw,
            pitch,
            on_ground,
            horizontal_collision,
        } => {
            let p = "move_player_rot";
            check_finite_angle(p, "yaw", *yaw)?;
            check_finite_angle(p, "pitch", *pitch)?;
            Serverbound::MovePlayerRot(play::MovePlayerRot {
                yaw: *yaw,
                pitch: *pitch,
                flags: pack_move_player_flags(*on_ground, *horizontal_collision),
            })
        }
        PlayPacketRequest::MovePlayerStatusOnly {
            on_ground,
            horizontal_collision,
        } => Serverbound::MovePlayerStatus(play::MovePlayerStatus {
            flags: pack_move_player_flags(*on_ground, *horizontal_collision),
        }),
    };
    encode_serverbound(packet)
}

fn component_error(field: impl Into<String>) -> PacketBuildError {
    PacketBuildError::ComponentCannotEncode {
        packet: "container_click".into(),
        field: field.into(),
    }
}
fn range(field: impl Into<String>) -> PacketBuildError {
    out_of_range("container_click", field)
}

fn validate(request: &ContainerClickRequest) -> Result<(), PacketBuildError> {
    ServerboundPlayPacket::validate_container_click_limits(
        request.container_id,
        request.state_id,
        request.slot,
        request.changed_slots.len() as u64,
    )?;
    i8::try_from(request.button).map_err(|_| range("button"))?;
    if !(0..=6).contains(&request.click_type) {
        return Err(PacketBuildError::UnknownAction {
            packet: "container_click".into(),
            field: "click_type".into(),
        });
    }
    let mut slots = std::collections::HashSet::new();
    for (i, (slot, _)) in request.changed_slots.iter().enumerate() {
        i16::try_from(*slot).map_err(|_| range(format!("changed_slots[{i}].slot")))?;
        if !slots.insert(slot) {
            return Err(range(format!("changed_slots[{i}].slot")));
        }
    }
    Ok(())
}

/// Encode a client mod container click with the connection registries and the game's
/// API-to-engine stack conversion. Uses the ordinary item codec and Hashed::create;
/// refuses any lost component or unresolved registry reference.
pub fn encode_container_click(
    request: &ContainerClickRequest,
    builtin: &Builtin,
    dynamic: &Dynamic,
    convert: impl Fn(&mod_api::ItemStack) -> Option<sunrise_items::items::ItemStack>,
) -> Result<ServerboundPlayPacket, PacketBuildError> {
    validate(request)?;
    let names = SyncedNames { builtin, dynamic };
    let stack = |value: &Option<mod_api::ItemStack>,
                 field: &str|
     -> Result<Option<Hashed>, PacketBuildError> {
        let Some(value) = value else {
            return Ok(None);
        };
        i32::try_from(value.count()).map_err(|_| range(format!("{field}.count")))?;
        if let Some(damage) = value.damage_component() {
            i32::try_from(damage)
                .map_err(|_| component_error(format!("{field}.components.minecraft:damage")))?;
        }
        let engine = convert(value).ok_or_else(|| component_error(field))?;
        let missing = RefCell::new(None);
        let lookup = |registry: &str, name: &str| {
            let id = builtin
                .get(registry)
                .and_then(|ids| ids.id(name))
                .or_else(|| dynamic.id(registry, name))
                .or_else(|| {
                    (dynamic.custom_components && registry == "minecraft:data_component_type")
                        .then(|| sunrise_protocol::item::custom::id(name))
                        .flatten()
                });
            if id.is_none() {
                missing
                    .borrow_mut()
                    .get_or_insert_with(|| format!("{field}.components.{name}"));
            }
            id
        };
        let item_id = lookup("minecraft:item", &engine.item)
            .ok_or_else(|| component_error(format!("{field}.item")))?;
        let mut network = crate::lan::guest::stack_to_network(
            |name| lookup("minecraft:item", name).unwrap_or(item_id),
            lookup,
            &Some(engine),
        )
        .ok_or_else(|| component_error(field))?;
        if let Some(field) = missing.into_inner() {
            return Err(component_error(field));
        }
        // The saved-component converter can report a built-in removal through both
        // its generic and specialised branches. A component patch is a set.
        let mut removed = std::collections::HashSet::new();
        network.removed.retain(|id| removed.insert(*id));
        if value.damage_component().is_some()
            && !network
                .added
                .iter()
                .any(|(id, _)| *id == sunrise_protocol::item::component::DAMAGE)
        {
            return Err(component_error(format!(
                "{field}.components.minecraft:damage"
            )));
        }
        // The existing codec deliberately drops components it cannot encode. A builder must refuse them.
        if let Some(patch) = value.components() {
            for key in patch.as_nbt().keys() {
                let key = key.to_string().map_err(|_| component_error(field))?;
                let (removed, name) = key
                    .strip_prefix('!')
                    .map_or((false, key.as_str()), |name| (true, name));
                let id = builtin
                    .get("minecraft:data_component_type")
                    .and_then(|ids| ids.id(name))
                    .or_else(|| {
                        dynamic
                            .custom_components
                            .then(|| sunrise_protocol::item::custom::id(name))
                            .flatten()
                    })
                    .ok_or_else(|| component_error(format!("{field}.components.{key}")))?;
                let represented = if removed {
                    network.removed.contains(&id)
                } else {
                    network.added.iter().any(|(kind, _)| *kind == id)
                };
                if !represented {
                    return Err(component_error(format!("{field}.components.{key}")));
                }
            }
        }
        checked_hash(&network, &names, field).map(Some)
    };
    let mut changed = Vec::with_capacity(request.changed_slots.len());
    for (i, (slot, value)) in request.changed_slots.iter().enumerate() {
        changed.push((
            i16::try_from(*slot).map_err(|_| range(format!("changed_slots[{i}].slot")))?,
            stack(value, &format!("changed_slots[{i}].stack"))?,
        ));
    }
    let packet = ContainerClick {
        container: i32::try_from(request.container_id).map_err(|_| range("container_id"))?,
        state: i32::try_from(request.state_id).map_err(|_| range("state_id"))?,
        slot: i16::try_from(request.slot).map_err(|_| range("slot"))?,
        button: i8::try_from(request.button).map_err(|_| range("button"))?,
        input: request.click_type,
        changed,
        carried: stack(&request.carried, "carried")?,
    };
    encode_serverbound(Serverbound::ContainerClick(packet))
}

fn checked_hash(
    stack: &Stack,
    names: &dyn sunrise_protocol::item::Names,
    field: &str,
) -> Result<Hashed, PacketBuildError> {
    if stack.added.len() > 256 || stack.removed.len() > 256 {
        return Err(range(format!("{field}.components")));
    }
    Hashed::create(stack, names).map_err(|error| {
        use sunrise_protocol::item::StackHashError;
        component_error(match error {
            StackHashError::Component(id) => format!("{field}.components.{id}"),
            _ => format!("{field}.components"),
        })
    })
}

#[cfg(test)]
#[path = "mod_packet_builders_tests.rs"]
mod tests;
