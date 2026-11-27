// Included in entity_nbt.rs to retain its private scope.
/// `Projectile` + `AbstractArrow.addAdditionalSaveData` of a live arrow;
/// `in_block` is its `inBlockState`, `raw` the compound it was loaded with.
pub fn arrow_to_nbt(
    p: &Projectile,
    uuid: [i32; 4],
    owner: Option<[i32; 4]>,
    in_block: Option<Tag>,
    raw: Option<&Tag>,
    items: &crate::items::Items,
) -> Option<Tag> {
    let a = p.arrow.as_deref()?;
    let ProjectileKind::Arrow {
        base_damage,
        critical,
        ..
    } = p.kind
    else {
        return None;
    };
    let base = Base {
        // A spectral arrow is its own entity type.
        id: if a.spectral || &*a.pickup_item.item == "minecraft:spectral_arrow" {
            "minecraft:spectral_arrow"
        } else {
            "minecraft:arrow"
        },
        uuid,
        position: p.position,
        motion: p.velocity,
        rotation: [p.yaw, p.pitch],
        on_ground: p.ground.on_ground,
        fall_distance: a.fall_distance,
    };
    let mut tag = base.tag(raw);
    // `SpectralArrow.addAdditionalSaveData` always writes its integer duration.
    if base.id == "minecraft:spectral_arrow" {
        put(&mut tag, "Duration", Tag::Int(a.spectral_duration));
    }
    for (key, value) in [
        ("Fire", Tag::Short(a.fire_ticks as i16)),
        ("Air", Tag::Short(p.air_supply as i16)),
        ("life", Tag::Short(a.life as i16)),
        ("shake", Tag::Byte(a.shake as i8)),
        ("inGround", Tag::Byte(i8::from(a.in_ground))),
        ("pickup", Tag::Byte(a.pickup.byte())),
        ("damage", Tag::Double(base_damage.to_bits())),
        ("crit", Tag::Byte(i8::from(critical))),
        ("PierceLevel", Tag::Byte(a.pierce as i8)),
        ("HasBeenShot", Tag::Byte(i8::from(a.has_been_shot))),
        ("item", item_to_nbt(&a.pickup_item, items)),
    ] {
        put(&mut tag, key, value);
    }
    if let Tag::Compound(map) = &mut tag {
        let sound = a.saved_sound_event(map.get(&"SoundEvent".into()));
        map.insert("SoundEvent".into(), sound);
        // Projectile: LeftOwner only when set; Owner as the UUID.
        map.remove(&"LeftOwner".into());
        map.remove(&"inBlockState".into());
        map.remove(&"weapon".into());
        map.remove(&"can_break".into());
    }
    if let Some(weapon) = &a.fired_from_weapon {
        put(&mut tag, "weapon", item_to_nbt(weapon, items));
    }
    // `Projectile.addAdditionalSaveData`: the actual decoded predicate,
    // never a stale source compound or the pickup item's component patch.
    if let Some(predicate) = &a.can_break {
        put(&mut tag, "can_break", predicate.clone());
    }
    if p.left_owner {
        put(&mut tag, "LeftOwner", Tag::Byte(1));
    }
    if let Some(state) = in_block {
        put(&mut tag, "inBlockState", state);
    }
    // Entity.saveWithoutId writes NoGravity only when set.
    if p.no_gravity {
        put(&mut tag, "NoGravity", Tag::Byte(1));
    }
    if let Some(owner) = owner {
        put(&mut tag, "Owner", Tag::IntArray(owner.to_vec()));
    }
    Some(tag)
}
/// A saved arrow as a live one (`readAdditionalSaveData`), with its
/// `inBlockState` compound for the caller to resolve. `effect` is the tipped
/// arrow's lasting effect from its item.
pub fn arrow_from_nbt(
    tag: &Tag,
    player: Option<[i32; 4]>,
    effect: impl FnOnce(&crate::items::ItemStack) -> Option<(&'static str, i32)>,
) -> Option<(Projectile, Option<Tag>)> {
    let position = position(tag)?;
    let owner = uuid_from_tag(tag.get("Owner"));
    let from_player = player.is_some() && owner == player;
    // `getDefaultPickupItem`: a `SpectralArrow` is picked up as its own item.
    let spectral = kind(tag).as_deref() == Some("minecraft:spectral_arrow");
    let default_item = if spectral {
        "minecraft:spectral_arrow"
    } else {
        "minecraft:arrow"
    };
    let item = tag
        .get("item")
        .and_then(item_from_nbt)
        .unwrap_or_else(|| crate::items::ItemStack::new(default_item, 1));
    let kind = ProjectileKind::Arrow {
        base_damage: float(tag.get("damage")).unwrap_or(2.0),
        critical: flag(tag, "crit"),
        effect: effect(&item),
    };
    let mut p = Projectile::new(kind, position, from_player.then_some(Who::Player));
    p.velocity = motion(tag);
    [p.yaw, p.pitch] = loaded_rotation(tag);
    p.set_old_pos_and_rot();
    p.left_owner = flag(tag, "LeftOwner");
    p.owner_uuid = owner;
    p.no_gravity = flag(tag, "NoGravity");
    p.ground.on_ground = flag(tag, "OnGround");
    projectile_fields::load_arrow_entity_fields(&mut p, tag);
    let a = p.arrow.as_mut()?;
    a.life = number(tag.get("life")).unwrap_or(0) as i16 as i32;
    a.shake = (number(tag.get("shake")).unwrap_or(0) as i8 as u8).into();
    a.in_ground = flag(tag, "inGround");
    a.pickup = Pickup::from_byte(number(tag.get("pickup")).unwrap_or(0));
    a.pierce = number(tag.get("PierceLevel")).unwrap_or(0) as u8;
    a.has_been_shot = flag(tag, "HasBeenShot");
    // `Projectile.readAdditionalSaveData`: one block predicate or a nonempty
    // list. Unsupported conjuncts remain saved, but cannot grant permission.
    a.can_break = tag
        .get("can_break")
        .filter(|predicate| match predicate {
            Tag::Compound(_) => true,
            Tag::List(list) => {
                !list.is_empty() && list.iter().all(|p| matches!(p, Tag::Compound(_)))
            }
            _ => false,
        })
        .cloned();
    a.spectral = spectral;
    // `SpectralArrow.readAdditionalSaveData`: absent Duration defaults to 200.
    if spectral {
        a.spectral_duration = number(tag.get("Duration")).unwrap_or(200) as i32;
    }
    a.pickup_item = item;
    a.load_weapon_and_sound(tag, item_from_nbt);
    p.stuck = a.in_ground;
    Some((p, tag.get("inBlockState").cloned()))
}

/// `LargeFireball`, `SmallFireball` and `WitherSkull`: `AbstractHurtingProjectile`'s
/// `acceleration_power`, `Projectile`'s owner and flags, `ExplosionPower`,
/// and a skull's `dangerous`.
pub fn fireball_to_nbt(
    p: &Projectile,
    uuid: [i32; 4],
    owner: Option<[i32; 4]>,
    raw: Option<&Tag>,
    items: &crate::items::Items,
) -> Option<Tag> {
    let power = match p.kind {
        ProjectileKind::LargeFireball { power } => Some(power),
        ProjectileKind::SmallFireball
        | ProjectileKind::DragonFireball
        | ProjectileKind::WindCharge
        | ProjectileKind::BreezeWindCharge
        | ProjectileKind::WitherSkull { .. } => None,
        _ => return None,
    };
    let base = Base {
        id: p.kind.id(),
        uuid,
        position: p.position,
        motion: p.velocity,
        rotation: [p.yaw, p.pitch],
        on_ground: p.ground.on_ground,
        fall_distance: p.fall_distance,
    };
    let mut tag = base.tag(raw);
    put(
        &mut tag,
        "acceleration_power",
        Tag::Double(p.acceleration.to_bits()),
    );
    put_projectile(&mut tag, p, owner);
    if projectile_fields::keeps_entity_fields(p) {
        put_projectile_entity_fields(&mut tag, p);
    }
    if let Some(power) = power {
        put(&mut tag, "ExplosionPower", Tag::Byte(power as i8));
    }
    // WitherSkull.addAdditionalSaveData: `dangerous`, false for a black skull.
    if let ProjectileKind::WitherSkull { dangerous } = p.kind {
        put(&mut tag, "dangerous", Tag::Byte(i8::from(dangerous)));
    }
    // Fireball.addAdditionalSaveData: the item `getItem` returns, a fire
    // charge unless `setItem` was given another (kept as a single one).
    if let Some(item) = &p.item {
        put(&mut tag, "Item", item_to_nbt(item, items));
    }
    Some(tag)
}

/// `Projectile.addAdditionalSaveData`: Owner, LeftOwner only when set, and
/// the explicit HasBeenShot flag.
fn put_projectile(tag: &mut Tag, p: &Projectile, owner: Option<[i32; 4]>) {
    if let Tag::Compound(map) = tag {
        map.remove(&"LeftOwner".into());
    }
    if p.left_owner {
        put(tag, "LeftOwner", Tag::Byte(1));
    }
    // Entity.saveWithoutId writes NoGravity only when set.
    if p.no_gravity {
        put(tag, "NoGravity", Tag::Byte(1));
    }
    put(tag, "HasBeenShot", Tag::Byte(i8::from(p.has_been_shot)));
    if let Some(owner) = owner {
        put(tag, "Owner", Tag::IntArray(owner.to_vec()));
    }
}

/// The thrown item projectiles Sunrise saves: each entity type's id is also
/// the item it saves when none was loaded (`getDefaultItem`).
const THROWABLE_ITEMS: [&str; 6] = [
    "minecraft:snowball",
    "minecraft:egg",
    "minecraft:ender_pearl",
    "minecraft:splash_potion",
    "minecraft:lingering_potion",
    "minecraft:experience_bottle",
];

fn decode_throwable_components(raw: &Tag, items: &crate::items::Items) -> Option<Tag> {
    let Tag::Compound(entries) = raw else {
        return None;
    };
    let reg = &items.component_registry;
    let type_ids = reg.ids("minecraft:data_component_type");
    let check_registry = !type_ids.is_empty();
    let mut decoded = Compound::new();
    for (raw_key, value) in entries {
        let Ok(key_text) = raw_key.to_string() else {
            continue;
        };
        let (removed, name) = match key_text.strip_prefix('!') {
            Some(rest) => (true, rest),
            None => (false, key_text.as_str()),
        };
        let Ok(parsed) = sunrise_core::identifier::Identifier::parse(name) else {
            continue;
        };
        let id = parsed.to_string();
        if sunrise_world::item_defaults::is_transient_component(&id) {
            continue;
        }
        if check_registry {
            let known = type_ids.iter().any(|entry| entry == &id)
                || reg.persistent_components.iter().any(|entry| entry == &id)
                || sunrise_command::item::DECODED_COMPONENTS.contains(&id.as_str())
                || sunrise_protocol::item::custom::id(&id).is_some();
            if !known {
                continue;
            }
        }
        if removed {
            if !matches!(value, Tag::Compound(_)) {
                continue;
            }
            decoded.remove(&JavaString::from(id.as_str()));
            decoded.insert(
                JavaString::from(format!("!{id}").as_str()),
                Tag::Compound(Compound::new()),
            );
            continue;
        }
        let value = if id.starts_with("minecraft:") {
            let Ok(value) = sunrise_command::item::decoded_component_value(&id, value, reg) else {
                continue;
            };
            value
        } else {
            if sunrise_core::nbt::validate_component_value(value).is_err() {
                continue;
            }
            value.clone()
        };
        decoded.remove(&JavaString::from(format!("!{id}").as_str()));
        decoded.insert(JavaString::from(id.as_str()), value);
    }
    (!decoded.is_empty()).then_some(Tag::Compound(decoded))
}

/// Decodes a throwable `Item` tag with vanilla's item and per-entry component codec rules.
pub fn decode_throwable_item(
    tag: &Tag,
    items: &crate::items::Items,
) -> Option<crate::items::ItemStack> {
    let Tag::Compound(_) = tag else {
        return None;
    };
    let (namespace, path) = sunrise_world::datafix::identifier(&text(tag.get("id"))?)?;
    let id = format!("{namespace}:{path}");
    if id == "minecraft:air" || (!items.items.is_empty() && items.get(&id).is_none()) {
        return None;
    }
    let components = tag
        .get("components")
        .and_then(|raw| decode_throwable_components(raw, items));
    let (damage, extra) = crate::item_data::decode_components(components.as_ref());
    Some(crate::items::ItemStack {
        item: Arc::from(id.as_str()),
        count: 1,
        damage,
        extra,
    })
}

/// Decode the `Item` field of a throwable compound using vanilla's item and component codecs.
pub fn decode_throwable_item_field(
    tag: &Tag,
    items: &crate::items::Items,
) -> Option<crate::items::ItemStack> {
    tag.get("Item")
        .and_then(|item| decode_throwable_item(item, items))
}

/// The thrown kind an entity id names: vanilla's, or a content mod's type.
/// A potion flies as the item its entity type names, which is the item a
/// summoned one has when it names none (`getDefaultItem`).
pub fn throwable_kind(id: &str) -> Option<ProjectileKind> {
    if !THROWABLE_ITEMS.contains(&id) {
        return crate::projectiles::thrown_type(id).map(ProjectileKind::Thrown);
    }
    match id {
        "minecraft:snowball" => Some(ProjectileKind::Snowball),
        "minecraft:egg" => Some(ProjectileKind::Egg),
        "minecraft:ender_pearl" => Some(ProjectileKind::EnderPearl),
        // The entity type is the id; the `Item` read later may be another potion.
        "minecraft:experience_bottle" => Some(ProjectileKind::ExperienceBottle),
        _ => Some(ProjectileKind::Potion(
            Box::new(crate::items::ItemStack::new(id, 1)),
            id == "minecraft:lingering_potion",
        )),
    }
}

/// Whether its chunk saves it: the kinds `arrow_to_nbt`, `fireball_to_nbt`
/// and [`throwable_to_nbt`] write.
pub fn saved_with_chunk(p: &Projectile) -> bool {
    // No wildcard: a kind added to `ProjectileKind` must say here whether its
    // chunk saves it (`Entity.shouldBeSaved` is true for every projectile in
    // vanilla), and a `true` needs a writer in `entity_chunks`.
    match &p.kind {
        ProjectileKind::LargeFireball { .. }
        | ProjectileKind::SmallFireball
        | ProjectileKind::DragonFireball
        | ProjectileKind::WindCharge
        | ProjectileKind::BreezeWindCharge
        | ProjectileKind::WitherSkull { .. }
        | ProjectileKind::Snowball
        | ProjectileKind::Egg
        | ProjectileKind::EnderPearl
        | ProjectileKind::Potion(..)
        | ProjectileKind::ExperienceBottle
        | ProjectileKind::LlamaSpit
        | ProjectileKind::ShulkerBullet { .. }
        | ProjectileKind::Thrown(_) => true,
        ProjectileKind::Arrow { .. } => p.arrow.is_some(),
        // Not written yet: tridents save from their own list. Wither skulls and
        // bullets are written now.
        ProjectileKind::Trident => false,
    }
}

/// `ThrowableItemProjectile.addAdditionalSaveData`: `Projectile`'s keys and
/// the `Item` it flies as.
pub fn throwable_to_nbt(
    p: &Projectile,
    uuid: [i32; 4],
    owner: Option<[i32; 4]>,
    raw: Option<&Tag>,
    items: &crate::items::Items,
) -> Option<Tag> {
    let stack = p.thrown_item_stack()?;
    let base = Base {
        id: p.kind.id(),
        uuid,
        position: p.position,
        motion: p.velocity,
        rotation: [p.yaw, p.pitch],
        on_ground: p.ground.on_ground,
        fall_distance: p.fall_distance,
    };
    let mut tag = base.tag(raw);
    put_projectile(&mut tag, p, owner);
    put_projectile_entity_fields(&mut tag, p);
    // Entity.saveWithoutId writes NoGravity only when set.
    if p.no_gravity {
        put(&mut tag, "NoGravity", Tag::Byte(1));
    }
    // ThrowableItemProjectile.addAdditionalSaveData writes the entity's typed Item at count 1.
    put(&mut tag, "Item", item_to_nbt(&stack, items));
    Some(tag)
}

/// `ThrowableItemProjectile.readAdditionalSaveData`, validating `Item` against `items`.
pub fn throwable_from_nbt_with_items(
    tag: &Tag,
    player: Option<[i32; 4]>,
    items: &crate::items::Items,
) -> Option<Projectile> {
    let kind = throwable_kind(&kind(tag)?)?;
    let owner = owner_uuid(tag);
    let from_player = player.is_some() && owner == player;
    let mut p = Projectile::new(kind, position(tag)?, from_player.then_some(Who::Player));
    p.set_thrown_item(decode_throwable_item_field(tag, items));
    p.velocity = motion(tag);
    [p.yaw, p.pitch] = loaded_rotation(tag);
    p.set_old_pos_and_rot();
    p.left_owner = flag(tag, "LeftOwner");
    p.has_been_shot = flag(tag, "HasBeenShot");
    p.owner_uuid = owner;
    p.no_gravity = flag(tag, "NoGravity");
    load_projectile_entity_fields(&mut p, tag);
    p.ground.on_ground = flag(tag, "OnGround");
    Some(p)
}

/// `ThrowableItemProjectile.readAdditionalSaveData` without a loaded item catalog.
pub fn throwable_from_nbt(tag: &Tag, player: Option<[i32; 4]>) -> Option<Projectile> {
    throwable_from_nbt_with_items(tag, player, &crate::items::Items::default())
}

/// `LlamaSpit` and `ShulkerBullet` as `/summon` or a chunk loads them, with
/// `ShulkerBullet.readAdditionalSaveData`'s `Dir`, `Steps` and per-axis target
/// deltas: `Projectile`'s `Owner`, `LeftOwner`, `NoGravity` and the motion.
/// `target` is the entity the caller resolved from `Target`; a bullet loaded
/// without one falls (`ShulkerBullet.tick`).
pub fn spit_or_bullet_from_nbt(
    tag: &Tag,
    player: Option<[i32; 4]>,
    target: Option<Who>,
) -> Option<Projectile> {
    let at = position(tag)?;
    let kind = match kind(tag)?.as_str() {
        "minecraft:llama_spit" => ProjectileKind::LlamaSpit,
        "minecraft:shulker_bullet" => ProjectileKind::ShulkerBullet {
            target,
            // `Dir`: `Direction.LEGACY_ID_CODEC`, the step the bullet is on,
            // absent while it has not chosen one. A byte read through
            // `Direction.from3DDataValue`, which wraps it: `abs(d % 6)`.
            dir: number(tag.get("Dir")).map(|d| ((d as i8) % 6).unsigned_abs()),
            // `getIntOr("Steps", 0)`: any int, a negative one included.
            steps: number(tag.get("Steps")).unwrap_or(0) as i32,
            delta: [
                float(tag.get("TXD")).unwrap_or(0.0),
                float(tag.get("TYD")).unwrap_or(0.0),
                float(tag.get("TZD")).unwrap_or(0.0),
            ],
            // `Entity.random` is not saved: a loaded bullet gets a stream of
            // its own from where it is, as `Projectile::new` does for an
            // arrow.
            seed: (at.x.to_bits() ^ at.z.to_bits().rotate_left(17)) | 1,
        },
        _ => return None,
    };
    let owner = owner_uuid(tag);
    let from_player = player.is_some() && owner == player;
    let mut p = Projectile::new(kind, at, from_player.then_some(Who::Player));
    p.velocity = motion(tag);
    [p.yaw, p.pitch] = rotation(tag);
    p.owner_uuid = owner;
    projectile_fields::load_projectile_flags_and_entity_fields(&mut p, tag);
    p.ground.on_ground = flag(tag, "OnGround");
    Some(p)
}

/// The save of [`spit_or_bullet_from_nbt`]'s kinds: `Projectile`'s keys, and
/// for a bullet the `Target`, `Dir`, `Steps` and target deltas vanilla writes.
pub fn spit_or_bullet_to_nbt(
    p: &Projectile,
    uuid: [i32; 4],
    owner: Option<[i32; 4]>,
    target: Option<[i32; 4]>,
    raw: Option<&Tag>,
) -> Option<Tag> {
    if !matches!(
        p.kind,
        ProjectileKind::LlamaSpit | ProjectileKind::ShulkerBullet { .. }
    ) {
        return None;
    }
    let base = Base {
        id: p.kind.id(),
        uuid,
        position: p.position,
        motion: p.velocity,
        rotation: [p.yaw, p.pitch],
        on_ground: p.ground.on_ground,
        fall_distance: p.fall_distance,
    };
    let mut tag = base.tag(raw);
    put_projectile(&mut tag, p, owner);
    put_projectile_entity_fields(&mut tag, p);
    // `ShulkerBullet.addAdditionalSaveData`: the target it homes on (written
    // only while it has one), the step it is on (omitted while it has not
    // chosen one), the steps left and the per-axis target deltas.
    if let ProjectileKind::ShulkerBullet {
        dir, steps, delta, ..
    } = &p.kind
    {
        if let Tag::Compound(map) = &mut tag {
            map.remove(&"Target".into());
            map.remove(&"Dir".into());
        }
        if let Some(target) = target {
            put(&mut tag, "Target", uuid_tag(target));
        }
        if let Some(dir) = dir {
            put(&mut tag, "Dir", Tag::Byte(*dir as i8));
        }
        put(&mut tag, "Steps", Tag::Int(*steps));
        for (key, v) in [("TXD", delta[0]), ("TYD", delta[1]), ("TZD", delta[2])] {
            put(&mut tag, key, Tag::Double(v.to_bits()));
        }
    }
    Some(tag)
}

/// `ShulkerBullet.readAdditionalSaveData`'s `Target`: the UUID of the entity
/// the bullet homes on, which vanilla keeps even while that entity is away.
pub fn bullet_target(tag: &Tag) -> Option<[i32; 4]> {
    uuid_from_tag(tag.get("Target"))
}

/// `Projectile`'s `Owner`.
pub fn owner_uuid(tag: &Tag) -> Option<[i32; 4]> {
    uuid_from_tag(tag.get("Owner"))
}

/// `Fireball.readAdditionalSaveData` (and, around it, `Projectile`'s Owner and
/// LeftOwner and `AbstractHurtingProjectile`'s acceleration power): a fireball
/// also reads the item it is drawn as through `setItem`, whose default is the
/// fire charge; a wind charge is no `Fireball` and has none.
pub fn fireball_from_nbt(tag: &Tag, player: Option<[i32; 4]>) -> Option<Projectile> {
    let kind = match kind(tag)?.as_str() {
        "minecraft:fireball" => ProjectileKind::LargeFireball {
            power: i32::from(number(tag.get("ExplosionPower")).unwrap_or(1) as i8),
        },
        "minecraft:small_fireball" => ProjectileKind::SmallFireball,
        "minecraft:dragon_fireball" => ProjectileKind::DragonFireball,
        "minecraft:wind_charge" => ProjectileKind::WindCharge,
        "minecraft:breeze_wind_charge" => ProjectileKind::BreezeWindCharge,
        // WitherSkull.readAdditionalSaveData: an absent `dangerous` is a black skull.
        "minecraft:wither_skull" => ProjectileKind::WitherSkull {
            dangerous: flag(tag, "dangerous"),
        },
        _ => return None,
    };
    let owner = uuid_from_tag(tag.get("Owner"));
    let from_player = player.is_some() && owner == player;
    let mut p = Projectile::new(kind, position(tag)?, from_player.then_some(Who::Player));
    p.velocity = motion(tag);
    [p.yaw, p.pitch] = loaded_rotation(tag);
    p.set_old_pos_and_rot();
    p.left_owner = flag(tag, "LeftOwner");
    p.has_been_shot = flag(tag, "HasBeenShot");
    p.owner_uuid = owner;
    p.no_gravity = flag(tag, "NoGravity");
    if projectile_fields::keeps_entity_fields(&p) {
        load_projectile_entity_fields(&mut p, tag);
    }
    p.ground.on_ground = flag(tag, "OnGround");
    p.fall_distance = float(tag.get("fall_distance")).unwrap_or(0.0);
    p.acceleration = float(tag.get("acceleration_power")).unwrap_or(0.1);
    if let Some(item) = tag.get("Item").and_then(item_from_nbt) {
        p.set_item(item);
    }
    Some(p)
}

/// `FireworkRocketEntity.readAdditionalSaveData`: Life, LifeTime (0 when
/// absent: a bare `/summon` explodes at once), FireworksItem and
/// ShotAtAngle, with `Projectile`'s Owner and LeftOwner.
pub struct RocketNbt {
    pub position: Vec3,
    pub motion: Vec3,
    pub life: i32,
    pub lifetime: i32,
    pub item: Option<crate::items::ItemStack>,
    pub angled: bool,
    pub owner: Option<Who>,
    /// The saved `Owner`, whoever it names.
    pub owner_uuid: Option<[i32; 4]>,
    pub left_owner: bool,
    pub on_ground: bool,
    /// `Entity.load`'s Rotation.
    pub rotation: [f32; 2],
    /// `Projectile.hasBeenShot`: false until its first tick posts `projectile_shoot`.
    pub has_been_shot: bool,
    /// `Entity.isSilent`: no launch sound.
    pub silent: bool,
    /// `Entity.load`'s fall_distance.
    pub fall_distance: f64,
}
pub fn firework_rocket_from_nbt(tag: &Tag, player: Option<[i32; 4]>) -> Option<RocketNbt> {
    let owner = uuid_from_tag(tag.get("Owner"));
    Some(RocketNbt {
        position: position(tag)?,
        motion: motion(tag),
        life: number(tag.get("Life")).unwrap_or(0) as i32,
        lifetime: number(tag.get("LifeTime")).unwrap_or(0) as i32,
        item: tag.get("FireworksItem").and_then(item_from_nbt),
        angled: flag(tag, "ShotAtAngle"),
        owner: (player.is_some() && owner == player).then_some(Who::Player),
        owner_uuid: owner,
        left_owner: flag(tag, "LeftOwner"),
        on_ground: flag(tag, "OnGround"),
        rotation: rotation(tag),
        has_been_shot: flag(tag, "HasBeenShot"),
        silent: flag(tag, "Silent"),
        fall_distance: float(tag.get("fall_distance")).unwrap_or(0.0),
    })
}

/// What `FireworkRocketEntity.addAdditionalSaveData` writes, over `Projectile`'s
/// Owner and LeftOwner.
pub struct RocketSave<'a> {
    pub rotation: [f32; 2],
    pub position: Vec3,
    pub motion: Vec3,
    pub life: i32,
    pub lifetime: i32,
    pub item: &'a crate::items::ItemStack,
    pub angled: bool,
    pub owner: Option<[i32; 4]>,
    pub left_owner: bool,
    /// `Entity.onGround`, from the rocket's last `Entity.move`.
    pub on_ground: bool,
    pub has_been_shot: bool,
    /// `Entity.fallDistance`.
    pub fall_distance: f64,
}
pub fn firework_rocket_to_nbt(
    r: &RocketSave,
    uuid: [i32; 4],
    raw: Option<&Tag>,
    items: &crate::items::Items,
) -> Tag {
    let mut tag = Base {
        id: "minecraft:firework_rocket",
        uuid,
        position: r.position,
        motion: r.motion,
        rotation: r.rotation,
        on_ground: r.on_ground,
        fall_distance: r.fall_distance,
    }
    .tag(raw);
    for (key, value) in [
        ("Life", Tag::Int(r.life)),
        ("LifeTime", Tag::Int(r.lifetime)),
        ("FireworksItem", item_to_nbt(r.item, items)),
        ("ShotAtAngle", Tag::Byte(i8::from(r.angled))),
        ("HasBeenShot", Tag::Byte(i8::from(r.has_been_shot))),
    ] {
        put(&mut tag, key, value);
    }
    if r.left_owner {
        put(&mut tag, "LeftOwner", Tag::Byte(1));
    }
    if let Some(owner) = r.owner {
        put(&mut tag, "Owner", Tag::IntArray(owner.to_vec()));
    }
    tag
}
