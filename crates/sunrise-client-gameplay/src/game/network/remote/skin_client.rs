use super::*;

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    pub(in crate::game) fn commit_head_skin_journal(
        &self,
        journal: sunrise_client::skins::HeadSkinJournal,
    ) {
        journal.commit(
            &mut self.client.skin_profiles.borrow_mut(),
            &mut self.client.skin_downloads.0.borrow_mut(),
        );
    }

    /// Whether a downloaded skin or cape is in the atlas to draw, marking it
    /// drawn; if not (new, or its slot taken), its image is queued for upload
    /// again and it is drawn from the next frame.
    pub(in crate::game) fn again_skin(&self, location: &str) -> bool {
        self.again_skin_with_journal(location, None)
    }
    fn again_skin_with_journal(
        &self,
        location: &str,
        journal: Option<&sunrise_client::skins::HeadSkinJournal>,
    ) -> bool {
        // A slot without its sprite (the atlas was rebuilt, or the upload
        // never happened) counts as missing, so the skin is stored again.
        let present = self.client.skin_slots.contains(location)
            && self.client.gui.atlas.sprites.contains_key(location);
        if let Some(journal) = journal {
            if present {
                journal.record_drawn(location);
            } else {
                journal.record_again(location);
            }
        } else {
            let mut downloads = self.client.skin_downloads.0.borrow_mut();
            if present {
                downloads.drawn(location);
            } else {
                downloads.again(location);
            }
        }
        present
    }
    /// A mod's `GuiCanvas::player_face` skin (`skins::face_skin`): a player with a `PlayerInfo`
    /// on this connection draws its skin as the player list does; any other UUID its profile,
    /// looked up by id as a head's is. Until uploaded, the UUID's default. It borrows the name: a
    /// looked-up skin's is copied into `name`, reused from face to face.
    pub(in crate::game) fn player_face_skin_with_journal<'a>(
        &'a self,
        uuid: u128,
        journal: Option<&sunrise_client::skins::HeadSkinJournal>,
        name: &'a mut String,
    ) -> &'a str {
        let listed = self
            .client
            .remote
            .as_ref()
            .filter(|r| r.tab_players.contains_key(&uuid))
            .map(|r| {
                let skin = r.skins.get(&uuid).map_or_else(
                    || sunrise_level::profile_lookup::default_skin(uuid),
                    |skin| skin.body(uuid).0,
                );
                self.shown_face_with_journal(uuid, skin, journal)
            });
        sunrise_client::skins::face_skin_with_journal(
            uuid,
            listed,
            &mut self.client.skin_profiles.borrow_mut(),
            &mut self.client.skin_downloads.0.borrow_mut(),
            |location| self.client.gui.atlas.sprites.contains_key(location),
            journal,
            name,
        )
    }
    /// Resolve inline `object:player` profiles through the same skin cache as
    /// player heads on items. The next frame uses the uploaded atlas texture.
    pub(in crate::game::client_side) fn resolve_inline_profiles(
        &mut self,
        profiles: &[std::sync::Arc<serde_json::Value>],
    ) {
        use sunrise_client::skins;
        for profile in profiles {
            let skin = skins::head_skin(
                profile,
                &mut self.client.skin_profiles.borrow_mut(),
                &mut self.client.skin_downloads.0.borrow_mut(),
                |location| self.client.gui.atlas.sprites.contains_key(location),
            );
            let id = skins::parse_profile(profile).id;
            let name = profile
                .as_str()
                .or_else(|| profile.get("name").and_then(serde_json::Value::as_str));
            // The player list's skin for the same profile is drawn too.
            let keys = [
                Some(format!("id:{id:032x}")),
                name.map(|n| format!("name:{}", n.to_ascii_lowercase())),
            ];
            for key in keys.iter().flatten() {
                if let Some(listed) = self.client.gui.atlas.object_skins.get(key)
                    && listed.starts_with("skins/")
                {
                    let listed = listed.clone();
                    self.again_skin(&listed);
                }
            }
            self.client
                .gui
                .atlas
                .set_object_skin(format!("id:{id:032x}"), skin.clone());
            if let Some(name) = name {
                self.client
                    .gui
                    .atlas
                    .set_object_skin(format!("name:{}", name.to_ascii_lowercase()), skin);
            }
        }
    }
    /// Puts a downloaded skin in an atlas skin slot (`SkinSlots::place`),
    /// unless every slot holds a skin drawn in the last frame.
    pub(in crate::game::client_side) fn store_skin(&mut self, location: String, rgba: Vec<u8>) {
        let Some((slot, evicted)) = self.client.skin_slots.place(location.clone()) else {
            return;
        };
        if let Some(old) = evicted {
            self.client.gui.atlas.sprites.remove(&old);
        }
        let Some(mut rect) = self
            .client
            .gui
            .atlas
            .sprites
            .get(&format!("sunrise/skin{slot}"))
            .copied()
        else {
            return;
        };
        let (w, h) = (
            self.client.gui.atlas.width as f32,
            self.client.gui.atlas.height as f32,
        );
        self.client.map_patches.push(sunrise_client::font::Patch {
            x: (rect.u0 * w).round() as u32,
            y: (rect.v0 * h).round() as u32,
            width: 64,
            height: 64,
            rgba,
        });
        // A cape is a 64x32 texture in the top half of the slot (skins::process_cape).
        if location.starts_with("capes/") {
            rect.v1 = (rect.v0 + rect.v1) / 2.0;
        }
        self.client.gui.atlas.sprites.insert(location, rect);
    }

    /// The player list's view of the connection: the listed players (with
    /// their downloaded skins once ready), online mode, and which players
    /// are in the level with an upside-down name.
    pub(in crate::game) fn publish_tab_object_skins(&mut self) {
        let Some(r) = self.client.remote.as_mut() else {
            return;
        };
        let fingerprint = (
            r.listed.len(),
            r.listed.iter().fold(0, |xor, uuid| xor ^ uuid),
            r.skins_revision,
            self.client.gui.atlas.object_skins_revision,
        );
        if r.published_skins == Some(fingerprint) {
            return;
        }
        for uuid in &r.listed {
            let Some(p) = r.tab_players.get(uuid) else {
                continue;
            };
            let skin = r
                .skins
                .get(uuid)
                .map(|skin| skin.body(*uuid).0)
                .unwrap_or_else(|| sunrise_level::profile_lookup::default_skin(*uuid));
            for key in [
                format!("id:{uuid:032x}"),
                format!("name:{}", p.name.to_ascii_lowercase()),
            ] {
                if object_skin_changed(&self.client.gui.atlas.object_skins, &key, skin) {
                    self.client.gui.atlas.set_object_skin(key, skin.to_owned());
                }
            }
        }
        // Publication itself mutates the atlas; cache its final revision.
        r.published_skins = Some((
            fingerprint.0,
            fingerprint.1,
            fingerprint.2,
            self.client.gui.atlas.object_skins_revision,
        ));
    }

    pub(in crate::game) fn poll_own_profile(&mut self) {
        let auth = self.auth();
        let Some(r) = self.client.remote.as_mut() else {
            return;
        };
        match &mut r.own_profile {
            OwnProfile::Idle => {
                let Some(auth) = auth else {
                    r.own_profile = OwnProfile::Done(None);
                    return;
                };
                let (send, recv) = std::sync::mpsc::channel();
                std::thread::Builder::new()
                    .name("Own profile".into())
                    .spawn(move || {
                        let _ = send.send(sunrise_client::auth::own_textures(auth.as_ref()));
                    })
                    .ok();
                r.own_profile = OwnProfile::Fetching(recv);
            }
            OwnProfile::Fetching(recv) => {
                if let Ok(own) = recv.try_recv() {
                    r.own_profile = OwnProfile::Done(own);
                }
            }
            OwnProfile::Done(_) => {}
        }
    }

    /// Whether `skin` is a downloaded skin not yet in the atlas ([`Self::again_skin`]).
    fn skin_pending_with_journal(
        &self,
        skin: &str,
        journal: Option<&sunrise_client::skins::HeadSkinJournal>,
    ) -> bool {
        (skin.starts_with("skins/") || skin == "sunrise/offline_skin")
            && !self.again_skin_with_journal(skin, journal)
    }

    /// A face in the player list or spectator menu as drawn, borrowed: `skin`, or the
    /// UUID's default (`DefaultPlayerSkin.get`) while the downloaded skin is not yet in the
    /// atlas, as `PlayerInfo.getSkin` gives before the texture has loaded.
    pub(in crate::game) fn shown_face<'s>(&self, uuid: u128, skin: &'s str) -> &'s str {
        self.shown_face_with_journal(uuid, skin, None)
    }

    fn shown_face_with_journal<'s>(
        &self,
        uuid: u128,
        skin: &'s str,
        journal: Option<&sunrise_client::skins::HeadSkinJournal>,
    ) -> &'s str {
        match self.skin_pending_with_journal(skin, journal) {
            true => sunrise_level::profile_lookup::default_skin(uuid),
            false => skin,
        }
    }

    /// [`Self::shown_face`], owned.
    pub(in crate::game) fn face_skin(&self, uuid: u128, skin: &str) -> String {
        self.shown_face(uuid, skin).to_owned()
    }
}
