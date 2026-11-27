//! `/data merge entity` (and a mod's `EntityRef::merge_nbt`) on an arrow, a spectral arrow
//! and a trident: `Entity.load` with `AbstractArrow.readAdditionalSaveData` over the
//! resident entity.
use crate::game::resource_load::load_resources;
use crate::settings::Settings;
use sunrise_core::nbt::Tag;
use sunrise_gameplay_common::game::commands_common::Target;
use sunrise_sim::entities::arrows::Pickup;
use sunrise_sim::projectiles::ProjectileKind;
use {sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress};

/// What `entity_merge_nbt` does: the saved compound, `nbt` merged in, loaded back.
fn merge(game: &mut sunrise_gameplay::game::ServerGame<'_>, target: Target, nbt: Tag) {
    let old = game.command_target_nbt(target).unwrap();
    let merged = sunrise_sim::data_cmd::merged(&old, &nbt);
    assert_ne!(merged, old);
    game.apply_entity_nbt(target, merged).unwrap();
}

#[test]
#[ignore = "daily"]
fn a_merge_reloads_an_arrows_own_save_data() {
    let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(
        sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role =
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
    let mut game = sunrise_gameplay::game::ServerGame::new(
        &loaded.resources,
        loaded.pack,
        options,
        settings.sim_settings(),
        None,
        crate::game::profile_resolver,
    )
    .unwrap();
    game.build_registries();

    let position = Tag::List(vec![
        Tag::Double(8.5_f64.to_bits()),
        Tag::Double(202.0_f64.to_bits()),
        Tag::Double(4.5_f64.to_bits()),
    ]);
    let stuck = |id: &str, uuid: [i32; 4]| {
        Tag::compound([
            ("id", Tag::String(id.into())),
            ("UUID", Tag::IntArray(uuid.to_vec())),
            ("Pos", position.clone()),
            ("inGround", Tag::Byte(1)),
            ("life", Tag::Short(1100)),
            ("shake", Tag::Byte(7)),
            ("Duration", Tag::Int(50)),
        ])
    };

    let uuid = [5007, 1, 2, 3];
    game.load_arrow(stuck("minecraft:arrow", uuid), Some(uuid), None);
    let id = game.creatures.projectiles.last().unwrap().id;
    merge(
        &mut game,
        Target::Projectile(id),
        Tag::compound([
            ("life", Tag::Short(0)),
            ("pickup", Tag::Byte(1)),
            ("crit", Tag::Byte(1)),
            ("PierceLevel", Tag::Byte(3)),
            ("damage", Tag::Double(5.0_f64.to_bits())),
        ]),
    );
    let p = game.creatures.projectiles.last().unwrap();
    assert_eq!(p.id, id);
    let a = p.arrow.as_ref().unwrap();
    // The merged keys are read...
    assert_eq!((a.life, a.pickup, a.pierce), (0, Pickup::Allowed, 3));
    assert!(matches!(
        p.kind,
        ProjectileKind::Arrow {
            base_damage: 5.0,
            critical: true,
            ..
        }
    ));
    // ...and the rest of the save comes back as it was.
    assert!(a.in_ground && p.stuck);
    assert_eq!(a.shake, 7);
    assert!(!a.spectral);
    assert_eq!(game.persist.uuids.get(&id), Some(&uuid));

    // A spectral arrow stays one: `/data`'s compound has no `id` to read it from.
    let uuid = [5007, 4, 5, 6];
    game.load_arrow(stuck("minecraft:spectral_arrow", uuid), Some(uuid), None);
    let id = game.creatures.projectiles.last().unwrap().id;
    merge(
        &mut game,
        Target::Projectile(id),
        Tag::compound([("life", Tag::Short(5))]),
    );
    let a = game
        .creatures
        .projectiles
        .last()
        .unwrap()
        .arrow
        .as_ref()
        .unwrap();
    assert_eq!(a.life, 5);
    assert!(a.spectral);
    assert_eq!(a.spectral_duration, 50);
    assert_eq!(&*a.pickup_item.item, "minecraft:spectral_arrow");

    // A thrown trident's despawn counter resets the same way.
    let uuid = [5007, 7, 8, 9];
    game.load_trident(stuck("minecraft:trident", uuid), Some(uuid));
    let id = game.tridents.iter().last().unwrap().id;
    merge(
        &mut game,
        Target::Trident(id),
        Tag::compound([("life", Tag::Short(0))]),
    );
    let t = game.tridents.iter().find(|t| t.id == id).unwrap();
    assert_eq!(t.life, 0);
    assert!(t.in_ground);
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
