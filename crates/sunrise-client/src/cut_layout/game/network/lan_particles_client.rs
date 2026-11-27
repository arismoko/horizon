#![cfg(test)]
use sunrise_level::particle_options::ParticleType;
use sunrise_protocol::entity_data::{Particle, ParticleData};
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "daily"]
    fn received_block_particles_follow_the_provider_matrix() {
        use {
            crate::game::server_side::native_entity_refs::tests::with_client_game,
            sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client,
            sunrise_gameplay_common::game::in_own_process,
        };
        in_own_process(
            "cut_layout::game::client_side::lan_particles_client::tests::received_block_particles_follow_the_provider_matrix",
            || {
                with_client_game(|game| {
                    let _server = logged_client(game);
                    let types = [
                        ParticleType::Block,
                        ParticleType::DustPillar,
                        ParticleType::BlockCrumble,
                        ParticleType::FallingDust,
                        ParticleType::BlockMarker,
                    ];
                    let registry = game
                        .client
                        .remote
                        .as_ref()
                        .unwrap()
                        .builtin
                        .get("minecraft:particle_type")
                        .unwrap();
                    let ids =
                        types.map(|ty| registry.id(&format!("minecraft:{}", ty.name())).unwrap());
                    for (name, terrain, dust) in [
                        ("air", false, true),
                        ("cave_air", false, true),
                        ("void_air", false, true),
                        ("moving_piston", false, false),
                        ("barrier", false, false),
                        ("structure_void", false, false),
                        ("light", true, false),
                        ("water", true, false),
                        ("chest", true, true),
                        ("sand", true, true),
                        ("stone", true, true),
                    ] {
                        let state = game
                            .pack
                            .assets
                            .registry
                            .default_state(&format!("minecraft:{name}"))
                            .unwrap();
                        for ((ty, id), admit) in types
                            .into_iter()
                            .zip(ids)
                            .zip([terrain, terrain, terrain, dust, true])
                        {
                            let result = game.received_particle(
                                &Particle {
                                    id,
                                    data: Some(ParticleData::State(state.0 as i32)),
                                },
                                [0.5, 64.5, 0.5],
                            );
                            assert_eq!(result.is_some(), admit, "{name} for {}", ty.name());
                            if let Some(options) = result {
                                assert_eq!(options.ty, ty);
                                let (sprite, tint) = options.block.unwrap();
                                if ty == ParticleType::BlockMarker {
                                    assert!(!sprite.is_empty());
                                    assert_eq!(tint, 0xFFFFFF);
                                } else if ty == ParticleType::FallingDust {
                                    assert!(
                                        sprite.is_empty(),
                                        "falling dust uses the provider's sprite set"
                                    );
                                    match name {
                                        "sand" => assert_eq!(tint, 0xDBD3A0),
                                        "stone" => assert_eq!(tint, 0x707070),
                                        "air" | "cave_air" | "void_air" => assert_eq!(tint, 0),
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
                })
            },
        );
    }
}
