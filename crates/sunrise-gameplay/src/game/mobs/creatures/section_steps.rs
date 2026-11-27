use sunrise_core::geometry::Vec3;
use sunrise_sim::projectiles::Projectile;

pub(in crate::game) struct Steps {
    enabled: bool,
    seen: Vec<u32>,
}

pub(in crate::game) fn begin(game: &mut crate::game::ServerGame<'_>) -> Steps {
    Steps {
        enabled: game.entity_section_scope().is_some(),
        seen: std::mem::take(&mut game.native_lifecycle.entity_sections.projectile_ids),
    }
}

pub(in crate::game) fn record(
    game: &mut crate::game::ServerGame<'_>,
    steps: &mut Steps,
    projectile: Option<&Projectile>,
    from: Vec3,
    keep: bool,
) -> bool {
    if let Some(p) = projectile
        && p.position != from
        && keep
    {
        let (from, to) = (
            [from.x, from.y, from.z],
            [p.position.x, p.position.y, p.position.z],
        );
        if steps.enabled {
            steps.seen.push(p.id);
            return game.entity_section_step_visible(p.id, from, to);
        }
        game.entity_section_entered(p.id, from, to);
    }
    true
}

pub(in crate::game) fn finish(game: &mut crate::game::ServerGame<'_>, mut steps: Steps) {
    if steps.enabled {
        {
            let live = &mut game.common.native_lifecycle.entity_sections.projectile_live;
            live.clear();
            live.extend(
                game.common
                    .creatures
                    .projectiles
                    .iter()
                    .map(|projectile| projectile.id),
            );
        }
        for id in steps.seen.drain(..) {
            if !game
                .native_lifecycle
                .entity_sections
                .projectile_live
                .contains(&id)
            {
                game.entity_section_forget(id);
            }
        }
    } else {
        steps.seen.clear();
    }
    steps.seen.extend(std::mem::take(
        &mut game.native_lifecycle.entity_sections.projectile_ids,
    ));
    game.native_lifecycle.entity_sections.projectile_ids = steps.seen;
}

pub(in crate::game) fn teleport_player(game: &mut crate::game::ServerGame<'_>, to: Vec3) {
    game.stop_riding();
    let from = game.player.as_ref().map(|player| player.position);
    if let Some(player) = game.player.as_mut() {
        player.position = to;
        player.velocity = Vec3::ZERO;
        player.fall_distance = 0.0;
        player.impulse.reset();
        player.teleported = true;
        player.teleported_delta = [false; 3];
    }
    game.previous_position = to;
    if let (Some(from), Ok(id)) = (from, u32::try_from(game.own_entity_id())) {
        game.entity_section_step(id, [from.x, from.y, from.z], [to.x, to.y, to.z]);
    }
}

use sunrise_gameplay_common::game::entity_sections::EntitySectionsGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
