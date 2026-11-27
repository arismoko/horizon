//! Explicit client attack batches; the stored tick/frame pick lives in client_pick.
use crate::game::ClientGame;
use mod_api::{CrosshairTarget, EntityRef};
use sunrise_gameplay_common::game::commands_common::Target;

fn target_point(eye: [f64; 3], bounds: [[f64; 3]; 2], reach: f64) -> Option<([f64; 3], f64)> {
    if !reach.is_finite()
        || reach < 0.0
        || (0..3).any(|i| {
            !eye[i].is_finite()
                || !bounds[0][i].is_finite()
                || !bounds[1][i].is_finite()
                || bounds[0][i] > bounds[1][i]
        })
    {
        return None;
    }
    let point = std::array::from_fn(|i| eye[i].clamp(bounds[0][i], bounds[1][i]));
    let distance = (0..3)
        .map(|i| (point[i] - eye[i]).powi(2))
        .sum::<f64>()
        .sqrt();
    (distance <= reach).then_some((point, distance))
}

impl ClientGame<'_> {
    pub fn targeting_entity_hit(&self, entity: EntityRef) -> Option<([f64; 3], f64)> {
        let remote = self.client.remote.as_ref().filter(|r| r.logged_in)?;
        if !self.common.native_window() || remote.entity_id as u32 == entity.id() {
            return None;
        }
        let target = self.native_client_entity_target(entity)?;
        let attackable = match target {
            Target::Mob(_)
            | Target::Stand(_)
            | Target::Frame(_)
            | Target::Painting(_)
            | Target::Boat(_)
            | Target::Cart(_)
            | Target::Crystal(_)
            | Target::DragonPart(_)
            | Target::Cushion(_) => true,
            Target::Other(id) => self.client.interactions.contains_key(&(id as i32)),
            Target::Projectile(id) => self
                .common
                .creatures
                .projectiles
                .iter()
                .any(|p| p.id == id && self.projectile_is_pickable(p)),
            Target::Trident(id) => self
                .common
                .tridents
                .iter()
                .any(|t| t.id == id && self.trident_is_pickable(t)),
            _ => false,
        };
        if !attackable {
            return None;
        }
        let bounds = self.entity_bounding_box(entity).or_else(|| {
            self.tracked_special_box(entity.id() as i32).map(|b| {
                let (a, b) = (b.min_position(), b.max_position());
                [[a.x, a.y, a.z], [b.x, b.y, b.z]]
            })
        })?;
        let eye = self.common.eye_position()?;
        let (point, distance) = target_point(eye, bounds, self.common.entity_reach)?;
        if !self.held_range_allows(distance) {
            return None;
        }
        if distance > 0.0 {
            let dir = std::array::from_fn(|i| (point[i] - eye[i]) / distance);
            if self.clip_ray(eye, dir, distance).is_some() {
                return None;
            }
        }
        Some((point, distance))
    }
    /// The optional batch is resolved once for this click, then each hit uses vanilla's send path.
    pub fn mod_attack_targets(&mut self) -> bool {
        if !mod_api::__private::listened(mod_api::HookId::ClientAttackTarget) {
            return false;
        }
        let committed = self.crosshair_target();
        let selected = match committed {
            Some(CrosshairTarget::Entity { entity, point }) => Some((entity, point)),
            _ => None,
        };
        let mut targets: Vec<_> = selected.map(|(entity, _)| entity).into_iter().collect();
        let mut canceled = false;
        self.native_client_attack_target(&mut targets, &mut canceled);
        if canceled {
            return true;
        }
        let mut seen = std::collections::HashSet::new();
        for &entity in &targets {
            if !seen.insert(entity) {
                continue;
            }
            let distance =
                if let Some((_, point)) = selected.filter(|(selected, _)| *selected == entity) {
                    // This is the stored pick, not an independently requested batch target.
                    // Preserve its point and occlusion policy for this stored decision.
                    if self.crosshair_target() == committed {
                        self.common.attack_hit_distance(point)
                    } else {
                        None
                    }
                } else {
                    self.targeting_entity_hit(entity)
                        .map(|(_, distance)| distance)
                };
            if let Some(distance) = distance {
                self.remote_attack_entity(entity.id() as i32, distance);
            }
        }
        selected.is_some() || !targets.is_empty()
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    pub fn receive_attacks(server: &mut sunrise_protocol::connection::Connection) -> Vec<i32> {
        use sunrise_protocol::packets::play::Serverbound as S;
        let mut ids = Vec::new();
        loop {
            let (id, payload) = server.recv().unwrap();
            match S::decode(id, &payload).unwrap() {
                S::Attack(attack) => ids.push(attack.id),
                S::Punch(_) => return ids,
                _ => {}
            }
        }
    }

    #[cfg(test)]
    use super::*;

    #[cfg(test)]
    #[test]
    fn target_range_uses_nearest_box_point_and_rejects_invalid_geometry() {
        let bounds = [[2.0, -1.0, -1.0], [4.0, 1.0, 1.0]];
        assert_eq!(
            target_point([0.0; 3], bounds, 2.0),
            Some(([2.0, 0.0, 0.0], 2.0))
        );
        assert_eq!(target_point([0.0; 3], bounds, 1.999), None);
        assert_eq!(
            target_point([3.0, 0.0, 0.0], bounds, 0.0),
            Some(([3.0, 0.0, 0.0], 0.0))
        );
        for reach in [-1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(target_point([0.0; 3], bounds, reach), None);
        }
        assert_eq!(target_point([f64::NAN; 3], bounds, 3.0), None);
        assert_eq!(target_point([0.0; 3], [bounds[1], bounds[0]], 3.0), None);
    }

    pub fn finish_login(
        game: &mut crate::game::ClientGame<'_>,
        server: &mut sunrise_protocol::connection::Connection,
    ) {
        use sunrise_protocol::packets::{Empty, configuration as cfg, login};
        server.recv().unwrap(); // Intention.
        server.recv().unwrap(); // Hello.
        server
            .send(
                &login::Clientbound::LoginFinished(login::LoginFinished {
                    profile: login::GameProfile {
                        uuid: 42,
                        name: "SessionReadTest".into(),
                        properties: vec![],
                    },
                    session: 1,
                })
                .encode(),
            )
            .unwrap();
        let (id, payload) = server.recv().unwrap();
        assert!(matches!(
            login::Serverbound::decode(id, &payload).unwrap(),
            login::Serverbound::LoginAcknowledged(_)
        ));
        server.recv().unwrap(); // Brand.
        server.recv().unwrap(); // Client information.
        server
            .send(&cfg::Clientbound::FinishConfiguration(Empty).encode())
            .unwrap();
        game.acknowledge_test_configuration();
        let (id, payload) = server.recv().unwrap();
        assert!(matches!(
            cfg::Serverbound::decode(id, &payload).unwrap(),
            cfg::Serverbound::FinishConfiguration(_)
        ));
    }

    /// A FIFO swing packet fences the real connection's outgoing attack packets.
    pub fn sent_attacks(
        game: &crate::game::ClientGame<'_>,
        server: &mut sunrise_protocol::connection::Connection,
    ) -> Vec<i32> {
        use sunrise_protocol::packets::{Empty, play::Serverbound as S};
        game.send(S::Punch(Empty));
        receive_attacks(server)
    }
}

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::dragon_common::DragonCommonGameExt1 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
