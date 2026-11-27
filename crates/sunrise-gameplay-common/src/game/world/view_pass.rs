#[cfg(any(test, feature = "test-support"))]
pub mod tests {

    use sunrise_protocol::{
        buf::Reader,
        packets::play::Clientbound as C,
        viewed_levels::{CHANNEL, Level::Index, Message, REVISION_3, ScopedPacket},
    };

    /// What a client was sent of the Nether (level 2) on `sunrise:levels`.
    #[derive(Default)]
    pub struct Sent {
        pub opened: bool,
        pub chunks: Vec<(i32, i32)>,
        pub forgotten: Vec<(i32, i32)>,
        pub closed: bool,
        pub times: Vec<i64>,
        /// The other packets of its runs, in order.
        pub others: Vec<C>,
        pub sounds: Vec<sunrise_protocol::packets::play::Sound>,
    }
    pub fn sent(rx: &std::sync::mpsc::Receiver<sunrise_sim::lan::Out>) -> Sent {
        let mut sent = Sent::default();
        let bodies = rx.try_iter().flat_map(|o| match o {
            sunrise_sim::lan::Out::Packet(body) => vec![body],
            sunrise_sim::lan::Out::Packets(bodies) => bodies,
            sunrise_sim::lan::Out::Disconnect(_) => Vec::new(),
        });
        for body in bodies {
            let mut r = Reader::new(&body);
            let id = r.varint().unwrap();
            let Ok(C::CustomPayload(p)) = C::decode(id, r.rest()) else {
                continue;
            };
            if p.channel != CHANNEL {
                continue;
            }
            match Message::decode_in(&p.data, REVISION_3).unwrap() {
                Message::Open { level, .. } => sent.opened |= level == Index(2),
                Message::Close { level } => sent.closed |= level == Index(2),
                Message::Run {
                    level: Index(2),
                    packets,
                } => {
                    for packet in packets {
                        let ScopedPacket::Play(id, body) = packet else {
                            continue;
                        };
                        match C::decode(id, body) {
                            Ok(C::LevelChunkWithLight(c)) => sent.chunks.push((c.x, c.z)),
                            Ok(C::ForgetLevelChunk(f)) => sent.forgotten.push((f.pos[0], f.pos[1])),
                            Ok(C::SetTime(t)) if t.clocks.is_empty() => {
                                sent.times.push(t.game_time)
                            }
                            Ok(C::Sound(p)) => sent.sounds.push(p),
                            Ok(other) => sent.others.push(other),
                            Err(_) => {}
                        }
                    }
                }
                _ => {}
            }
        }
        sent
    }
    /// The chunks of a radius-1 square at (40, 40).
    pub fn square() -> Vec<(i32, i32)> {
        (39..=41)
            .flat_map(|x| (39..=41).map(move |z| (x, z)))
            .collect()
    }
    /// What the viewer was sent of entity `id` in the Nether's runs: where `AddEntity` put it and
    /// where each move after took it (the client's `VecDeltaCodec`), and whether it was removed.
    #[derive(Debug, Default)]
    pub struct Tracked {
        pub added: Option<(i32, [f64; 3])>,
        pub moves: Vec<[f64; 3]>,
        /// Its entity events (`EntityEvent`).
        pub events: Vec<i8>,
        pub removed: bool,
    }

    pub fn tracked(packets: &[C], id: i32) -> Tracked {
        use sunrise_protocol::packets::play;
        let mut t = Tracked::default();
        let mut base = [0.0; 3];
        for packet in packets {
            match packet {
                C::AddEntity(p) if p.id == id => {
                    t.added = Some((p.kind, p.position));
                    base = p.position;
                }
                C::MoveEntityPos(play::MoveEntityPos(m))
                | C::MoveEntityPosRot(play::MoveEntityPosRot(m))
                    if m.id == id =>
                {
                    base = m.delta.as_ref().unwrap().end(base);
                    t.moves.push(base);
                }
                C::EntityPositionSync(p) if p.id == id => {
                    base = p.path.end();
                    t.moves.push(base);
                }
                C::EntityEvent(p) if p.id == id => t.events.push(p.event),
                C::RemoveEntities(p) if p.ids.contains(&id) => t.removed = true,
                _ => {}
            }
        }
        t
    }
    /// A packet a client was sent: a plain one, or what `sunrise:levels` carried.
    #[derive(Debug)]
    pub enum Got {
        Plain(C),
        Open(i32),
        Wrapped(i32, C),
        Groups(i32, sunrise_protocol::connection_groups::Section),
        Close(i32),
    }
    /// Everything the viewer was sent since the last call, in order, each packet decoded.
    pub fn every(rx: &std::sync::mpsc::Receiver<sunrise_sim::lan::Out>) -> Vec<Got> {
        let bodies = rx.try_iter().flat_map(|o| match o {
            sunrise_sim::lan::Out::Packet(body) => vec![body],
            sunrise_sim::lan::Out::Packets(bodies) => bodies,
            sunrise_sim::lan::Out::Disconnect(_) => Vec::new(),
        });
        decode_ordered(bodies)
    }
    pub fn decode_ordered(bodies: impl IntoIterator<Item = Vec<u8>>) -> Vec<Got> {
        let mut got = Vec::new();
        for body in bodies {
            let mut r = Reader::new(&body);
            let id = r.varint().unwrap();
            let Ok(packet) = C::decode(id, r.rest()) else {
                continue;
            };
            let C::CustomPayload(p) = &packet else {
                got.push(Got::Plain(packet));
                continue;
            };
            if p.channel != CHANNEL {
                continue;
            }
            match Message::decode_in(&p.data, REVISION_3).unwrap() {
                Message::Open { level, .. } => got.push(Got::Open(level.number())),
                Message::Close { level } => got.push(Got::Close(level.number())),
                Message::Run { level, packets } => {
                    got.extend((packets.into_iter()).filter_map(|packet| {
                        match packet {
                            ScopedPacket::Play(id, body) => C::decode(id, body)
                                .ok()
                                .map(|packet| Got::Wrapped(level.number(), packet)),
                            ScopedPacket::ConnectionGroups(body) => Some(Got::Groups(
                                level.number(),
                                sunrise_protocol::connection_groups::Section::decode(body).unwrap(),
                            )),
                        }
                    }))
                }
                Message::Swap { .. } => unreachable!("no swap below revision 4"),
                Message::Far { .. } => unreachable!("no far below revision 5"),
            }
        }
        got
    }
    /// The Respawns the viewer was sent: their level and what they keep.
    pub fn respawns(got: &[Got]) -> Vec<(String, i8)> {
        (got.iter())
            .filter_map(|g| match g {
                Got::Plain(C::Respawn(p)) => Some((p.spawn.dimension.clone(), p.keep)),
                _ => None,
            })
            .collect()
    }
}
