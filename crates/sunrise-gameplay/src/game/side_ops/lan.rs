#[allow(unused_variables)]
impl SideOps for crate::game::server_side::ServerState {
    type Game<'g> = crate::game::ServerGame<'g>;

    fn side_guest_rides(&self, vehicle: u32) -> bool {
        self.lan
            .as_ref()
            .is_some_and(|h| h.hub.guests.iter().any(|g| g.vehicle == Some(vehicle)))
    }
    fn side_command_players(&self) -> impl Iterator<Item = &Guest> {
        self.lan.iter().flat_map(|h| h.hub.online_players())
    }

    fn side_object_target(
        game: &Self::Game<'_>,
        id: i32,
    ) -> Option<([f64; 3], [f64; 3], [f64; 3])> {
        game.server_object_target(id)
    }
}
use sunrise_gameplay_common::game::side_ops::lan::SideOps;
use sunrise_sim::lan::guest::Guest;
