use super::Game;
use horizon_client::projectiles::Projectile;
use horizon_core::geometry::Vec3;

type Steps = Option<Vec<(u32, Vec3, Vec3)>>;

pub(super) fn begin(game: &Game<'_>) -> Steps {
    game.entity_section_scope().map(|_| Vec::new())
}

pub(super) fn record(steps: &mut Steps, projectile: Option<&Projectile>, from: Vec3) {
    if let Some(p) = projectile
        && p.position != from
        && let Some(steps) = steps.as_mut()
    {
        steps.push((p.id, from, p.position));
    }
}

pub(super) fn finish(game: &mut Game<'_>, steps: Steps) {
    let Some(steps) = steps else {
        return;
    };
    let live: horizon_client::fast_hash::FxHashSet<u32> =
        game.creatures.projectiles.iter().map(|p| p.id).collect();
    for (id, from, to) in steps {
        if live.contains(&id) {
            game.entity_section_step(id, [from.x, from.y, from.z], [to.x, to.y, to.z]);
        } else {
            game.entity_section_forget(id);
        }
    }
}

pub(super) fn teleport_player(game: &mut Game<'_>, to: Vec3) {
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
