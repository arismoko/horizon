//! `set_local_rotation`: a client mod turns the local player's view; the look goes out in the
//! tick's own player sync, one move packet with vanilla's clamp, never a packet of the call's own.
use sunrise_protocol::connection::Connection;
use sunrise_protocol::packets::play::Serverbound as S;

/// The move packets the client sent before a fence, as (kind, yaw, pitch).
#[cfg(feature = "client")]
pub(crate) fn sent_moves(
    game: &sunrise_client_gameplay::game::ClientGame<'_>,
    server: &mut Connection,
) -> Vec<(&'static str, f32, f32)> {
    game.send(S::Punch(sunrise_protocol::packets::Empty));
    let mut moves = Vec::new();
    loop {
        let (id, payload) = server.recv().unwrap();
        match S::decode(id, &payload).unwrap() {
            S::MovePlayerPosRot(p) => moves.push(("pos_rot", p.yaw, p.pitch)),
            S::MovePlayerRot(p) => moves.push(("rot", p.yaw, p.pitch)),
            S::MovePlayerPos(_) => moves.push(("pos", f32::NAN, f32::NAN)),
            S::MovePlayerStatus(_) => moves.push(("status", f32::NAN, f32::NAN)),
            S::Punch(_) => return moves,
            _ => {}
        }
    }
}

/// How many packets of any kind the client sent before a fence.
#[cfg(feature = "client")]
pub(crate) fn sent_packets(
    game: &sunrise_client_gameplay::game::ClientGame<'_>,
    server: &mut Connection,
) -> usize {
    game.send(S::Punch(sunrise_protocol::packets::Empty));
    let mut count = 0;
    loop {
        let (id, payload) = server.recv().unwrap();
        if matches!(S::decode(id, &payload).unwrap(), S::Punch(_)) {
            return count;
        }
        count += 1;
    }
}
