//! Passages within one level (#3775, docs/modding/src/how-to/passages.md): the server's
//! registry a mod fills through `LevelRef::register_passage`, its sync to the Sunrise
//! clients that hold each passage's chunk (`sunrise:passage_sync`, sent with the chunk
//! through the hub's chunk watches), and the copy a client keeps until it forgets the chunk.
use mod_api::{
    DimensionKey, Passage, PassageId, PassageRegistrationError as Refused, PassageTurn, Vec3,
};
use sunrise_level::fast_hash::FxHashMap;
use sunrise_protocol::passage_sync::PassageForm;

/// Passages a level holds at most. A client holds every passage of the chunks it has, so
/// this bounds its copy and what the server sends it whatever the view distance.
pub const MAX_PER_LEVEL: u32 = 256;
/// Passages one chunk holds at most: what the server sends with the chunk, and the
/// passages a client looks through around a moving player (#3775).
pub const MAX_PER_CHUNK: u32 = 8;
/// The longest side of an opening, in blocks: an opening stays within 23 blocks of its
/// centre, so the chunk holding the centre finds it.
pub(in crate::game) const MAX_SIDE: u32 = 32;
/// How far from 1 a unit axis's squared length, and from 0 the axes' dot product, may be.
pub(in crate::game) const AXIS_TOLERANCE: f64 = 1e-6;
/// `ServerGamePacketListenerImpl.clampHorizontal` and `clampVertical`.
pub(in crate::game) const HORIZONTAL_LIMIT: f64 = 3.0e7;
pub(in crate::game) const VERTICAL_LIMIT: f64 = 2.0e7;

/// The passages of the server's levels and their changes still to tell, and on a client
/// the passages its server told it of. Everything goes when a level or the world retires
/// (`retire_level`, `retire_world`); the numbers go on, so an old one is never reused while
/// the game runs.
#[derive(Default)]
pub struct Passages {
    /// The server's passages, by level, then by chunk ([`Passage::chunk`]).
    pub levels: FxHashMap<DimensionKey, LevelPassages>,
    /// Where each of the server's passages is.
    pub index: FxHashMap<PassageId, (DimensionKey, [i32; 2])>,
    /// The last number given; numbers start at 1 and are never reused.
    pub last: u64,
    /// The passages whose change the clients have not all been told, one entry each: a
    /// live passage not told to every client yet, or a removed one some were told of. So it
    /// holds at most the live passages and those told before their removal.
    pub dirty: FxHashMap<PassageId, Dirty>,
    /// A client's copy: what its server told it of, kept with the chunk.
    pub held: FxHashMap<PassageId, Passage>,
    /// A client's crossing that waits for the server's position packet
    /// (`native_passage_cross.rs`).
    pub crossing: Option<PassageId>,
    /// The server's refusal of the client's last crossing, until the next.
    pub refused: Option<(PassageId, mod_api::PassageCrossRefusal)>,
}

#[derive(Default)]
pub struct LevelPassages {
    pub count: u32,
    pub chunks: FxHashMap<[i32; 2], Vec<(PassageId, Passage)>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dirty {
    /// Registered and not told to every client tracking its chunk; `told` once a chunk's
    /// sync gave it to one, so a removal must be told too.
    Added { told: bool },
    /// Removed from this level and chunk after the clients were told of it.
    Removed(DimensionKey, [i32; 2]),
}

/// Why a client did not keep a passage its server sent (`remote_passage_sync`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeldRefusal {
    /// Not a passage the server could have registered.
    Bad(Refused),
    /// Its chunk is not one the client holds.
    ChunkNotHeld,
    /// The client already holds a level's worth (256).
    Full,
}

impl Passages {
    /// Level `key` closed (`LevelEvent.Unload`): its passages and their untold changes go.
    pub fn retire_level(&mut self, key: DimensionKey) {
        if self.levels.remove(&key).is_none() && self.dirty.is_empty() {
            return;
        }
        self.index.retain(|_, (level, _)| *level != key);
        let index = &self.index;
        self.dirty.retain(|id, dirty| match dirty {
            Dirty::Added { .. } => index.contains_key(id),
            Dirty::Removed(level, _) => *level != key,
        });
    }

    /// The world opened, closed or failed: every passage and change goes, the server's and
    /// a client's copy, but not the numbering.
    pub fn retire_world(&mut self) {
        self.levels.clear();
        self.index.clear();
        self.dirty.clear();
        self.held.clear();
        self.crossing = None;
        self.refused = None;
    }

    /// A new hub (the LAN opened again) records the chunks it sends from its start while a
    /// passage lives, so a chunk it sends before the next `flush_passages` carries them.
    pub fn record_on(&self, hub: &mut sunrise_sim::lan::hub::Hub) {
        if !self.index.is_empty() {
            hub.chunk_watches.record_for_passages();
        }
    }

    /// The server's passage `id`, if level `key` holds it.
    pub fn find(&self, key: DimensionKey, id: PassageId) -> Option<Passage> {
        let &(at, chunk) = self.index.get(&id).filter(|(at, _)| *at == key)?;
        let list = self.levels.get(&at)?.chunks.get(&chunk)?;
        list.iter().find(|(held, _)| *held == id).map(|(_, p)| *p)
    }
}

pub fn passage_carried(p: &Passage, point: [f64; 3]) -> [f64; 3] {
    let o = p.origin;
    let d = p.turned(Vec3::new(point[0] - o.x, point[1] - o.y, point[2] - o.z));
    [
        p.destination.x + d.x,
        p.destination.y + d.y,
        p.destination.z + d.z,
    ]
}

pub fn passage_turned(p: &Passage, [x, y, z]: [f64; 3]) -> [f64; 3] {
    let v = p.turned(Vec3::new(x, y, z));
    [v.x, v.y, v.z]
}

pub fn passage_yaw_turn(p: &Passage) -> f32 {
    match p.turn {
        PassageTurn::Clockwise90 => 90.0,
        PassageTurn::Clockwise180 => 180.0,
        PassageTurn::Counterclockwise90 => -90.0,
        _ => 0.0,
    }
}

pub fn passage_within_reach(p: &Passage, point: [f64; 3]) -> bool {
    let o = p.origin;
    let d = [point[0] - o.x, point[1] - o.y, point[2] - o.z];
    let dot = |a: Vec3| d[0] * a.x + d[1] * a.y + d[2] * a.z;
    let w = dot(p.axis_w).clamp(-p.width / 2.0, p.width / 2.0);
    let h = dot(p.axis_h).clamp(-p.height / 2.0, p.height / 2.0);
    let nearest = [0, 1, 2].map(|k| {
        let (aw, ah) = (
            [p.axis_w.x, p.axis_w.y, p.axis_w.z],
            [p.axis_h.x, p.axis_h.y, p.axis_h.z],
        );
        d[k] - aw[k] * w - ah[k] * h
    });
    let reach = f64::from(sunrise_sim::lan::hub::passage_cross::REACH);
    nearest.iter().map(|v| v * v).sum::<f64>() <= reach * reach
}

pub fn passage_chunk_of(point: [f64; 3]) -> (i32, i32) {
    (
        (point[0].floor() as i32) >> 4,
        (point[2].floor() as i32) >> 4,
    )
}

/// The refusal for a passage that is not a rectangle the engine can carry a player through.
pub fn check(p: &Passage) -> Result<(), Refused> {
    let parts = |v: Vec3| [v.x, v.y, v.z];
    let finite = [p.origin, p.axis_w, p.axis_h, p.destination]
        .into_iter()
        .flat_map(parts)
        .chain([p.width, p.height])
        .all(f64::is_finite);
    if !finite {
        return Err(Refused::NotFinite);
    }
    let dot = |a: Vec3, b: Vec3| a.x * b.x + a.y * b.y + a.z * b.z;
    let (w, h) = (p.axis_w, p.axis_h);
    if (dot(w, w) - 1.0).abs() > AXIS_TOLERANCE
        || (dot(h, h) - 1.0).abs() > AXIS_TOLERANCE
        || dot(w, h).abs() > AXIS_TOLERANCE
    {
        return Err(Refused::BadAxes);
    }
    let side = |s: f64| s > 0.0 && s <= f64::from(MAX_SIDE);
    if !side(p.width) || !side(p.height) {
        return Err(Refused::BadSize { max: MAX_SIDE });
    }
    let ends = [(p.origin, w, h), (p.destination, p.turned(w), p.turned(h))];
    for (centre, w, h) in ends {
        for (sw, sh) in [(-0.5, -0.5), (0.5, -0.5), (-0.5, 0.5), (0.5, 0.5)] {
            let (a, b) = (sw * p.width, sh * p.height);
            let corner = [
                centre.x + w.x * a + h.x * b,
                centre.y + w.y * a + h.y * b,
                centre.z + w.z * a + h.z * b,
            ];
            if corner[0].abs() > HORIZONTAL_LIMIT
                || corner[2].abs() > HORIZONTAL_LIMIT
                || corner[1].abs() > VERTICAL_LIMIT
            {
                return Err(Refused::OutsideWorld);
            }
        }
    }
    Ok(())
}

pub(in crate::game) fn turn_index(turn: PassageTurn) -> u8 {
    match turn {
        PassageTurn::Clockwise90 => 1,
        PassageTurn::Clockwise180 => 2,
        PassageTurn::Counterclockwise90 => 3,
        _ => 0,
    }
}

pub fn form(id: PassageId, p: &Passage) -> PassageForm {
    let v = |v: Vec3| [v.x, v.y, v.z];
    PassageForm {
        id: id.raw(),
        origin: v(p.origin),
        axis_w: v(p.axis_w),
        axis_h: v(p.axis_h),
        width: p.width,
        height: p.height,
        destination: v(p.destination),
        turn: turn_index(p.turn),
    }
}

pub fn passage(f: &PassageForm) -> Passage {
    let v = |[x, y, z]: [f64; 3]| Vec3::new(x, y, z);
    Passage {
        origin: v(f.origin),
        axis_w: v(f.axis_w),
        axis_h: v(f.axis_h),
        width: f.width,
        height: f.height,
        destination: v(f.destination),
        turn: match f.turn {
            1 => PassageTurn::Clockwise90,
            2 => PassageTurn::Clockwise180,
            3 => PassageTurn::Counterclockwise90,
            _ => PassageTurn::None,
        },
        destination_level: None,
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    use super::PassageForm;
    #[cfg(test)]
    use super::{Refused, check};
    use mod_api::{Passage, PassageId, PassageTurn, Vec3};
    use std::sync::mpsc;
    use sunrise_protocol::packets::play::Clientbound;
    use sunrise_protocol::passage_sync::Message;

    /// A 2 by 3 doorway facing south at (0.5, 65.5, 8.5) in chunk (0, 0), coming out 100
    /// blocks east, turned a clockwise quarter.
    pub fn doorway() -> Passage {
        Passage {
            origin: Vec3::new(0.5, 65.5, 8.5),
            axis_w: Vec3::new(1.0, 0.0, 0.0),
            axis_h: Vec3::new(0.0, 1.0, 0.0),
            width: 2.0,
            height: 3.0,
            destination: Vec3::new(100.5, 65.5, 8.5),
            turn: PassageTurn::Clockwise90,
            destination_level: None,
        }
    }

    /// The doorway as it travels, numbered `id`.
    pub fn doorway_form(id: u64) -> PassageForm {
        PassageForm {
            id,
            origin: [0.5, 65.5, 8.5],
            axis_w: [1.0, 0.0, 0.0],
            axis_h: [0.0, 1.0, 0.0],
            width: 2.0,
            height: 3.0,
            destination: [100.5, 65.5, 8.5],
            turn: 1,
        }
    }

    /// The play packets a guest was sent, in order; ones this build cannot read are left out.
    pub fn packets(rx: &mpsc::Receiver<sunrise_sim::lan::Out>) -> Vec<Clientbound> {
        use sunrise_protocol::buf::Reader;
        use sunrise_sim::lan::Out;
        let mut packets = Vec::new();
        for out in rx.try_iter() {
            let bodies = match out {
                Out::Packet(p) => vec![p],
                Out::Packets(p) => p,
                Out::Disconnect(_) => panic!("unexpected disconnect"),
            };
            for body in bodies {
                let mut r = Reader::new(&body);
                let id = r.varint().unwrap();
                packets.extend(Clientbound::decode(id, r.rest()).ok());
            }
        }
        packets
    }

    /// The passage messages among `packets`, in order.
    pub fn passage_messages(packets: &[Clientbound]) -> Vec<Message> {
        packets
            .iter()
            .filter_map(|p| match p {
                Clientbound::CustomPayload(p) if p.channel == "sunrise:passage_sync" => {
                    Some(sunrise_protocol::passage_sync::decode(&p.data).unwrap())
                }
                _ => None,
            })
            .collect()
    }

    #[cfg(test)]
    /// The doorway passes; each way of not being a finite rectangle with unit axes at a
    /// right angle, sides in (0, 32] and corners inside vanilla's clamps is refused by name,
    /// at either end.
    #[test]
    fn a_passage_that_is_not_a_finite_rectangle_inside_the_world_is_refused() {
        assert_eq!(check(&doorway()), Ok(()));
        let with = |change: fn(&mut Passage)| {
            let mut p = doorway();
            change(&mut p);
            check(&p)
        };
        assert_eq!(with(|p| p.origin.y = f64::NAN), Err(Refused::NotFinite));
        assert_eq!(with(|p| p.height = f64::INFINITY), Err(Refused::NotFinite));
        assert_eq!(
            with(|p| p.axis_w = Vec3::new(2.0, 0.0, 0.0)),
            Err(Refused::BadAxes)
        );
        assert_eq!(
            with(|p| p.axis_h = Vec3::new(0.6, 0.8, 0.0)),
            Err(Refused::BadAxes)
        );
        assert_eq!(with(|p| p.width = 0.0), Err(Refused::BadSize { max: 32 }));
        assert_eq!(with(|p| p.height = -1.0), Err(Refused::BadSize { max: 32 }));
        assert_eq!(with(|p| p.width = 32.5), Err(Refused::BadSize { max: 32 }));
        assert_eq!(with(|p| p.width = 32.0), Ok(()));
        // The far side's corner: 29,999,999.5 + 1 east is past 30,000,000.
        assert_eq!(
            with(|p| p.destination.x = 29_999_999.5),
            Ok(()),
            "turned a quarter, the width runs north-south there"
        );
        assert_eq!(
            with(|p| {
                p.destination.x = 29_999_999.5;
                p.turn = PassageTurn::None;
            }),
            Err(Refused::OutsideWorld)
        );
        assert_eq!(
            with(|p| p.origin.z = -30_000_000.5),
            Err(Refused::OutsideWorld)
        );
        assert_eq!(
            with(|p| p.origin.y = 20_000_000.0),
            Err(Refused::OutsideWorld)
        );
    }

    #[cfg(any(test, feature = "test-support"))]
    /// Removes the passage in the second `chunk_sent` it hears.
    pub struct RemoveOnSecondSent {
        pub heard: std::sync::Mutex<u32>,
        pub id: PassageId,
    }
    #[cfg(any(test, feature = "test-support"))]
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for RemoveOnSecondSent {
        fn chunk_sent(&self, host: &mut dyn mod_api::ServerHost, e: &mut mod_api::ChunkSentEvent) {
            let mut heard = self.heard.lock().unwrap();
            *heard += 1;
            if *heard == 2 {
                assert!(e.chunk.level().remove_passage(host, self.id));
            }
        }
    }
}
