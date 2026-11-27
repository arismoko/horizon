#[cfg(feature = "client")]
#[allow(unused_variables)]
impl SideOps for crate::game::client_side::ClientState {
    type Game<'g> = crate::game::ClientGame<'g>;

    fn side_guest_rides(&self, vehicle: u32) -> bool {
        false
    }
    fn side_command_players(&self) -> impl Iterator<Item = &Guest> {
        std::iter::empty()
    }

    fn side_object_target(
        game: &Self::Game<'_>,
        id: i32,
    ) -> Option<([f64; 3], [f64; 3], [f64; 3])> {
        game.client_object_target(id)
    }
}
use sunrise_gameplay_common::game::side_ops::lan::SideOps;
use sunrise_sim::lan::guest::Guest;
