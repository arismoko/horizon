#![cfg(test)]
use sunrise_level::particle_options::ParticleType;
#[cfg(test)]
#[cfg(feature = "client")]
pub(crate) mod tests {

    use super::*;

    #[test]
    #[ignore = "daily"]
    fn typed_cloud_remote_metadata_keeps_valid_options_on_malformed_update() {
        if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::client_side::potions::tests::typed_cloud_remote_metadata_keeps_valid_options_on_malformed_update",
            "SUNRISE_TEST_TYPED_CLOUD_CLIENT_CASE",
            &["cloud"],
        ) {
            return;
        }
        use sunrise_gameplay::game::lan_particles::tests::MeasuredParticle;
        use sunrise_protocol::{
            entity_data::{Particle, ParticleData, Value, serializer},
            packets::play::AddEntity,
        };

        sunrise_client_gameplay::game::native_entity_refs::tests::with_client_game(|game| {
            let entries = [mod_api::ModEntry::new("test", Box::new(MeasuredParticle))];
            mod_platform::__private::part_tests::install_parts_fixture(&entries).unwrap();
            sunrise_gameplay_common::game::mods::resolve_particle_providers(&Default::default())
                .unwrap();
            let id = 42_i32;
            let particle = Particle {
                id: 0,
                data: Some(ParticleData::Mod {
                    name: "test:measured".into(),
                    options: vec![1, 1, 0, 0],
                }),
            };
            game.spawn_remote_projectile(
                "minecraft:area_effect_cloud",
                &AddEntity {
                    id,
                    uuid: 0,
                    kind: 0,
                    position: [0.0, 80.0, 0.0],
                    velocity: [0.0; 3],
                    pitch: 0,
                    yaw: 0,
                    head_yaw: 0,
                    data: 0,
                },
                (0.0, 0.0),
            );
            assert_eq!(
                game.client.remote_clouds[0].particle.ty,
                ParticleType::EntityEffect
            );
            game.remote_projectile_data(
                id,
                &[(10, serializer::PARTICLE, Value::Particle(particle.clone()))],
            );
            let original = game.client.remote_clouds[0].particle.typed_particle.clone();
            assert!(original.is_some());
            let mut bad = particle;
            let Some(ParticleData::Mod { options, .. }) = &mut bad.data else {
                unreachable!()
            };
            options.truncate(1);
            game.remote_projectile_data(id, &[(10, serializer::PARTICLE, Value::Particle(bad))]);
            assert_eq!(
                game.client.remote_clouds[0].particle.typed_particle,
                original
            );
        });
    }
}

#[cfg(feature = "client")]
#[cfg(test)]
#[path = "potions/cloud_water_tests.rs"]
pub(crate) mod cloud_water_tests;
