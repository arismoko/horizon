//! A movement report the mods may veto (`player_move`): held on the guest, ruled on by the game.
//!
//! With a listener, a validated report that changes the player's position or look is not committed
//! in the poll. The hub keeps it on the guest, tells the game ([`HostEvent::PlayerMove`]) and stops
//! reading that guest's inbox, so the player's later reports stay in order behind it; the game rules
//! on it in the same tick and the inbox goes on from there (the turn pauses as it does for a raw
//! payload). The game
//! fires the hook with nothing of the hub borrowed and calls [`Hub::resolve_move`]: commit, or drop
//! and correct the client to the accepted position and look. With no listener,
//! [`Hub::accept_move`] commits at once. Either way, the game completes the effects of an
//! accepted move before the inbox resumes.
//!
//! Not heard, and committed in place: a player's first tick in a level (the tick after a join or a
//! level change), the confirmation of a teleport, and a passage crossing. A report that arrives
//! while a teleport is unconfirmed never gets here (the hub only applies its rotation), so the
//! correction a veto sends is not heard either.

use super::{HostEvent, Hub};
use crate::lan::guest::GuestMove;

/// How many ticks a report may stay pending, the tick it was held at the first of them: it can be
/// ruled on up to the 20th tick, and the 21st finds it out of time, dropped with the correction and
/// never committed.
pub const MOVE_PENDING_TICKS: u64 = 20;

/// Whether the mods hear a validated report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hearing {
    /// A report from the player's own movement: heard when a mod listens and it changes something.
    Heard,
    /// The confirmation of a teleport: validated and committed in place. The last reported
    /// location stays the teleport's destination.
    Confirmation,
    /// A passage crossing, whose caller reads the position at once: committed in place.
    Crossing,
}

/// A movement report that passed the server's own checks (`handlePlayerPositionChange`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Accepted {
    pub to: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub flags: u8,
    /// The box rests on something at `to`.
    pub stands: bool,
    /// `clientIsFloating`, from this report.
    pub floating: bool,
}

/// An [`Accepted`] report waiting for the mods, and what it was accepted against.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingMove {
    pub report: Accepted,
    /// The hub tick it was held at.
    pub since: u64,
    /// The guest's teleport counter and level then: a teleport or a level change since drops it.
    pub teleports: i32,
    pub dimension: String,
    /// The last reported location when it was held: what the hook is told it moves from.
    pub from: ([f64; 3], [f32; 2]),
    /// The hub paused the guest's turn for the ruling already (once: a report nobody rules on does
    /// not pause it again, so it cannot stall the tick).
    pub paused: bool,
}

/// What the hook is told: the last reported location and the report.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoveReport {
    pub player: u128,
    pub from: [f64; 3],
    pub from_rotation: [f32; 2],
    pub to: [f64; 3],
    pub to_rotation: [f32; 2],
}

/// The committed position and the pre-move state needed by the game's movement effects.
#[derive(Clone, Copy, Debug)]
pub struct CommittedMove {
    pub movement: GuestMove,
    pub horizontal_collision: bool,
    pub from: [f64; 3],
    pub jump_yaw: Option<f32>,
}

impl Hub {
    /// Commits the validated report `m` of guest `i`, or holds it for the mods when one listens, it
    /// is `Heard` and it changes the position or the look. A passenger never gets here (its vehicle
    /// carries it).
    pub(super) fn accept_move(
        &mut self,
        i: usize,
        m: Accepted,
        hearing: Hearing,
        events: &mut Vec<HostEvent>,
    ) {
        if hearing == Hearing::Heard && self.moves_listened && self.move_changes(i, &m) {
            let (since, conn) = (self.ticks, self.guests[i].conn);
            let (teleports, dimension) = {
                let g = &self.guests[i];
                (g.teleport_counter, g.dimension.clone())
            };
            let held = PendingMove {
                report: m,
                since,
                teleports,
                dimension,
                from: self.guests[i].last_reported,
                paused: false,
            };
            // The report is the new baseline before the listener runs.
            let heard = (m.to, [m.yaw, m.pitch.clamp(-90.0, 90.0)]);
            self.guests.update(i, |g| {
                g.last_reported = heard;
                g.pending_move = Some(held);
            });
            events.push(HostEvent::PlayerMove(conn));
            return;
        }
        let committed = self.commit_move(i, m);
        if self.dispatch_move_effects {
            events.push(HostEvent::AcceptedMove(self.guests[i].conn, committed));
        }
        // Committed in place: the baseline follows it, except on a teleport's confirmation, which
        // leaves it at the teleport's destination.
        if hearing != Hearing::Confirmation {
            let seen = (m.to, [m.yaw, m.pitch.clamp(-90.0, 90.0)]);
            self.guests.update(i, |g| g.last_reported = seen);
        }
    }

    /// The report differs from the last reported location, and is not of the player's first tick
    /// in its level: the tick after it entered (a join, a level change) is not heard, the next is.
    fn move_changes(&self, i: usize, m: &Accepted) -> bool {
        let g = &self.guests[i];
        self.ticks > g.level_entered + 1
            && (m.to != g.last_reported.0
                || [m.yaw, m.pitch.clamp(-90.0, 90.0)] != g.last_reported.1)
    }

    /// `handlePlayerPositionChange`'s commit: the report becomes the player's state.
    fn commit_move(&mut self, i: usize, m: Accepted) -> CommittedMove {
        let Accepted {
            to,
            yaw,
            pitch,
            flags,
            stands,
            floating,
        } = m;
        self.guests.update(i, |g| {
            g.client_is_floating = floating;
            // Entity.move's verticalCollisionBelow, which the check above reads of the last move.
            g.rests_on_ground = stands;
            g.accept_move(to);
            // ServerChunkCache.move: its pairs are checked again.
            g.moved = true;
            let from = g.previous_position;
            let moved = [0, 1, 2].map(|k| g.position[k] - from[k]);
            if g.moves.is_empty() {
                g.moves_from_ground = g.on_ground;
            }
            let on_ground = flags & 1 != 0;
            let jump_yaw =
                (g.on_ground && !on_ground && moved[1] > 0.0 && !g.spectator()).then_some(g.yaw);
            let movement = GuestMove {
                position: g.position,
                delta: moved,
                on_ground,
                sneaking: g.sneaking,
            };
            g.moves.push(movement);
            g.known_movement = moved;
            g.yaw = yaw;
            g.pitch = pitch.clamp(-90.0, 90.0);
            g.head_yaw = g.yaw;
            g.on_ground = on_ground;
            g.ticks = g.ticks.max(1);
            CommittedMove {
                movement,
                horizontal_collision: flags & 2 != 0,
                from,
                jump_yaw,
            }
        })
    }

    /// Whether guest `i`'s inbox waits behind a pending report. A report past its time, or whose
    /// player teleported or changed level since, ends here: dropped, with the correction only when
    /// it simply ran out of time (a teleport has corrected the client itself).
    pub(super) fn holds_for_move(&mut self, i: usize) -> bool {
        let Some(p) = &self.guests[i].pending_move else {
            return false;
        };
        if !self.pending_is_current(i, p) {
            self.guests.update(i, |g| g.pending_move = None);
            return false;
        }
        if self.move_expired(p) {
            self.guests.update(i, |g| g.pending_move = None);
            self.correct_move(i);
            return false;
        }
        true
    }

    /// Whether guest `i`'s turn pauses for the game's ruling on its held report (the turn goes on in
    /// the same tick once the game has ruled), with the credits of what it read handed to `events`.
    /// The first time only: a report nobody rules on does not pause it again.
    pub(super) fn pause_for_move(
        &mut self,
        i: usize,
        credits: &mut Vec<crate::raw_payloads::ingress::ReceiveLease<crate::lan::NetEvent>>,
        events: &mut Vec<HostEvent>,
    ) -> bool {
        let Some(_) = self.guests[i].pending_move.as_ref().filter(|p| !p.paused) else {
            return false;
        };
        self.guests.update(i, |g| {
            if let Some(p) = g.pending_move.as_mut() {
                p.paused = true;
            }
        });
        events.push(HostEvent::InputCredits(std::mem::take(credits)));
        true
    }

    /// The report has been pending for [`MOVE_PENDING_TICKS`] ticks already.
    fn move_expired(&self, p: &PendingMove) -> bool {
        self.ticks.saturating_sub(p.since) >= MOVE_PENDING_TICKS
    }

    fn pending_is_current(&self, i: usize, p: &PendingMove) -> bool {
        let g = &self.guests[i];
        g.teleport_counter == p.teleports && g.dimension == p.dimension
    }

    /// Sends the client back to the accepted position and look (the existing teleport correction).
    fn correct_move(&mut self, i: usize) {
        let at = self.guests[i].position;
        self.resend_teleport(i, Some(at));
    }

    /// What the hook is told of guest `conn`'s pending report, if it has one.
    pub fn pending_move_report(&self, conn: u32) -> Option<MoveReport> {
        let g = self.guest(conn)?;
        let p = g.pending_move.as_ref()?;
        Some(MoveReport {
            player: g.uuid,
            from: p.from.0,
            from_rotation: p.from.1,
            to: p.report.to,
            to_rotation: [p.report.yaw, p.report.pitch.clamp(-90.0, 90.0)],
        })
    }

    /// The mods ruled on guest `conn`'s pending report: `commit` applies it, otherwise (or when the
    /// ruling is past [`MOVE_PENDING_TICKS`]) it is dropped and the client corrected. Nothing happens if the player is gone or has no pending
    /// report any more (a disconnect, a teleport or a level change dropped it).
    pub fn resolve_move(&mut self, conn: u32, commit: bool) -> Option<CommittedMove> {
        let i = self.guests.connection_index(conn)?;
        let p = self.guests.update(i, |g| g.pending_move.take())?;
        if !self.pending_is_current(i, &p) {
            return None;
        }
        // A ruling that comes too late is no ruling: out of time, the report is dropped.
        if !commit || self.move_expired(&p) {
            self.correct_move(i);
            return None;
        }
        Some(self.commit_move(i, p.report))
    }
}
