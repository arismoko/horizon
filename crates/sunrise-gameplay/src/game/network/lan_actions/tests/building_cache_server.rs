type Boxes = Vec<([f64; 3], [f64; 3])>;
use sunrise_gameplay_common::game::Dim;

/// The other players' boxes in the Builder's turn: what blocks its placing.
pub fn seen_by_builder(game: &mut crate::game::ServerGame<'_>) -> Boxes {
    let seen = game.as_guest(3, |g| g.server.lan_building_boxes.0.boxes(Dim::Overworld));

    seen.unwrap()
}
