use super::*;
use sunrise_client::skins;

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    pub(in crate::game::client_side) fn remote_status(&self) -> Option<serde_json::Value> {
        let r = self.client.remote.as_ref()?;
        Some(serde_json::json!({
            "address": sunrise_client::net::shown_address(&r.session.address),
            "name": r.session.name,
            "joined": r.joined,
            // ServerboundPlayerLoadedPacket sent: the level around the
            // player is ready, and the client ticks and sends its moves.
            "loaded": r.loaded_sent,
            "status": r.status,
            "entity_id": r.entity_id,
            "dimension": r.dimension.name,
            "game_mode": r.game_mode,
            "chunks": self.world.chunks.len(),
            "entities": r.entities.values().fold(std::collections::BTreeMap::<&str, u32>::new(), |mut m, t| {
                *m.entry(t.kind.as_str()).or_default() += 1;
                m
            }),
            "players": r.players.values().collect::<Vec<_>>(),
            // Where the other players' entities are (harnesses check movement).
            // What the other players hold and wear, as this client draws them.
            "player_equipment": self
                .mobs
                .iter()
                .filter(|m| r.level_players.contains_key(&(m.id as i32)))
                .map(|m| {
                    let e = &m.extra;
                    let armor = sunrise_items::equipment::ARMOR.map(|slot| e.slot_id(slot));
                    serde_json::json!({
                        "id": m.id,
                        "swinging": m.ai.swinging,
                        "using": m.extra.using_item,
                        "armor": armor,
                        "main_hand": e.main_hand_id(),
                        "off_hand": e.off_hand_id(),
                    })
                })
                .collect::<Vec<_>>(),
            "player_positions": r
                .entities
                .values()
                .filter(|t| t.kind == "minecraft:player")
                .map(|t| t.position)
                .collect::<Vec<_>>(),
            // The other players this client draws dead (lying down, after EntityEvent DEATH).
            "dead_players": self
                .mobs
                .iter()
                .filter(|m| r.level_players.contains_key(&(m.id as i32)) && !m.alive())
                .map(|m| m.id)
                .collect::<Vec<_>>(),
            // The other players this client draws lying in a bed (SLEEPING_POS).
            "lying_players": self
                .mobs
                .iter()
                .filter(|m| r.level_players.contains_key(&(m.id as i32)) && m.extra.ints[1] != 0)
                .map(|m| m.id)
                .collect::<Vec<_>>(),
            // The player's own model exists (third person and the inventory draw it).
            "player_model": self.client.player_model.is_some(),
            // The world view draws its body now (third person, or lying in a bed;
            // not while invisible).
            "own_body": self.player_model_shown() && self.own_body_visible(),
            "text_display_vertices": self.client.text_display_vertices,
            "text_displays": self.client.text_displays.0.len(),
            "health": r.health.0,
            "food": r.health.1,
            "decode_errors": r.decode_errors,
            "chat": r.chat.iter().rev().take(10).rev().map(|(t, _)| t.clone()).collect::<Vec<_>>(),
        }))
    }
    /// `PlayerInfo.getSkin` for every player entity, including NPCs that are
    /// never in the tab list: resolve each profile's textures and give the
    /// level's player models their skin.
    pub(in crate::game) fn update_remote_skins(&mut self) {
        self.poll_own_profile();
        let Some(r) = self.client.remote.as_mut() else {
            return;
        };
        let local = r.profile;
        let level: Vec<_> = r
            .level_players
            .iter()
            .map(|(id, uuid)| (*id as u32, *uuid))
            .collect();
        let listed = skin_entries(
            &r.level_players,
            &r.skin_textures,
            &mut r.level_player_textures,
        );
        // Unpacked and verified on the skin downloader's threads: the game
        // thread reads answers and starts the rest (`SkinManager.getOrLoad`).
        let entries: Vec<(u128, Arc<skins::Textures>)> = {
            let keys = r.services_keys.lock().unwrap_or_else(|e| e.into_inner());
            let mut downloads = self.client.skin_downloads.0.borrow_mut();
            listed
                .iter()
                .filter_map(|(uuid, property)| {
                    property
                        .unpack(&mut downloads, &keys, *uuid != local)
                        .map(|textures| (*uuid, textures))
                })
                .collect()
        };
        // Minecraft.profileFuture: the signed-in account's active skin and cape
        // when the server sent none (single player), or only a skin without
        // the equipped cape: the account's skin, with its model, where the server
        // sent none, and its cape where the server sent none.
        let own = match &r.own_profile {
            OwnProfile::Done(Some(own)) => match entries.iter().find(|(u, _)| *u == local) {
                // Each profile's textures stay behind their `Arc`: only the
                // signed-in account's merge is copied.
                Some((_, textures)) => {
                    let mut merged = textures.as_ref().clone();
                    if merged.skin.is_none() {
                        merged.skin.clone_from(&own.skin);
                        if own.skin.is_some() {
                            merged.slim = own.slim;
                        }
                    }
                    if merged.cape.is_none() {
                        merged.cape.clone_from(&own.cape);
                    }
                    Some(merged)
                }
                None => (own.skin.is_some() || own.cape.is_some()).then(|| own.clone()),
            },
            _ => None,
        };
        let mut skins = HashMap::new();
        {
            // PlayerInfo.createSkinLookup: only the local player's own skin
            // may be unsigned.
            let mut downloads = self.client.skin_downloads.0.borrow_mut();
            let mut resolve = |uuid, textures: &skins::Textures| {
                if let skins::SkinLookup::Ready(skin) =
                    skins::lookup(textures, uuid != local, |kind, url| {
                        downloads.state(kind, url)
                    })
                    && skin != skins::ResolvedSkin::default()
                {
                    skins.insert(uuid, skin);
                }
            };
            for (uuid, textures) in entries.iter().filter(|(u, _)| own.is_none() || *u != local) {
                resolve(*uuid, textures);
            }
            if let Some(textures) = &own {
                resolve(local, textures);
            }
        }
        // An offline name's skin from the launcher, where neither the server
        // nor an account gives the local player textures.
        if !skins.contains_key(&local)
            && let Some((rgba, is_slim)) = skins::offline_skin()
        {
            const OFFLINE: &str = "sunrise/offline_skin";
            if !self.client.skin_slots.contains(OFFLINE)
                || !self.client.gui.atlas.sprites.contains_key(OFFLINE)
            {
                self.store_skin(OFFLINE.into(), rgba.clone());
            }
            skins.entry(local).or_default().custom_body = Some((OFFLINE.into(), *is_slim));
        }
        // An offline name's cape from the launcher, where the profile has none.
        if skins.get(&local).is_none_or(|s| s.cape.is_none())
            && let Some(rgba) = skins::offline_cape()
        {
            const OFFLINE: &str = "capes/offline";
            if !self.client.skin_slots.contains(OFFLINE)
                || !self.client.gui.atlas.sprites.contains_key(OFFLINE)
            {
                // Kept for `again`, which finds no download or disk file for this cape.
                self.client
                    .skin_downloads
                    .0
                    .borrow_mut()
                    .remember_image(OFFLINE, rgba);
                self.store_skin(OFFLINE.into(), rgba.to_vec());
            }
            skins.entry(local).or_default().cape = Some(OFFLINE.into());
        }
        // Only the skins drawn hold atlas slots: the level's players, not
        // every entry of a busy server's player list.
        // Build once: resolving each player must not scan every level entity.
        let players: HashMap<_, _> = self
            .common
            .mobs
            .iter()
            .enumerate()
            .filter(|(_, m)| &*m.kind.id == "minecraft:player")
            .map(|(i, m)| (m.id, i))
            .collect();
        for (id, uuid) in level {
            let Some(&i) = players.get(&id) else {
                continue;
            };
            let (skin, is_slim, cape) =
                skins::drawable_skin(uuid, skins.get(&uuid), true, |sprite| {
                    self.again_skin(sprite)
                });
            let mob = &mut self.common.mobs[i];
            mob.skin = Some(skin.to_owned());
            mob.slim = is_slim;
            mob.cape = cape.map(str::to_owned);
        }
        if self.client.player_model.is_some() {
            // Avatar.isModelPartShown(CAPE): the Skin Customization option.
            let show_cape = self.client.settings.file.bool("modelPart_cape");
            let (skin, is_slim, cape) =
                skins::drawable_skin(local, skins.get(&local), show_cape, |sprite| {
                    self.again_skin(sprite)
                });
            if let Some(mob) = self.client.player_model.as_mut() {
                mob.skin = Some(skin.to_owned());
                mob.slim = is_slim;
                mob.cape = cape.map(str::to_owned);
            }
        }
        if let Some(r) = self.client.remote.as_mut()
            && r.skins != skins
        {
            r.skins_revision = r.skins_revision.wrapping_add(1);
            r.skins = skins;
        }
        // ClientMannequin.updateSkin (PlayerSkinRenderCache.lookup): a
        // mannequin's DATA_PROFILE `textures` property resolves like a
        // player's, never required to be signed.
        let mannequins: Vec<(usize, sunrise_client::skins::Textures)> = self
            .common
            .mobs
            .iter()
            .enumerate()
            .filter(|(_, m)| &*m.kind.id == "minecraft:mannequin" && m.profile.is_some())
            .map(|(i, m)| {
                let profile = m.profile.as_ref();
                let t = profile.and_then(|p| p.properties.iter().find(|p| p.name == "textures"));
                let textures = if let Some(t) = t {
                    skins::unpack_insecure(&t.value)
                } else {
                    // `ResolvableProfile.resolveProfile`: no `textures` and
                    // only a name or an id resolves through the services.
                    let key = profile.and_then(|p| match (&p.name, p.uuid) {
                        (Some(name), None) => {
                            Some(sunrise_client::skins::NameOrId::Name(name.clone()))
                        }
                        (None, Some(id)) => Some(sunrise_client::skins::NameOrId::Id(id)),
                        _ => None,
                    });
                    key.and_then(
                        |key| match self.client.skin_profiles.borrow_mut().state(&key) {
                            sunrise_client::skins::Resolve::Done(Some(profile)) => {
                                profile.textures().map(skins::unpack_insecure)
                            }
                            _ => None,
                        },
                    )
                    .unwrap_or_default()
                };
                (i, textures)
            })
            .collect();
        let mut resolved: Vec<(usize, Option<String>, bool)> = Vec::new();
        for (id, textures) in mannequins {
            let mut downloads = self.client.skin_downloads.0.borrow_mut();
            let (skin, slim) =
                match skins::lookup(&textures, false, |kind, url| downloads.state(kind, url)) {
                    skins::SkinLookup::Ready(skin) => skin.custom_body.unzip(),
                    _ => (None, None),
                };
            resolved.push((id, skin, slim.unwrap_or(false)));
        }
        for (i, skin, slim) in resolved {
            let skin = skin.filter(|s| !s.starts_with("skins/") || self.again_skin(s));
            // Resolution and atlas uploads do not insert or remove mobs.
            let mob = &mut self.common.mobs[i];
            mob.slim = slim && skin.is_some();
            mob.skin = skin;
        }
    }
}
