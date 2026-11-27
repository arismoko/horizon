use sunrise_protocol::{
    connection::ConnReader,
    packets::{
        configuration::Ping,
        play::{Clientbound as C, Serverbound as S},
    },
};

pub(in crate::game) fn packets_through_pong(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    reader: &mut ConnReader,
) -> Vec<S> {
    game.handle_packet(C::Ping(Ping { id: 26 })).unwrap();
    // Pong follows everything the handler queued, including an erroneous close.
    std::iter::from_fn(|| {
        let (id, body) = reader.recv().unwrap();
        match S::decode(id, &body).unwrap() {
            S::Pong(p) if p.id == 26 => None,
            packet => Some(packet),
        }
    })
    .collect()
}
