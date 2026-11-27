//! Crossing a passage within a level (#3775, docs/modding/src/how-to/passages.md): the client
//! moves its own player through (`ClientHost::cross_passage`) and tells its server the passage
//! and where it crossed from; the server checks that move as any move (the hub,
//! `lan/hub/passage_cross.rs`), finds the passage, asks the mods (`player_passage_cross`),
//! works out where the player comes out itself and follows with a position packet that
//! changes nothing, or puts the player back.
use crate::game::ClientGame;
use crate::game::client_side::native_net::PlaySend;
use mod_api::{
    Passage, PassageCrossError as Error, PassageCrossRefusal as Refusal, PassageId, PassageTurn,
    Vec3,
};
use sunrise_core::geometry::Vec3 as Motion;
use sunrise_protocol::packets::{configuration::CustomPayload, play::Serverbound as S};
use sunrise_protocol::passage_sync::{CROSS_CHANNEL, Cross, encode_cross};
use sunrise_sim::lan::hub::passage_cross::{REACH, refusal_named};

/// Where `point` comes out of `p`: as far from the destination, turned, as it is from the
/// opening's centre (Immersive Portals' `Portal.transformPoint`).
fn carried(p: &Passage, point: [f64; 3]) -> [f64; 3] {
    let o = p.origin;
    let d = p.turned(Vec3::new(point[0] - o.x, point[1] - o.y, point[2] - o.z));
    [
        p.destination.x + d.x,
        p.destination.y + d.y,
        p.destination.z + d.z,
    ]
}

/// `v` turned with `p`, as a velocity is (`TeleportationUtil.transformEntityVelocity`).
fn turned(p: &Passage, [x, y, z]: [f64; 3]) -> [f64; 3] {
    let v = p.turned(Vec3::new(x, y, z));
    [v.x, v.y, v.z]
}

/// The yaw `p` adds, in degrees: a clockwise quarter turn takes a look south (0) to west (90).
fn yaw_turn(p: &Passage) -> f32 {
    match p.turn {
        PassageTurn::Clockwise90 => 90.0,
        PassageTurn::Clockwise180 => 180.0,
        PassageTurn::Counterclockwise90 => -90.0,
        _ => 0.0,
    }
}

/// Whether `point` is within [`REACH`] of the nearest point of `p`'s opening (Immersive
/// Portals' `getDistanceToNearestPointInPortal`).
fn within_reach(p: &Passage, point: [f64; 3]) -> bool {
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
    let reach = f64::from(REACH);
    nearest.iter().map(|v| v * v).sum::<f64>() <= reach * reach
}

/// The chunk holding `point`.
#[allow(dead_code)]
fn chunk_of(point: [f64; 3]) -> (i32, i32) {
    (
        (point[0].floor() as i32) >> 4,
        (point[2].floor() as i32) >> 4,
    )
}

impl ClientGame<'_> {
    /// `ClientHost::cross_passage`: the window's player comes out of `passage` with its
    /// feet, its last tick's place, its velocity and its look carried through, and the
    /// server is told the passage and where the feet were; a payload a mod cancels moves
    /// nothing. Until the server's position packet, the next crossing waits.
    pub fn cross_passage(&mut self, passage: PassageId) -> Result<(), Error> {
        if self.client.remote.is_none() {
            return Err(Error::Unavailable);
        }
        if self.common.player.is_none() {
            return Err(Error::NoPlayer);
        }
        if self.common.passages.crossing.is_some() {
            return Err(Error::Pending);
        }
        if self.common.riding.is_some() {
            return Err(Error::Riding);
        }
        let p = *self
            .common
            .passages
            .held
            .get(&passage)
            .ok_or(Error::UnknownPassage)?;
        let Some(player) = self.common.player.as_ref() else {
            return Err(Error::NoPlayer);
        };
        let from = [player.position.x, player.position.y, player.position.z];
        if !within_reach(&p, from) {
            return Err(Error::TooFar { limit: REACH });
        }
        // Sent first: a mod's `client_packet_send` may cancel it, and then nothing moves.
        let data = encode_cross(&Cross {
            id: passage.raw(),
            from,
        });
        let payload = S::CustomPayload(CustomPayload {
            channel: CROSS_CHANNEL.into(),
            data,
        });
        match self.send_play(payload) {
            PlaySend::Admitted => {}
            PlaySend::Cancelled => return Err(Error::Cancelled),
            PlaySend::Unavailable => return Err(Error::Unavailable),
        }
        // The send hook's listeners only read the host: the player is still there.
        let player = self.common.player.as_mut().expect("checked above");
        let [x, y, z] = carried(&p, from);
        player.position = Motion::new(x, y, z);
        let v = player.velocity;
        let [vx, vy, vz] = turned(&p, [v.x, v.y, v.z]);
        player.velocity = Motion::new(vx, vy, vz);
        let o = self.common.previous_position;
        let [ox, oy, oz] = carried(&p, [o.x, o.y, o.z]);
        self.common.previous_position = Motion::new(ox, oy, oz);
        let turn = yaw_turn(&p);
        self.common.yaw += turn;
        self.client.rope_rot_old.0 += turn;
        self.common.passages.crossing = Some(passage);
        self.common.passages.refused = None;
        Ok(())
    }

    /// `ClientHost::last_passage_refusal`.
    pub fn last_passage_refusal(&self) -> Option<(PassageId, Refusal)> {
        self.common.passages.refused
    }

    /// The server refused the window's crossing of passage `id`: the reason is kept for
    /// the mods, and the position packet that follows puts the player back.
    pub(in crate::game) fn remote_passage_refused(&mut self, id: u64, reason: u8) {
        let Some(why) = refusal_named(reason) else {
            log::debug!("Sunrise: a passage refusal this build does not know: {reason}");
            return;
        };
        if self.common.passages.crossing == Some(PassageId::from_raw(id)) {
            self.client.packet_holds.borrow_mut().clear();
        }
        self.common.passages.refused = Some((PassageId::from_raw(id), why));
    }
}

#[cfg(test)]
mod tests {
    use super::{carried, turned, within_reach, yaw_turn};
    use mod_api::{Passage, PassageTurn, Vec3};

    fn near_doorway() -> Passage {
        Passage {
            origin: Vec3::new(0.5, 65.5, 8.5),
            axis_w: Vec3::new(1.0, 0.0, 0.0),
            axis_h: Vec3::new(0.0, 1.0, 0.0),
            width: 2.0,
            height: 3.0,
            destination: Vec3::new(24.5, 65.5, 8.5),
            turn: PassageTurn::Clockwise90,
            destination_level: None,
        }
    }

    #[test]
    fn a_crossing_carries_the_feet_the_motion_and_the_look() {
        let p = near_doorway();
        assert_eq!(carried(&p, [0.5, 65.0, 8.25]), [24.75, 65.0, 8.5]);
        assert_eq!(carried(&p, [1.5, 66.0, 9.0]), [24.0, 66.0, 9.5]);
        assert_eq!(turned(&p, [0.0, -0.0784, 0.25]), [-0.25, -0.0784, 0.0]);
        assert_eq!(yaw_turn(&p), 90.0);
        assert!(within_reach(&p, [0.5, 65.0, 28.5]));
        assert!(!within_reach(&p, [0.5, 65.0, 28.51]));
        assert!(within_reach(&p, [21.5, 65.5, 8.5]));
        assert!(!within_reach(&p, [21.6, 65.5, 8.5]));
        assert!(!within_reach(&p, [13.5, 79.0, 20.5]));
    }
}
