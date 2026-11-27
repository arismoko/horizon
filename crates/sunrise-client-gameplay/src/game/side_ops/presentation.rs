#[cfg(feature = "client")]
#[allow(unused_variables)]
impl SideOps for crate::game::client_side::ClientState {
    type Game<'g> = crate::game::ClientGame<'g>;

    fn side_swing_hand(game: &mut Self::Game<'_>, hand: Hand, animation: SwingAnimation) -> bool {
        game.client_swing_hand(hand, animation)
    }
    fn side_fake_action_unswung(_game: &Self::Game<'_>) -> bool {
        false
    }
    fn side_sound_event_names(game: &Self::Game<'_>) -> Vec<String> {
        game.client_sound_event_names()
    }
    fn side_sync_pack_tag_ids(game: &mut Self::Game<'_>) {
        {
            game.client_sync_pack_tag_ids();
        }
    }
    fn side_sync_hub_clocks(game: &mut Self::Game<'_>) {
        {
            game.client_sync_hub_clocks();
        }
    }
    fn side_has_singleplayer_server(game: &Self::Game<'_>) -> bool {
        game.client_has_singleplayer_server()
    }
    fn side_is_published(game: &Self::Game<'_>) -> bool {
        game.client_is_published()
    }
    fn side_clear_lod(game: &mut Self::Game<'_>) {
        {
            game.client_clear_lod();
        }
    }
    fn side_clear_render_caches(game: &mut Self::Game<'_>) {
        {
            game.client_clear_render_caches();
        }
    }
    fn side_expire_cracks(game: &mut Self::Game<'_>) {
        {
            game.client_expire_cracks();
        }
    }
    fn side_update_spawner_display(game: &mut Self::Game<'_>, key: (i32, i32, i32)) -> bool {
        game.client_update_spawner_display(key)
    }
    fn side_memory_inputs(game: &Self::Game<'_>) -> MemoryInputs {
        game.client_memory_inputs()
    }
    fn side_memory_report(game: &Self::Game<'_>) -> MemoryReport {
        game.client_memory_report()
    }
    fn side_test_holder_padding(game: &Self::Game<'_>, pos: (i32, i32, i32)) -> i32 {
        game.client_test_holder_padding(pos)
    }
    fn side_test_rotation(game: &Self::Game<'_>, pos: (i32, i32, i32)) -> Rotation {
        game.client_test_rotation(pos)
    }
    fn side_pick_camera_id(game: &Self::Game<'_>) -> Option<u32> {
        game.client_pick_camera_id()
    }
    fn side_live_remote_camera_id(game: &Self::Game<'_>) -> Option<u32> {
        game.client_live_remote_camera_id()
    }
    fn side_is_pick_camera(game: &Self::Game<'_>, id: u32) -> bool {
        game.client_is_pick_camera(id)
    }
    fn side_outline_camera_kind(game: &Self::Game<'_>) -> EntityContextKind {
        game.client_outline_camera_kind()
    }
    fn side_close_container(game: &mut Self::Game<'_>) {
        {
            game.close_container_client();
        }
    }
}
use sunrise_gameplay_common::game::memory_common::MemoryInputs;
use sunrise_gameplay_common::game::memory_common::MemoryReport;
use sunrise_gameplay_common::game::side_ops::presentation::SideOps;
use sunrise_sim::first_person_sim::Hand;
use sunrise_sim::first_person_sim::SwingAnimation;
use sunrise_world::collision_context::EntityContextKind;
use sunrise_world::worldgen::structure::Rotation;
