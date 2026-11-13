//! Viewed levels (#3716): a client that keeps levels besides its own, so that a server can send it
//! the chunks and updates of another level, each packet wrapped with that level. One reserved
//! engine channel carries it, [`CHANNEL`], used by both sides.
//!
//! **Configuration** (both directions): the server sends a client whose `minecraft:brand` is
//! Horizon's a query, [`query`] (`[0, REVISION]`). A client that knows the channel answers with
//! [`answer`] (`[1, REVISION, max_levels]`) before it acknowledges `FinishConfiguration`:
//! `max_levels` is how many levels besides its own it can view. No answer, or one of a revision
//! the server does not serve, means none. A vanilla client is never asked.
//!
//! **Revisions.** Each side asks and answers with its own, [`REVISION`]. A server serves every
//! revision from [`OLDEST_REVISION`] to its own and keeps what the client answered ([`Answer`]),
//! so that it gives a client only what that client's revision knows. A client of an older server
//! is not served: an older server reads a later revision's answer as none.
//!
//! | Revision | Feature | Introduced by |
//! |---|---|---|
//! | 1 | Viewed levels, stages 9a–9d: channel and play payloads below | #3716 |
//! | 2 ([`REVISION_2`]) | Switch into a viewed level on Respawn (keep 3), retaining the level left as viewed ([`Answer::switches`]) | #4004 |
//! | 3 ([`REVISION_3`]) | Viewed levels' sounds ([`SOUNDS`], [`Answer::sounds`]) | #3983 |
//!
//! Pending acts (#3982) need revision 4, not revision 2 or 3. A new message kind is
//! accepted only at its own revision or later; its decoder must land before any sender enables it.
//!
//! **Play** (clientbound only): the payload after the channel is a kind byte, then:
//!
//! | Kind | Bytes | Meaning |
//! |---|---|---|
//! | 0 `open` | VarInt level, `SpawnInfo` (Respawn's codec) | The client makes a world for the level. |
//! | 1 `run` | VarInt level, then one or more entries: VarInt length, then one play clientbound packet as `Clientbound::encode` writes it | Packets of that level, in order. |
//! | 2 `close` | VarInt level | Every chunk of the level was forgotten: the client drops the world. |
//!
//! The level is its index in the Login packet's `levels` list, never the player's own. Only the
//! [`LEVEL_SCOPED`] kinds and, from revision 3, the [`SOUNDS`] may be wrapped, and a run never
//! mixes levels. A serverbound play payload on the channel is a protocol error
//! (`custom_payload`'s codec refuses it).
use crate::{
    buf::{Codec, Reader, Writer},
    ids::play::clientbound as id,
    packets::play::{Clientbound, SpawnInfo},
};

/// The reserved engine channel (`custom_payload::is_reserved_channel`).
pub const CHANNEL: &str = "horizon:levels";
/// Revision 2 of the exchange: what a client of it does besides revision 1's is in the module's
/// list of revisions.
pub const REVISION_2: u8 = 2;
/// Revision 3 of the exchange (#3983): see the module's list of revisions.
pub const REVISION_3: u8 = 3;
/// The capability exchange's revision this build asks and answers.
pub const REVISION: u8 = REVISION_3;
/// The oldest revision a server serves.
pub const OLDEST_REVISION: u8 = 1;

const QUERY: u8 = 0;
const ANSWER: u8 = 1;
const OPEN: u8 = 0;
const RUN: u8 = 1;
const CLOSE: u8 = 2;

/// The play clientbound packets a `run` may carry: the ones about one level's chunks, blocks,
/// time and weather, and its entities. Two are narrowed further by their fields: `GameEvent` only
/// for rain and thunder (events 1, 2, 7 and 8), `LevelEvent` only when not global. Never wrapped:
/// anything about the player or the connection, chunk batches, the chunk cache centre and radius,
/// global level events, chat, sounds (wrapped from revision 3: [`SOUNDS`]) and particles.
pub const LEVEL_SCOPED: [i32; 30] = [
    id::LEVEL_CHUNK_WITH_LIGHT,
    id::FORGET_LEVEL_CHUNK,
    id::LIGHT_UPDATE,
    id::CHUNKS_BIOMES,
    id::BLOCK_UPDATE,
    id::SECTION_BLOCKS_UPDATE,
    id::BLOCK_ENTITY_DATA,
    id::BLOCK_EVENT,
    id::BLOCK_DESTRUCTION,
    id::LEVEL_EVENT,
    id::SET_TIME,
    id::GAME_EVENT,
    id::ADD_ENTITY,
    id::MOVE_ENTITY_POS,
    id::MOVE_ENTITY_POS_ROT,
    id::MOVE_ENTITY_ROT,
    id::ROTATE_HEAD,
    id::ENTITY_POSITION_SYNC,
    id::TELEPORT_ENTITY,
    id::SET_ENTITY_DATA,
    id::SET_EQUIPMENT,
    id::UPDATE_ATTRIBUTES,
    id::SET_ENTITY_MOTION,
    id::SET_PASSENGERS,
    id::SET_ENTITY_LINK,
    id::ENTITY_EVENT,
    id::ANIMATE,
    id::REMOVE_ENTITIES,
    id::UPDATE_MOB_EFFECT,
    id::REMOVE_MOB_EFFECT,
];

/// The play clientbound packets a `run` may also carry from [`REVISION_3`] on: the level's sounds
/// at a position and bound to an entity. A server sends them only to a client whose answer
/// [`Answer::sounds`]; below that revision they are a protocol error.
pub const SOUNDS: [i32; 2] = [id::SOUND, id::SOUND_ENTITY];

/// `GameEvent`s about a level's weather: START_RAINING, STOP_RAINING, RAIN_LEVEL_CHANGE and
/// THUNDER_LEVEL_CHANGE.
const WEATHER_EVENTS: [u8; 4] = [1, 2, 7, 8];

/// The most entities one viewed level holds on its client: a server tracks no more for one view,
/// and a client sent an `AddEntity` of a new one past it closes the connection. A fixed engine
/// limit of this revision, so that a server cannot fill a client's memory through one level.
pub const MAX_ENTITIES: usize = 8192;

/// Why a `horizon:levels` payload closes the connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewedLevelsError {
    /// A kind byte that is not in the table for its phase and direction.
    UnknownKind(u8),
    /// A wrapped packet of a kind that is not level-scoped: its play clientbound id.
    NotLevelScoped(i32),
    /// A wrapped packet whose bytes its play clientbound codec does not read exactly (cut short or
    /// with bytes left over): its id.
    MalformedPacket(i32),
    /// A `run` with no packet.
    EmptyRun,
    /// Bytes cut short, or not what their codec reads.
    CutShort,
    /// Bytes left over after the message.
    LeftOver,
    /// An answer the server did not ask for.
    UnaskedAnswer,
    /// A second answer in one configuration phase.
    SecondAnswer,
}

impl std::fmt::Display for ViewedLevelsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{CHANNEL}: ")?;
        match self {
            Self::UnknownKind(kind) => write!(f, "unknown kind {kind}"),
            Self::NotLevelScoped(id) => write!(f, "packet {id} is not level-scoped"),
            Self::MalformedPacket(id) => {
                write!(f, "packet {id} is cut short or has bytes left over")
            }
            Self::EmptyRun => f.write_str("a run with no packet"),
            Self::CutShort => f.write_str("bytes cut short"),
            Self::LeftOver => f.write_str("bytes left over"),
            Self::UnaskedAnswer => f.write_str("an answer the server did not ask for"),
            Self::SecondAnswer => f.write_str("a second answer"),
        }
    }
}

impl std::error::Error for ViewedLevelsError {}

impl From<crate::Error> for ViewedLevelsError {
    fn from(_: crate::Error) -> Self {
        Self::CutShort
    }
}

/// The server's query, configuration clientbound: the custom payload's data.
pub fn query() -> Vec<u8> {
    vec![QUERY, REVISION]
}

/// Reads the server's query. A query of another revision is still answered (with this one's), so
/// its bytes after the revision are not read.
pub fn read_query(data: &[u8]) -> Result<(), ViewedLevelsError> {
    let mut r = Reader::new(data);
    expect_kind(&mut r, QUERY)?;
    if r.u8()? == REVISION {
        end(&r)?;
    }
    Ok(())
}

/// The client's answer, configuration serverbound: it can view `max_levels` levels besides its
/// own.
pub fn answer(max_levels: u8) -> Vec<u8> {
    vec![ANSWER, REVISION, max_levels]
}

/// What a client answered: the revision it speaks and how many levels besides its own it can
/// view. The default is no answer: none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Answer {
    pub revision: u8,
    pub max_levels: u8,
}

impl Answer {
    /// Whether the client swaps worlds on a travel into a level it views ([`REVISION_2`]).
    pub fn switches(self) -> bool {
        self.revision >= REVISION_2
    }
    /// Whether the client may be sent its viewed levels' sounds ([`SOUNDS`], [`REVISION_3`]).
    pub fn sounds(self) -> bool {
        self.revision >= REVISION_3
    }
}

/// Reads a client's answer. An answer of a revision outside [`OLDEST_REVISION`] to [`REVISION`]
/// means none, and its bytes after the revision are not read.
pub fn read_answer(data: &[u8]) -> Result<Answer, ViewedLevelsError> {
    let mut r = Reader::new(data);
    expect_kind(&mut r, ANSWER)?;
    let revision = r.u8()?;
    if !(OLDEST_REVISION..=REVISION).contains(&revision) {
        return Ok(Answer::default());
    }
    let max_levels = r.u8()?;
    end(&r)?;
    Ok(Answer {
        revision,
        max_levels,
    })
}

/// A play payload, read.
#[derive(Clone, Debug, PartialEq)]
pub enum Message<'a> {
    /// The client makes a world for `level`.
    Open { level: i32, spawn: SpawnInfo },
    /// Packets of `level`, in order: each its play clientbound id and the bytes after it, for
    /// `play::Clientbound::decode`. Every one is level-scoped and read exactly by its codec.
    Run {
        level: i32,
        packets: Vec<(i32, &'a [u8])>,
    },
    /// Every chunk of `level` was forgotten: the client drops the world.
    Close { level: i32 },
}

impl<'a> Message<'a> {
    /// Reads a play payload's data (after its channel), at this build's [`REVISION`].
    pub fn decode(data: &'a [u8]) -> Result<Self, ViewedLevelsError> {
        Self::decode_in(data, REVISION)
    }

    /// Reads a play payload's data at `revision`: a kind that revision does not have, such as a
    /// sound below [`REVISION_3`], is refused as not level-scoped.
    pub fn decode_in(data: &'a [u8], revision: u8) -> Result<Self, ViewedLevelsError> {
        Self::decode_with(data, revision, true)
    }

    /// [`Message::decode_in`], keeping a run's [`SOUNDS`] only when `sounds`: otherwise each one's
    /// kind is checked against `revision` and its bytes are skipped unread, so a run of sounds
    /// nobody hears decodes and allocates nothing. A run of only such sounds is not empty: it
    /// comes back with no packet.
    pub fn decode_with(
        data: &'a [u8],
        revision: u8,
        sounds: bool,
    ) -> Result<Self, ViewedLevelsError> {
        let mut r = Reader::new(data);
        let kind = r.u8()?;
        let level = r.varint()?;
        let message = match kind {
            OPEN => Message::Open {
                level,
                spawn: <SpawnInfo as Codec<SpawnInfo>>::read(&mut r)?,
            },
            RUN => {
                let mut packets = Vec::new();
                let mut entries = 0;
                while r.remaining() > 0 {
                    let len =
                        usize::try_from(r.varint()?).map_err(|_| ViewedLevelsError::CutShort)?;
                    let mut entry = Reader::new(r.bytes(len)?);
                    let id = entry.varint()?;
                    let body = entry.rest();
                    entries += 1;
                    if !sounds && SOUNDS.contains(&id) {
                        if revision < REVISION_3 {
                            return Err(ViewedLevelsError::NotLevelScoped(id));
                        }
                        continue;
                    }
                    check_level_scoped(id, body, revision)?;
                    packets.push((id, body));
                }
                if entries == 0 {
                    return Err(ViewedLevelsError::EmptyRun);
                }
                Message::Run { level, packets }
            }
            CLOSE => Message::Close { level },
            other => return Err(ViewedLevelsError::UnknownKind(other)),
        };
        end(&r)?;
        Ok(message)
    }
}

/// The whole body of the play custom payload packet that opens `level` on the client, to send as
/// it is.
pub fn open(level: i32, spawn: &SpawnInfo) -> Vec<u8> {
    let mut w = head(OPEN, level);
    <SpawnInfo as Codec<SpawnInfo>>::write(spawn, &mut w);
    w.data
}

/// The whole body of the play custom payload packet that closes `level` on the client.
pub fn close(level: i32) -> Vec<u8> {
    head(CLOSE, level).data
}

/// A `run` being written: packets of one level, each as `Clientbound::encode` gives it, wrapped
/// without being decoded again.
pub struct Run {
    w: Writer,
}

impl Run {
    /// A run of `level` that starts with `packet`.
    pub fn new(level: i32, packet: &[u8]) -> Result<Self, ViewedLevelsError> {
        let mut run = Self {
            w: head(RUN, level),
        };
        run.push(packet)?;
        Ok(run)
    }

    /// Adds `packet` (a play clientbound packet as `Clientbound::encode` writes it), refusing one
    /// that is not level-scoped or that its codec does not read exactly.
    pub fn push(&mut self, packet: &[u8]) -> Result<(), ViewedLevelsError> {
        let mut r = Reader::new(packet);
        let id = r.varint()?;
        check_level_scoped(id, r.rest(), REVISION)?;
        let len = i32::try_from(packet.len()).map_err(|_| ViewedLevelsError::CutShort)?;
        self.w.varint(len);
        self.w.bytes(packet);
        Ok(())
    }

    /// The whole body of the play custom payload packet, to send as it is.
    pub fn finish(self) -> Vec<u8> {
        self.w.data
    }
}

/// The start of a play payload: the custom payload packet's id and channel, the kind and the level.
fn head(kind: u8, level: i32) -> Writer {
    let mut w = Writer::new();
    w.varint(id::CUSTOM_PAYLOAD);
    w.string(CHANNEL);
    w.u8(kind);
    w.varint(level);
    w
}

/// Refuses a packet (its id and the bytes after it) that is not level-scoped at `revision`, or
/// that its play clientbound codec does not read exactly.
fn check_level_scoped(id: i32, body: &[u8], revision: u8) -> Result<(), ViewedLevelsError> {
    let sound = revision >= REVISION_3 && SOUNDS.contains(&id);
    if !LEVEL_SCOPED.contains(&id) && !sound {
        return Err(ViewedLevelsError::NotLevelScoped(id));
    }
    let scoped =
        match Clientbound::decode(id, body).map_err(|_| ViewedLevelsError::MalformedPacket(id))? {
            Clientbound::GameEvent(e) => WEATHER_EVENTS.contains(&e.event),
            Clientbound::LevelEvent(e) => !e.global,
            _ => true,
        };
    if scoped {
        Ok(())
    } else {
        Err(ViewedLevelsError::NotLevelScoped(id))
    }
}

fn expect_kind(r: &mut Reader, kind: u8) -> Result<(), ViewedLevelsError> {
    match r.u8()? {
        k if k == kind => Ok(()),
        other => Err(ViewedLevelsError::UnknownKind(other)),
    }
}

fn end(r: &Reader) -> Result<(), ViewedLevelsError> {
    if r.remaining() > 0 {
        return Err(ViewedLevelsError::LeftOver);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packets::{
        custom_payload::CustomPayload,
        play::{self, GlobalPos},
    };

    fn nether() -> SpawnInfo {
        SpawnInfo {
            dimension_type: 3,
            dimension: "minecraft:the_nether".into(),
            seed: -0x1234_5678_9abc_def0,
            game_type: 1,
            previous_game_type: None,
            debug: false,
            flat: false,
            last_death: Some(GlobalPos {
                dimension: "minecraft:overworld".into(),
                pos: [4, 70, -9],
            }),
            portal_cooldown: 0,
            sea_level: 32,
        }
    }

    /// What a client's net thread has of a play packet off the wire: the custom payload decoded,
    /// then its data.
    fn received(body: &[u8]) -> CustomPayload {
        let mut r = Reader::new(body);
        let packet_id = r.varint().unwrap();
        let Clientbound::CustomPayload(p) = Clientbound::decode(packet_id, r.rest()).unwrap()
        else {
            panic!("not a custom payload");
        };
        assert_eq!(p.channel, "horizon:levels");
        p
    }

    #[test]
    fn open_run_and_close_come_back_as_sent() {
        let open = received(&open(2, &nether()));
        assert_eq!(
            Message::decode(&open.data).unwrap(),
            Message::Open {
                level: 2,
                spawn: nether()
            }
        );
        let sent = [
            Clientbound::BlockUpdate(play::BlockUpdate {
                pos: [12, 64, -7],
                state: 1,
            }),
            Clientbound::GameEvent(play::GameEvent {
                event: 7,
                param: 0.5,
            }),
            Clientbound::ForgetLevelChunk(play::ForgetLevelChunk { pos: [0, -1] }),
        ];
        let mut run = Run::new(2, &sent[0].encode()).unwrap();
        run.push(&sent[1].encode()).unwrap();
        run.push(&sent[2].encode()).unwrap();
        let run = received(&run.finish());
        let Message::Run { level, packets } = Message::decode(&run.data).unwrap() else {
            panic!("not a run");
        };
        assert_eq!(level, 2);
        let decoded: Vec<_> = packets
            .into_iter()
            .map(|(packet_id, body)| Clientbound::decode(packet_id, body).unwrap())
            .collect();
        assert_eq!(decoded, sent);
        let close = received(&close(1));
        assert_eq!(close.data, [2, 1]);
        assert_eq!(
            Message::decode(&close.data).unwrap(),
            Message::Close { level: 1 }
        );
    }

    #[test]
    fn a_packet_that_is_not_level_scoped_is_refused() {
        let health = Clientbound::SetHealth(play::SetHealth {
            health: 20.0,
            food: 20,
            saturation: 5.0,
        })
        .encode();
        let change_game_mode = Clientbound::GameEvent(play::GameEvent {
            event: 3,
            param: 1.0,
        })
        .encode();
        let global = Clientbound::LevelEvent(play::LevelEvent {
            kind: 1023,
            pos: [0, 64, 0],
            data: 0,
            global: true,
        })
        .encode();
        // Run kind, level 1, one entry, written by hand as a server that skips the check would.
        let wrap = |packet: &[u8]| [&[1, 1, packet.len() as u8][..], packet].concat();
        for (packet, packet_id) in [(health, 106), (change_game_mode, 39), (global, 47)] {
            assert_eq!(
                Message::decode(&wrap(&packet)),
                Err(ViewedLevelsError::NotLevelScoped(packet_id))
            );
            assert_eq!(
                Run::new(1, &packet).err(),
                Some(ViewedLevelsError::NotLevelScoped(packet_id))
            );
        }
    }

    #[test]
    fn a_wrapped_packet_cut_short_or_with_bytes_left_over_is_refused() {
        // Run kind, level 0, then one entry: its length, the packet id, the packet's bytes.
        // A block update (8) with no position or state.
        assert_eq!(
            Message::decode(&[1, 0, 1, 8]),
            Err(ViewedLevelsError::MalformedPacket(8))
        );
        // A rain level change (game event 39, event 7) with no float parameter.
        assert_eq!(
            Message::decode(&[1, 0, 2, 39, 7]),
            Err(ViewedLevelsError::MalformedPacket(39))
        );
        // A whole block update at (0, 0, 0) with state 1, then one byte left over.
        assert_eq!(
            Message::decode(&[1, 0, 11, 8, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0]),
            Err(ViewedLevelsError::MalformedPacket(8))
        );
        // A whole rain level change of 0.5, then one byte left over.
        assert_eq!(
            Message::decode(&[1, 0, 7, 39, 7, 0x3f, 0, 0, 0, 9]),
            Err(ViewedLevelsError::MalformedPacket(39))
        );
        for packet in [
            &[8][..],
            &[39, 7],
            &[8, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0],
            &[39, 7, 0x3f, 0, 0, 0, 9],
        ] {
            assert_eq!(
                Run::new(0, packet).err(),
                Some(ViewedLevelsError::MalformedPacket(i32::from(packet[0])))
            );
        }
    }

    #[test]
    fn the_wire_bytes_of_open_run_and_close() {
        let channel = b"horizon:levels";
        let head = |kind: u8, level: u8| [&[24, 14][..], channel, &[kind, level]].concat();
        let the_end = SpawnInfo {
            dimension_type: 0,
            dimension: "minecraft:the_end".into(),
            seed: 0,
            game_type: 0,
            previous_game_type: None,
            debug: false,
            flat: false,
            last_death: None,
            portal_cooldown: 0,
            sea_level: 0,
        };
        // SpawnInfo: type 0, the dimension's name, a zero seed, then game type, no previous
        // game type, not debug, not flat, no last death, cooldown 0 and sea level 0.
        let spawn = [&[0, 17][..], b"minecraft:the_end", &[0; 8], &[0; 7]].concat();
        assert_eq!(open(1, &the_end), [head(0, 1), spawn].concat());
        let block_update = [8, 0, 0, 0, 0, 0, 0, 0, 0, 1];
        let rain = [39, 7, 0x3f, 0, 0, 0];
        let mut run = Run::new(2, &block_update).unwrap();
        run.push(&rain).unwrap();
        let wire = [
            head(1, 2),
            vec![10],
            block_update.to_vec(),
            vec![6],
            rain.to_vec(),
        ]
        .concat();
        assert_eq!(run.finish(), wire);
        assert_eq!(close(3), head(2, 3));
    }

    #[test]
    fn a_play_payload_cut_short_left_over_or_of_another_kind_is_refused() {
        assert_eq!(Message::decode(&[]), Err(ViewedLevelsError::CutShort));
        assert_eq!(Message::decode(&[1, 3]), Err(ViewedLevelsError::EmptyRun));
        assert_eq!(
            Message::decode(&[1, 3, 9, 8]),
            Err(ViewedLevelsError::CutShort)
        );
        assert_eq!(
            Message::decode(&[2, 1, 0]),
            Err(ViewedLevelsError::LeftOver)
        );
        assert_eq!(
            Message::decode(&[3, 1]),
            Err(ViewedLevelsError::UnknownKind(3))
        );
    }

    #[test]
    fn answers_read_as_their_revision_and_level_count_or_none() {
        let answered = |revision, max_levels| {
            Ok(Answer {
                revision,
                max_levels,
            })
        };
        assert_eq!(query(), [0, 3]);
        assert_eq!(read_query(&[0, 3]), Ok(()));
        assert_eq!(read_query(&[0, 2]), Ok(()), "revision 2's query");
        assert_eq!(
            read_query(&[0, 4, 7, 7]),
            Ok(()),
            "a later revision's query"
        );
        assert_eq!(read_query(&[0, 1]), Ok(()), "an older revision's query");
        assert_eq!(read_query(&[0, 3, 7]), Err(ViewedLevelsError::LeftOver));
        assert_eq!(answer(2), [1, 3, 2]);
        assert_eq!(read_answer(&[1, 2, 2]), answered(2, 2));
        assert!(read_answer(&[1, 2, 2]).unwrap().switches());
        let older = read_answer(&[1, 1, 2]);
        assert_eq!(older, answered(1, 2), "revision 1 is still served");
        assert!(!older.unwrap().switches());
        assert!(read_answer(&[1, 3, 2]).unwrap().sounds());
        assert!(read_answer(&[1, 3, 2]).unwrap().switches());
        assert!(
            !read_answer(&[1, 2, 2]).unwrap().sounds(),
            "revision 2: no sounds"
        );
        assert!(!older.unwrap().sounds());
        for revision in [0, 4] {
            assert_eq!(
                read_answer(&[1, revision, 5, 9]),
                Ok(Answer::default()),
                "revision {revision} views none"
            );
        }
        assert_eq!(read_answer(&[1, 3]), Err(ViewedLevelsError::CutShort));
        assert_eq!(read_answer(&[0, 2]), Err(ViewedLevelsError::UnknownKind(0)));
    }

    #[test]
    fn a_run_carries_a_sound_from_revision_3_and_below_it_is_refused() {
        let sound = Clientbound::Sound(play::Sound {
            sound: (5, None),
            source: 4,
            x: 80,
            y: 520,
            z: -24,
            volume: 2.0,
            pitch: 0.5,
            seed: 7,
        });
        let run = received(&Run::new(1, &sound.encode()).unwrap().finish());
        let Message::Run { level, packets } = Message::decode(&run.data).unwrap() else {
            panic!("not a run");
        };
        assert_eq!(level, 1);
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].0, 119);
        assert_eq!(Clientbound::decode(119, packets[0].1).unwrap(), sound);
        // A sound bound to an entity: id 118, holder 6 (registry id 5), source 4, entity 9,
        // volume 1.0, pitch 1.0, seed 0.
        let bound = [
            118, 6, 4, 9, 0x3f, 0x80, 0, 0, 0x3f, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        assert!(Run::new(1, &bound).is_ok());
        // Below revision 3 either one is a protocol error, heard or not.
        let bound = Run::new(1, &bound).unwrap().finish();
        for (data, id) in [(&run.data, 119), (&received(&bound).data, 118)] {
            assert!(Message::decode_in(data, 3).is_ok());
            for revision in [1, 2] {
                for sounds in [true, false] {
                    assert_eq!(
                        Message::decode_with(data, revision, sounds),
                        Err(ViewedLevelsError::NotLevelScoped(id))
                    );
                }
            }
        }
        // Not heard: skipped unread, even cut short; the block update beside it is kept.
        let block = [8, 0, 0, 0, 0, 0, 0, 0, 0, 1];
        let wire = [&[1, 1, 2, 119, 0][..], &[10], &block].concat();
        assert_eq!(
            Message::decode_with(&wire, 3, false),
            Ok(Message::Run {
                level: 1,
                packets: vec![(8, &block[1..])]
            })
        );
        assert!(
            Message::decode_with(&wire, 3, true).is_err(),
            "heard, it is read"
        );
        assert_eq!(
            Message::decode_with(&run.data, 3, false),
            Ok(Message::Run {
                level: 1,
                packets: Vec::new()
            })
        );
    }
}
