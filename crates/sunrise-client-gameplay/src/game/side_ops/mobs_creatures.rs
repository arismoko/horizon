#[cfg(feature = "client")]
#[allow(unused_variables)]
impl SideOps for crate::game::client_side::ClientState {
    type Game<'g> = crate::game::ClientGame<'g>;

    fn side_handle_mob_events_reusable(
        game: &mut Self::Game<'_>,
        events: &mut Vec<MobEvent>,
    ) -> Result<()> {
        game.client_handle_mob_events_reusable(events)
    }
}
use sunrise_gameplay_common::game::Result;
use sunrise_gameplay_common::game::side_ops::mobs_creatures::SideOps;
use sunrise_sim::mobs::MobEvent;
