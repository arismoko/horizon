#[cfg(test)]
pub(crate) mod tests {

    use sunrise_gameplay_common::game::client_side::potions::tests::flt;

    use sunrise_core::{
        geometry::{Aabb, Vec3},
        nbt::Tag,
    };
    use sunrise_game_data::text::Text;
    use sunrise_level::particle_options::ParticleType;
    use sunrise_sim::{
        health::source_damage_type,
        mobs::{Damage, NamedDamage},
    };

    use sunrise_gameplay_common::game::dragon_common::Breath;

    use sunrise_gameplay_common::game::potions_common::{CloudSettings, PotionCloud};

    use crate::game::server_side::potions_server::{
        collect_dragon_breath, mob_can_be_affected, potion_cloud_from_nbt, potion_cloud_to_nbt,
    };

    /// `AreaEffectCloud.updateParticle` with a `custom_particle`: the codec's
    /// type and payload win over the potion colour; none falls back to the
    /// entity effect in that colour.
    #[test]
    fn cloud_custom_particle_overrides_potion_colour() {
        let cloud = |custom: Option<Tag>| {
            let mut tag = Tag::compound([
                ("Pos", Tag::List(vec![Tag::Double(0f64.to_bits()); 3])),
                ("Radius", flt(3.0)),
                ("Duration", Tag::Int(200)),
            ]);
            if let (Tag::Compound(map), Some(custom)) = (&mut tag, custom) {
                map.insert("custom_particle".into(), custom);
            }
            potion_cloud_from_nbt(&tag).unwrap().1.potion.unwrap()
        };
        let none = cloud(None).particle();
        assert_eq!(none.ty, ParticleType::EntityEffect);
        let flame = cloud(Some(Tag::compound([("type", Tag::String("flame".into()))])));
        assert_eq!(flame.particle().ty, ParticleType::Flame);
        let dust = cloud(Some(Tag::compound([
            ("type", Tag::String("minecraft:dust".into())),
            ("color", Tag::List(vec![flt(1.0), flt(0.0), flt(0.0)])),
            ("scale", flt(2.0)),
        ])))
        .particle();
        assert_eq!(dust.ty, ParticleType::Dust);
        assert_eq!(dust.color[..3], [1.0, 0.0, 0.0]);
        assert_eq!(dust.scale, 2.0);
        let effect = cloud(Some(Tag::compound([
            ("type", Tag::String("entity_effect".into())),
            ("color", Tag::Int(0x00FF_8000u32 as i32)),
        ])))
        .particle();
        assert_eq!(effect.ty, ParticleType::EntityEffect);
        assert_eq!(effect.color[..3], [1.0, 128.0 / 255.0, 0.0]);
    }
    /// `AreaEffectCloud.addAdditionalSaveData` writes `custom_particle` back.
    #[test]
    fn cloud_custom_particle_survives_a_save() {
        let particle = Tag::compound([("type", Tag::String("flame".into()))]);
        let tag = Tag::compound([
            ("Pos", Tag::List(vec![Tag::Double(0f64.to_bits()); 3])),
            ("Radius", flt(3.0)),
            ("Duration", Tag::Int(200)),
            ("custom_particle", particle.clone()),
        ]);
        let (at, cloud) = potion_cloud_from_nbt(&tag).unwrap();
        let _ = at;
        let saved = potion_cloud_to_nbt(&cloud, [1, 2, 3, 4], None);
        let Tag::Compound(map) = saved else {
            panic!("not a compound");
        };
        assert_eq!(map.get(&"custom_particle".into()), Some(&particle));
    }
    /// A mod effect's damage type keeps its registry id and tags whether it is
    /// vanilla's or a datapack's, and one the registry lacks is refused.
    #[test]
    fn effect_damage_sources_retain_their_ids() {
        let type_json = |id: &str| {
            serde_json::json!({
                "id": id, "message_id": "m", "exhaustion": "00000000",
                "scaling": "never", "effects": "hurt", "death_message_type": "default"
            })
        };
        let catalog = sunrise_world::damage_source::Catalog::from_json(
            &serde_json::json!({
                "data_version": sunrise_world::DATA_VERSION,
                "types": [type_json("minecraft:magic"), type_json("demo:scorch")],
            }),
            &serde_json::json!({
                "minecraft:is_fire": "{\"values\":[\"demo:scorch\"]}",
                "minecraft:bypasses_armor": "{\"values\":[\"demo:scorch\",\"minecraft:magic\"]}",
            }),
        )
        .unwrap();
        for (id, tags) in [
            ("minecraft:magic", NamedDamage::BYPASSES_ARMOR),
            (
                "demo:scorch",
                NamedDamage::IS_FIRE | NamedDamage::BYPASSES_ARMOR,
            ),
        ] {
            let named = NamedDamage::registered(&catalog, id).unwrap();
            assert_eq!((named.name, named.tags), (id, tags));
            let kind = crate::game::ServerGame::damage_kind(&catalog, id);
            assert_eq!(source_damage_type(Damage::of(kind)), id);
        }
        assert_eq!(NamedDamage::registered(&catalog, "demo:missing"), None);
    }
    /// `AreaEffectCloud.addAdditionalSaveData` with `readAdditionalSaveData`: a
    /// lingering cloud written at a save reads back with its lifetime, radius,
    /// use settings and the effects it sprays, with the hidden and ambient
    /// flags it carried.
    #[test]
    fn a_lingering_cloud_survives_a_save() {
        let cloud = Breath {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 7,
            at: [1.5, 64.0, -2.5],
            motion: [0.6, 0.0, 0.0],
            fall_distance: 0.0,
            fire_ticks: 0,
            first_tick: true,
            radius: 3.5,
            age: 12,
            duration: 400,
            from_dragon: false,
            victims: Vec::new(),
            potion: Some(PotionCloud {
                settings: CloudSettings {
                    wait_time: 10,
                    reapplication_delay: 25,
                    radius_on_use: -0.5,
                    duration_on_use: -5,
                    duration_scale: 0.5,
                },
                effects: vec![("poison", 200, 1), ("slowness", 100, 0)],
                hidden: vec!["slowness"],
                ambient: vec!["poison"],
                radius_per_tick: -0.008_750,
                color: 0x0033_FF44,
                custom_particle: None,
                victims: Vec::new(),
                source: Text::translate("entity.minecraft.area_effect_cloud", &[]),
                owner: None,
                guest: None,
            }),
        };
        let tag = potion_cloud_to_nbt(&cloud, [1, 2, 3, 4], None);
        let (at, back) = potion_cloud_from_nbt(&tag).expect("the cloud reads back");
        assert_eq!(at, Vec3::new(1.5, 64.0, -2.5), "the position");
        assert_eq!(back.motion, [0.6, 0.0, 0.0], "the motion");
        assert_eq!((back.age, back.duration), (12, 400));
        assert!((back.radius - 3.5).abs() < 1.0e-6, "{}", back.radius);
        let potion = back.potion.expect("a potion cloud");
        assert_eq!(
            (
                potion.settings.wait_time,
                potion.settings.reapplication_delay,
                potion.settings.duration_on_use
            ),
            (10, 25, -5)
        );
        assert!((potion.settings.radius_on_use + 0.5).abs() < 1.0e-6);
        assert!((potion.settings.duration_scale - 0.5).abs() < 1.0e-6);
        assert!((potion.radius_per_tick + 0.008_750).abs() < 1.0e-6);
        assert_eq!(
            potion.effects,
            vec![("poison", 200, 1), ("slowness", 100, 0)]
        );
        assert_eq!(potion.hidden, vec!["slowness"]);
        assert_eq!(potion.ambient, vec!["poison"]);
        assert_eq!(potion.color, 0x0033_FF44, "the colour it was drawn in");
        // The vanilla field names the reader of another client would look for.
        for key in [
            "Age",
            "Duration",
            "Radius",
            "RadiusPerTick",
            "WaitTime",
            "ReapplicationDelay",
            "RadiusOnUse",
            "DurationOnUse",
            "potion_duration_scale",
            "potion_contents",
        ] {
            assert!(tag.get(key).is_some(), "{key}");
        }
        assert_eq!(
            sunrise_sim::entity_nbt::kind(&tag).as_deref(),
            Some("minecraft:area_effect_cloud")
        );
    }
    /// `WitherBoss.canBeAffected` refuses only the wither effect: its `addEffect`
    /// refuses the rest, but a cloud of strength or healing still counts the boss
    /// as a victim (`AreaEffectCloud.serverTick` asks `canBeAffected`).
    #[test]
    fn a_cloud_still_counts_the_wither_as_a_victim() {
        let tags = sunrise_items::items::EntityTypeTags::default();
        for effect in ["strength", "instant_health"] {
            assert!(
                mob_can_be_affected(&tags, "minecraft:wither", None, effect),
                "{effect}"
            );
        }
        assert!(!mob_can_be_affected(
            &tags,
            "minecraft:wither",
            None,
            "wither"
        ));
    }
    /// `BottleItem.use`: collecting dragon's breath from an ender dragon's cloud (#8054).
    #[test]
    fn glass_bottle_collects_dragon_breath_from_dragon_cloud() {
        let player_box = Aabb::new(Vec3::new(-0.3, 64.0, -0.3), Vec3::new(0.3, 65.8, 0.3));
        let mut clouds = [Breath {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 1,
            at: [1.0, 64.0, 0.0],
            motion: [0.0; 3],
            fall_distance: 0.0,
            fire_ticks: 0,
            first_tick: true,
            radius: 5.0,
            age: 0,
            duration: 200,
            from_dragon: true,
            victims: Vec::new(),
            potion: None,
        }]
        .into_iter()
        .collect();
        assert!(collect_dragon_breath(player_box, &mut clouds).is_some());
        assert_eq!(clouds[0].radius, 4.5);
        assert!(collect_dragon_breath(player_box, &mut clouds).is_some());
        assert_eq!(clouds[0].radius, 4.0);
    }
    /// `BottleItem.use`: clouds that are expired, non-dragon, lingering potions,
    /// or out of reach cannot be bottled (#8054).
    #[test]
    fn glass_bottle_ignores_non_dragon_or_expired_or_out_of_range_clouds() {
        let player_box = Aabb::new(Vec3::new(-0.3, 64.0, -0.3), Vec3::new(0.3, 65.8, 0.3));
        // Non-dragon cloud (`from_dragon: false`).
        let mut non_dragon = [Breath {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 1,
            at: [0.0, 64.0, 0.0],
            motion: [0.0; 3],
            fall_distance: 0.0,
            fire_ticks: 0,
            first_tick: true,
            radius: 3.0,
            age: 0,
            duration: 600,
            from_dragon: false,
            victims: Vec::new(),
            potion: None,
        }]
        .into_iter()
        .collect();
        assert!(collect_dragon_breath(player_box, &mut non_dragon).is_none());
        assert_eq!(non_dragon[0].radius, 3.0);

        // Lingering potion cloud (`potion: Some(...)`).
        let mut lingering = [Breath {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 2,
            at: [0.0, 64.0, 0.0],
            motion: [0.0; 3],
            fall_distance: 0.0,
            fire_ticks: 0,
            first_tick: true,
            radius: 3.0,
            age: 0,
            duration: 600,
            from_dragon: true,
            victims: Vec::new(),
            potion: Some(PotionCloud {
                settings: CloudSettings::lingering(),
                effects: Vec::new(),
                hidden: Vec::new(),
                ambient: Vec::new(),
                radius_per_tick: -3.0 / 600.0,
                color: 0,
                custom_particle: None,
                victims: Vec::new(),
                source: Text::empty(),
                owner: None,
                guest: None,
            }),
        }]
        .into_iter()
        .collect();
        assert!(collect_dragon_breath(player_box, &mut lingering).is_none());
        assert_eq!(lingering[0].radius, 3.0);

        // Its last tick before the wait-shifted end still holds breath
        // (`tickCount - waitTime < duration`).
        let mut last = [Breath {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 3,
            at: [0.0, 64.0, 0.0],
            motion: [0.0; 3],
            fall_distance: 0.0,
            fire_ticks: 0,
            first_tick: true,
            radius: 5.0,
            age: 219,
            duration: 200,
            from_dragon: true,
            victims: Vec::new(),
            potion: None,
        }]
        .into_iter()
        .collect();
        assert!(collect_dragon_breath(player_box, &mut last).is_some());
        assert_eq!(last[0].radius, 4.5);

        // Expired cloud (`tickCount - waitTime >= duration`).
        let mut expired = [Breath {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 3,
            at: [0.0, 64.0, 0.0],
            motion: [0.0; 3],
            fall_distance: 0.0,
            fire_ticks: 0,
            first_tick: true,
            radius: 5.0,
            age: 220,
            duration: 200,
            from_dragon: true,
            victims: Vec::new(),
            potion: None,
        }]
        .into_iter()
        .collect();
        assert!(collect_dragon_breath(player_box, &mut expired).is_none());
        assert_eq!(expired[0].radius, 5.0);

        // Out of horizontal reach: cloud at 10 blocks away with radius 1:
        // cloud X range [9.0, 11.0], player inflated X range [-2.3, 2.3] -> no overlap.
        let mut far = [Breath {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 4,
            at: [10.0, 64.0, 0.0],
            motion: [0.0; 3],
            fall_distance: 0.0,
            fire_ticks: 0,
            first_tick: true,
            radius: 1.0,
            age: 0,
            duration: 200,
            from_dragon: true,
            victims: Vec::new(),
            potion: None,
        }]
        .into_iter()
        .collect();
        assert!(collect_dragon_breath(player_box, &mut far).is_none());
        assert_eq!(far[0].radius, 1.0);

        // Out of vertical reach: cloud at Y=64.0 (Y bounds [64.0, 64.5]),
        // player at Y=67.0 (inflated Y bounds [65.0, 70.8] does not reach 64.5).
        let high_player = Aabb::new(Vec3::new(-0.3, 67.0, -0.3), Vec3::new(0.3, 68.8, 0.3));
        let mut ground_cloud = [Breath {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 5,
            at: [0.0, 64.0, 0.0],
            motion: [0.0; 3],
            fall_distance: 0.0,
            fire_ticks: 0,
            first_tick: true,
            radius: 1.0,
            age: 0,
            duration: 200,
            from_dragon: true,
            victims: Vec::new(),
            potion: None,
        }]
        .into_iter()
        .collect();
        assert!(collect_dragon_breath(high_player, &mut ground_cloud).is_none());
        assert_eq!(ground_cloud[0].radius, 1.0);
    }
}
