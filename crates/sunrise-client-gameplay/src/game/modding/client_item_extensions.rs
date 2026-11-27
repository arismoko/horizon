//! Reload-owned item providers and baked humanoid equipment.
use super::*;
use mod_api::__private::game::{CallbackOwner, CallbackValue};
use mod_api::{
    ArmorModelRef, ArmorTextureRef, ClientItemExtensions, ClientItemExtensionsRegistrar, ItemKey,
    ModelPart,
};
use std::{
    borrow::Cow,
    collections::hash_map::Entry,
    sync::{
        Arc, LazyLock,
        atomic::{AtomicU64, Ordering},
    },
};
use sunrise_client::mobs::render::EquipmentModel;
use sunrise_level::fast_hash::FxHashMap;
use sunrise_sim::mobs::finalize::EquipmentRead;

pub static DEFAULT_TOOLTIP_FONT_ID: LazyLock<mod_api::Identifier> = LazyLock::new(|| {
    mod_api::Identifier::from_parts("minecraft", "default")
        .expect("minecraft:default is a valid identifier")
});

/// One callback owner per mod and item, shared by all generations of that provider.
#[derive(Default)]
pub struct ItemExtensionOwners(FxHashMap<(&'static str, ItemKey), CallbackOwner>);
impl ItemExtensionOwners {
    pub(in crate::game) fn owner_for(
        &mut self,
        mod_id: &'static str,
        item: ItemKey,
    ) -> Option<CallbackOwner> {
        match self.0.entry((mod_id, item)) {
            Entry::Occupied(entry) => Some(entry.get().clone()),
            Entry::Vacant(entry) => Some(entry.insert(CallbackOwner::new(mod_id)?).clone()),
        }
    }
}

pub(in crate::game) type ItemExtensionsPart = CallbackValue<Arc<dyn ClientItemExtensions>>;

pub(in crate::game) struct ArmorModel {
    pub name: String,
    pub geometry: Arc<EquipmentModel>,
}

pub(in crate::game) struct ArmorTexture {
    pub name: String,
    pub uv: [f32; 4],
}

pub struct ArmorExtensions {
    pub generation: u64,
    pub(in crate::game) models: Vec<ArmorModel>,
    pub(in crate::game) textures: Vec<ArmorTexture>,
    pub(in crate::game) items: FxHashMap<ItemKey, ItemExtensionsPart>,
}
impl Default for ArmorExtensions {
    fn default() -> Self {
        static GENERATION: AtomicU64 = AtomicU64::new(1);
        Self::empty(GENERATION.fetch_add(1, Ordering::Relaxed))
    }
}
impl ArmorExtensions {
    fn empty(generation: u64) -> Self {
        Self {
            generation,
            models: Vec::new(),
            textures: Vec::new(),
            items: FxHashMap::default(),
        }
    }
}

pub(in crate::game) struct Registration<'a> {
    state: ArmorExtensions,
    texture_keys: FxHashMap<String, u32>,
    published: Option<(&'a ArmorExtensions, &'a FxHashMap<String, u32>)>,
    sprite_uv: &'a dyn Fn(&str) -> Option<[f32; 4]>,
    owners: &'a mut ItemExtensionOwners,
}
impl<'a> Registration<'a> {
    fn new(
        sprite_uv: &'a dyn Fn(&str) -> Option<[f32; 4]>,
        owners: &'a mut ItemExtensionOwners,
    ) -> Self {
        Self {
            state: ArmorExtensions::default(),
            texture_keys: FxHashMap::default(),
            published: None,
            sprite_uv,
            owners,
        }
    }

    pub(in crate::game) fn register_listener(
        &mut self,
        listen: &mut dyn FnMut(&mut dyn ClientItemExtensionsRegistrar) -> bool,
    ) {
        let mut pending = Registration {
            state: ArmorExtensions::empty(self.state.generation),
            texture_keys: FxHashMap::default(),
            published: Some((&self.state, &self.texture_keys)),
            sprite_uv: self.sprite_uv,
            owners: &mut *self.owners,
        };
        // The shared dispatch boundary rejects an unwound listener. Returned registration
        // errors have already left the pending edits intact and do not veto its batch.
        if listen(&mut pending) {
            let Registration {
                state,
                texture_keys,
                ..
            } = pending;
            self.state.models.extend(state.models);
            self.state.textures.extend(state.textures);
            self.state.items.extend(state.items);
            self.texture_keys.extend(texture_keys);
        }
    }
}
impl ClientItemExtensionsRegistrar for Registration<'_> {
    fn register_model(
        &mut self,
        name: &str,
        model: &ModelPart,
    ) -> std::result::Result<ArmorModelRef, String> {
        mod_api::Identifier::parse(name).map_err(|why| format!("armour model {name}: {why}"))?;
        let published = self.published.map(|(state, _)| &state.models);
        if published
            .into_iter()
            .flatten()
            .chain(&self.state.models)
            .any(|model| model.name == name)
        {
            return Err(format!("duplicate armour model {name}"));
        }
        let baked =
            EquipmentModel::bake(model).map_err(|why| format!("armour model {name}: {why}"))?;
        let index = u32::try_from(published.map_or(0, Vec::len) + self.state.models.len())
            .map_err(|_| format!("armour model {name}: model table exhausted"))?;
        self.state.models.push(ArmorModel {
            name: name.to_owned(),
            geometry: Arc::new(baked),
        });
        Ok(ArmorModelRef::__new(self.state.generation, index))
    }
    fn register_texture(&mut self, name: &str) -> std::result::Result<ArmorTextureRef, String> {
        if let Some(&index) = self
            .published
            .and_then(|(_, keys)| keys.get(name))
            .or_else(|| self.texture_keys.get(name))
        {
            return Ok(ArmorTextureRef::__new(self.state.generation, index));
        }
        let uv = (self.sprite_uv)(name).ok_or_else(|| format!("unknown armour texture {name}"))?;
        let index = u32::try_from(
            self.published.map_or(0, |(state, _)| state.textures.len()) + self.state.textures.len(),
        )
        .map_err(|_| format!("armour texture {name}: texture table exhausted"))?;
        self.state.textures.push(ArmorTexture {
            name: name.to_owned(),
            uv,
        });
        self.texture_keys.insert(name.to_owned(), index);
        Ok(ArmorTextureRef::__new(self.state.generation, index))
    }
    fn client_item_extensions(
        &mut self,
        item: ItemKey,
        provider: Arc<dyn ClientItemExtensions>,
    ) -> std::result::Result<(), String> {
        let mod_id = mod_api::content::crash::current_native_mod()
            .ok_or_else(|| "client item extensions: no registering provider".to_owned())?;
        let name = mod_api::__private::item_key_id(item)
            .ok_or_else(|| format!("unknown item for extension provider {mod_id}"))?;
        if let Some(previous) = self
            .published
            .and_then(|(state, _)| state.items.get(&item))
            .or_else(|| self.state.items.get(&item))
        {
            return Err(format!(
                "duplicate client item extensions for {name}: {} and {mod_id}",
                previous.owner().id()
            ));
        }
        let owner = self
            .owners
            .owner_for(mod_id, item)
            .ok_or_else(|| format!("unknown item extension provider {mod_id}"))?;
        self.state.items.insert(
            item,
            CallbackValue::new(
                owner,
                "client item extensions",
                "client_item_extensions_drop",
                provider,
            ),
        );
        Ok(())
    }
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    pub fn prepare_client_item_extensions(
        gui: &UiResources,
        owners: &mut ItemExtensionOwners,
    ) -> ArmorExtensions {
        use mod_api::__private::game::RegistrationContextNew;
        let sprite_uv = |name: &str| {
            gui.atlas
                .sprites
                .get(name)
                .map(|r| [r.u0, r.v0, r.u1, r.v1])
        };
        let mut registration = Registration::new(&sprite_uv, owners);
        crate::game::client_side::native_hooks_client::register_client_item_extensions::native_register_client_item_extensions(
            &mod_api::RegistrationContext::new(),
            &mut registration,
        );
        registration.state
    }

    #[cfg(test)]
    fn resolve_client_item_extensions(&mut self) {
        self.client.armor_stand_assets.0.extensions = Self::prepare_client_item_extensions(
            &self.client.gui,
            &mut self.client.client_item_extension_owners,
        );
    }

    /// Item-owned tooltip font (`ClientHooks.getTooltipFont` -> `IClientItemExtensions.getFont`).
    ///
    /// Returns `minecraft:default` when no extension is registered, the provider returns `None`,
    /// the stack cannot cross the API boundary, or the provider panics.
    pub(in crate::game::client_side) fn item_tooltip_font(
        &self,
        stack: &sunrise_items::items::ItemStack,
    ) -> Cow<'static, mod_api::Identifier> {
        let default = Cow::Borrowed(&*DEFAULT_TOOLTIP_FONT_ID);
        let state = &self.client.armor_stand_assets.0.extensions;
        if state.items.is_empty() {
            return default;
        }
        let Some(item) = mod_api::__private::item_key(&stack.item) else {
            return default;
        };
        let Some(part) = state.items.get(&item) else {
            return default;
        };
        if !part.owner().is_enabled() {
            return default;
        }
        let Ok(api_stack) =
            sunrise_gameplay_common::game::api_values::TryIntoApi::try_into_api(stack)
        else {
            return default;
        };
        part.owner()
            .call("client item extensions", "tooltip_font", || {
                part.value().tooltip_font(self, &api_stack)
            })
            .ok()
            .flatten()
            .map_or(default, Cow::Owned)
    }

    /// Whether a dropped stack bobs and whether its copies spread
    /// (`IClientItemExtensions.shouldBobAsEntity` and `shouldSpreadAsEntity`).
    ///
    /// Both are `true` when no extension is registered, the stack cannot cross the API boundary,
    /// or the provider panics.
    pub(in crate::game::client_side) fn dropped_stack_motion(
        &self,
        stack: &sunrise_items::items::ItemStack,
    ) -> DroppedStackMotion {
        use sunrise_gameplay_common::game::api_values::TryIntoApi;
        let state = &self.client.armor_stand_assets.0.extensions;
        if state.items.is_empty() {
            return DroppedStackMotion::VANILLA;
        }
        let Some(part) =
            mod_api::__private::item_key(&stack.item).and_then(|item| state.items.get(&item))
        else {
            return DroppedStackMotion::VANILLA;
        };
        if !part.owner().is_enabled() {
            return DroppedStackMotion::VANILLA;
        }
        let Ok(api_stack) = stack.try_into_api() else {
            return DroppedStackMotion::VANILLA;
        };
        dispatch_dropped_stack_motion(self, part, &api_stack)
    }

    pub(in crate::game::client_side) fn extended_equipment_materials<'a>(
        &'a self,
        piece: &super::equipment_piece::EquipmentPiece<'_>,
        stack: &EquipmentRead<'_>,
        trim: Option<super::equipment_piece::MaterialBinding<'a>>,
        mut texture: impl FnMut(&str) -> Option<super::equipment_piece::MaterialBinding<'a>>,
        mut emissive: impl FnMut(&str) -> Option<super::equipment_piece::MaterialBinding<'a>>,
    ) -> Option<Vec<super::equipment_piece::EquipmentMaterial<'a>>> {
        let state = &self.client.armor_stand_assets.0.extensions;
        if state.items.is_empty() {
            return None;
        }
        let metadata = sunrise_gameplay_common::game::api_values::stack_read_metadata(stack.base());
        let item = match &metadata {
            Ok((item, ..)) => *item,
            // Only warn about a refused stack when it would have reached an active provider.
            Err(_) => mod_api::__private::item_key(&stack.base().item)?,
        };
        let part = state.items.get(&item)?;
        if !part.owner().is_enabled() {
            return None;
        }
        let metadata = match metadata {
            Ok(metadata) => metadata,
            Err(why) => {
                log::warn!(
                    "Sunrise: client item extensions for {}: invalid equipped stack: {why:?}",
                    part.owner().id()
                );
                return None;
            }
        };
        sunrise_gameplay_common::game::native_equipment_read::with_equipment_view(
            stack,
            metadata,
            |view, _| {
                dispatch_equipment(
                    self,
                    state,
                    part,
                    &view,
                    piece,
                    EquipmentBindings {
                        trim,
                        default: &mut texture,
                        emissive: &mut emissive,
                    },
                )
            },
        )
    }
}

/// How a dropped stack moves and spreads, as its item's client extensions answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::game::client_side) struct DroppedStackMotion {
    /// `shouldBobAsEntity`: the stack bobs up and down as it spins.
    pub bob: bool,
    /// `shouldSpreadAsEntity`: a stack's copies are drawn apart.
    pub spread: bool,
}
impl DroppedStackMotion {
    /// Vanilla's dropped stack: it bobs and its copies spread.
    pub const VANILLA: Self = Self {
        bob: true,
        spread: true,
    };
}

type MotionAnswer =
    fn(&dyn ClientItemExtensions, &dyn mod_api::ClientHost, &mod_api::ItemStack) -> bool;
fn dispatch_dropped_stack_motion(
    host: &dyn mod_api::ClientHost,
    part: &ItemExtensionsPart,
    stack: &mod_api::ItemStack,
) -> DroppedStackMotion {
    let provider = &**part.value();
    let ask = |operation, answer: MotionAnswer| {
        part.owner()
            .call("client_item_extensions", operation, || {
                answer(provider, host, stack)
            })
            .unwrap_or(true)
    };
    DroppedStackMotion {
        bob: ask("should_bob_as_entity", |p, host, stack| {
            p.should_bob_as_entity(host, stack)
        }),
        spread: ask("should_spread_as_entity", |p, host, stack| {
            p.should_spread_as_entity(host, stack)
        }),
    }
}

type Binding<'a> = super::equipment_piece::MaterialBinding<'a>;
struct EquipmentBindings<'a, 'call> {
    trim: Option<Binding<'a>>,
    default: &'call mut dyn FnMut(&str) -> Option<Binding<'a>>,
    emissive: &'call mut dyn FnMut(&str) -> Option<Binding<'a>>,
}
fn dispatch_equipment<'a>(
    host: &dyn mod_api::ClientHost,
    state: &'a ArmorExtensions,
    part: &ItemExtensionsPart,
    stack: &mod_api::ItemStackView<'_>,
    piece: &super::equipment_piece::EquipmentPiece<'_>,
    lookups: EquipmentBindings<'a, '_>,
) -> Option<Vec<super::equipment_piece::EquipmentMaterial<'a>>> {
    if !part.owner().is_enabled() {
        return None;
    }
    use super::equipment_piece::ResolvedEquipmentLayer;
    use mod_api::{EquipmentLayer, EquipmentLayerType};
    let EquipmentBindings {
        trim,
        default: texture,
        emissive,
    } = lookups;
    let provider = part.value().as_ref();
    let report_stale = |kind: &str| {
        log::warn!(
            "Sunrise: stale armour {kind} reference from {}; using the vanilla binding",
            part.owner().id()
        );
    };
    // A panic drops the whole result, even if an earlier layer succeeded.
    part.owner()
        .call("client item extensions", "client_item_extensions", || {
            let dye = provider.default_dye_color(host, stack);
            let layer_type = match piece.kind {
                "humanoid_baby" => EquipmentLayerType::HumanoidBaby,
                "humanoid_leggings" => EquipmentLayerType::HumanoidLeggings,
                _ => EquipmentLayerType::Humanoid,
            };
            piece.resolved_materials(
                trim,
                |index, layer| {
                    let snapshot = EquipmentLayer::__new(
                        &layer.texture,
                        layer.dyeable,
                        layer.color_when_undyed,
                        layer.use_player_texture,
                    );
                    let color =
                        provider.armor_layer_tint_color(host, stack, &snapshot, index as u32, dye);
                    if color == 0 {
                        return None;
                    }
                    let default = texture(&layer.texture);
                    let selected = provider.armor_texture(
                        host,
                        stack,
                        layer_type,
                        &snapshot,
                        default.map_or(layer.texture.as_str(), |b| b.name),
                    );
                    let (binding, final_texture) = if selected == ArmorTextureRef::DEFAULT {
                        (default?, false)
                    } else {
                        let (generation, index) = selected.__parts();
                        if generation == state.generation
                            && let Some(tex) = state.textures.get(index as usize)
                        {
                            (
                                Binding {
                                    name: &tex.name,
                                    uv: tex.uv,
                                },
                                true,
                            )
                        } else {
                            report_stale("texture");
                            (default?, false)
                        }
                    };
                    let selected = provider.armor_layer_model(
                        host,
                        stack,
                        layer_type,
                        &snapshot,
                        index as u32,
                    );
                    let model = if selected == ArmorModelRef::VANILLA {
                        None
                    } else {
                        let (generation, index) = selected.__parts();
                        let model = (generation == state.generation)
                            .then(|| state.models.get(index as usize))
                            .flatten();
                        if let Some(model) = model {
                            Some(&model.geometry)
                        } else {
                            report_stale("model");
                            None
                        }
                    };
                    let color = (color & 0xFF00_FF00)
                        | ((color & 0x00FF_0000) >> 16)
                        | ((color & 0x0000_00FF) << 16);
                    Some(ResolvedEquipmentLayer {
                        binding,
                        color,
                        model,
                        final_texture,
                    })
                },
                emissive,
            )
        })
        .ok()
}

#[cfg(feature = "client")]
#[cfg(test)]
mod tests {
    use super::super::equipment_piece::{EquipmentAssetLayer, EquipmentPiece};
    use super::*;
    use mod_api::{
        ClientHost, EquipmentLayer, EquipmentLayerType, ModelPolygon, ModelVertex, PartPose,
    };
    use sunrise_client::mobs::{
        render::{ArmorSlot, WornModel},
        skin::Paint,
    };
    use sunrise_sim::mobs::anim::Poses;
    struct Empty;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Empty {}
    fn owner(id: &'static str) -> mod_api::content::crash::ModGuard {
        mod_api::content::crash::enter_native_mod(mod_api::content::crash::ModCall {
            id,
            hook: "register_client_item_extensions",
            part: None,
        })
    }
    fn tree(x: f32) -> ModelPart {
        let face = ModelPolygon::new(
            [
                ModelVertex::new([x, 0.0, 0.0], 0.0, 0.0),
                ModelVertex::new([x + 1.0, 0.0, 0.0], 1.0, 0.0),
                ModelVertex::new([x + 1.0, 1.0, 0.0], 1.0, 1.0),
                ModelVertex::new([x, 1.0, 0.0], 0.0, 1.0),
            ],
            [0.0, 0.0, 1.0],
        );
        ModelPart::new(
            Vec::new(),
            vec![(
                "body".into(),
                ModelPart::new(vec![face], Vec::new(), PartPose::ZERO),
            )],
            PartPose::ZERO,
        )
    }
    #[test]
    fn a_deep_armour_model_draws_and_drops_with_slot_visibility_intact() {
        let mut deep = tree(0.0);
        deep.polygons = deep.get_child("body").unwrap().polygons.clone();
        deep.skip_draw = true;
        const DEPTH: u16 = 20_000;
        for _ in 0..DEPTH {
            deep = ModelPart::new(
                Vec::new(),
                vec![("link".into(), deep)],
                PartPose::offset(1.0, 0.0, 0.0),
            );
        }
        let mut hidden = tree(100.0);
        hidden.visible = false;
        let root = ModelPart::new(
            Vec::new(),
            vec![
                ("deep".into(), deep),
                ("hidden".into(), hidden),
                ("last".into(), tree(4.0)),
            ],
            PartPose::ZERO,
        );
        let baked = EquipmentModel::bake(&root).unwrap();
        drop(root);

        let draw = |slot| {
            let mut vertices = Vec::new();
            WornModel::Override(&baked, slot).submit(
                &mut None,
                glam::Mat4::IDENTITY,
                &Poses::default(),
                Paint {
                    uv: [0.0, 0.0, 1.0, 1.0],
                    scroll: 0.0,
                    color: u32::MAX,
                    light: 0xF0F0,
                    overlay: 0,
                },
                &mut vertices,
            );
            vertices
        };
        let chest = draw(ArmorSlot::Chest);
        assert_eq!(
            chest.len(),
            8,
            "skipDraw kept its child, and visibility hid its subtree"
        );
        assert_eq!(chest[0].pos, [f32::from(DEPTH) / 16.0, 0.0, 0.0]);
        assert_eq!(
            chest[4].pos,
            [0.25, 0.0, 0.0],
            "sibling follows the deep subtree"
        );
        assert!(
            draw(ArmorSlot::Head).is_empty(),
            "slot visibility hides both body faces"
        );
        drop(baked);
    }

    #[test]
    fn a_panicking_equipment_registrar_leaves_the_next_batch_usable() {
        use mod_api::__private::game::RegistrationContextNew;
        use mod_api::{Hooks, ModEntry, RegisterClientItemExtensionsEvent, RegistrationContext};
        use std::sync::Mutex;

        struct Provider;
        impl ClientItemExtensions for Provider {}

        type Refs = (ArmorModelRef, ArmorTextureRef, ArmorTextureRef);
        struct Registrant {
            model: &'static str,
            refs: Arc<Mutex<Vec<Refs>>>,
            panics: bool,
        }
        #[mod_api::__subscriptions]
        impl Hooks for Registrant {
            fn register_client_item_extensions(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterClientItemExtensionsEvent<'_>,
            ) {
                let extensions = &mut *event.extensions;
                if self.panics {
                    extensions.register_texture("orphan").unwrap();
                }
                let shared = extensions.register_texture("shared").unwrap();
                assert_eq!(extensions.register_texture("shared").unwrap(), shared);
                let other = extensions
                    .register_texture(if self.panics { "failed" } else { "clear" })
                    .unwrap();
                let model = extensions.register_model(self.model, &tree(4.0)).unwrap();
                let item = mod_api::__private::item_key("test:gear").unwrap();
                extensions
                    .client_item_extensions(item, Arc::new(Provider))
                    .unwrap();
                if self.panics {
                    assert_eq!(
                        extensions.register_texture("missing"),
                        Err("unknown armour texture missing".into())
                    );
                }
                self.refs.lock().unwrap().push((model, shared, other));
                if self.panics {
                    panic!("after staging an equipment model, textures and provider");
                }
            }
        }

        let failed_refs = Arc::new(Mutex::new(Vec::new()));
        let healthy_refs = Arc::new(Mutex::new(Vec::new()));
        mod_platform::testing::with_mods(
            vec![
                ModEntry::new(
                    "failed",
                    Box::new(Registrant {
                        model: "failed:shape",
                        refs: failed_refs.clone(),
                        panics: true,
                    }),
                ),
                ModEntry::new(
                    "healthy",
                    Box::new(Registrant {
                        model: "healthy:shape",
                        refs: healthy_refs.clone(),
                        panics: false,
                    }),
                ),
            ]
            .into(),
            || {
                let mut host = mod_api::testing::FakeHost::new();
                host.registries
                    .resolve_items([mod_api::Identifier::parse("test:gear").unwrap()])
                    .unwrap();
                let mut owners = ItemExtensionOwners::default();
                let sprites = |name: &str| (name != "missing").then_some([0.0, 0.0, 1.0, 1.0]);
                let mut registration = Registration::new(&sprites, &mut owners);
                let generation = registration.state.generation;
                assert_eq!(
                    registration.register_texture("original").unwrap().__parts(),
                    (generation, 0)
                );
                {
                    let _owner = owner("prefix");
                    assert_eq!(
                        registration
                            .register_model("prefix:shape", &tree(0.0))
                            .unwrap()
                            .__parts(),
                        (generation, 0)
                    );
                }
                crate::game::client_side::native_hooks_client::register_client_item_extensions::native_register_client_item_extensions(
                    &RegistrationContext::new(),
                    &mut registration,
                );
                assert!(
                    failed_refs.lock().unwrap().len() == 1,
                    "the failed listener completed all staged edits before panicking"
                );
                let [(model, shared, clear)] = healthy_refs.lock().unwrap()[..] else {
                    panic!("the healthy listener registered its bindings exactly once");
                };
                assert_eq!(model.__parts(), (generation, 1));
                assert_eq!(shared.__parts(), (generation, 1));
                assert_eq!(clear.__parts(), (generation, 2));
                let Registration {
                    state,
                    texture_keys,
                    ..
                } = registration;
                assert_eq!(
                    state
                        .models
                        .iter()
                        .map(|model| model.name.as_str())
                        .collect::<Vec<_>>(),
                    ["prefix:shape", "healthy:shape"]
                );
                assert_eq!(
                    state
                        .textures
                        .iter()
                        .map(|tex| tex.name.as_str())
                        .collect::<Vec<_>>(),
                    ["original", "shared", "clear"]
                );
                assert_eq!(texture_keys.len(), 3);
                assert_eq!(texture_keys["shared"], 1);
                assert_eq!(texture_keys["clear"], 2);
                assert_eq!(state.items.len(), 1);
                let item = mod_api::__private::item_key("test:gear").unwrap();
                assert_eq!(state.items[&item].owner().id(), "healthy");
                assert_eq!(
                    mod_api::__private::take_hook_panics(),
                    [mod_api::__private::HookPanicReport {
                        mod_id: "failed",
                        hook: "register_client_item_extensions",
                    }]
                );
                let mut next = Registration::new(&sprites, &mut owners);
                crate::game::client_side::native_hooks_client::register_client_item_extensions::native_register_client_item_extensions(
                    &RegistrationContext::new(),
                    &mut next,
                );
                assert_eq!(failed_refs.lock().unwrap().len(), 1);
                assert_eq!(healthy_refs.lock().unwrap().len(), 2);
                assert_eq!(next.state.items[&item].owner().id(), "healthy");
                assert!(mod_api::__private::take_hook_panics().is_empty());
            },
        );
    }

    struct TwoShapes {
        models: [ArmorModelRef; 2],
        textures: [ArmorTextureRef; 2],
        calls: Arc<std::sync::Mutex<Vec<String>>>,
    }
    impl ClientItemExtensions for TwoShapes {
        fn default_dye_color(&self, _: &dyn ClientHost, _: &mod_api::ItemStackView<'_>) -> u32 {
            self.calls.lock().unwrap().push("dye".into());
            0
        }
        fn armor_layer_tint_color(
            &self,
            _: &dyn ClientHost,
            _: &mod_api::ItemStackView<'_>,
            _: &EquipmentLayer<'_>,
            index: u32,
            _: u32,
        ) -> u32 {
            self.calls.lock().unwrap().push(format!("tint{index}"));
            match index {
                0 => 0xFF12_3456,
                1 => 0xFFFF_FFFF,
                _ => 0,
            }
        }
        fn armor_texture(
            &self,
            _: &dyn ClientHost,
            _: &mod_api::ItemStackView<'_>,
            _: EquipmentLayerType,
            layer: &EquipmentLayer<'_>,
            default_texture: &str,
        ) -> ArmorTextureRef {
            let index = match (layer.texture, default_texture) {
                ("inner", "cit_inner") => 0,
                ("outer", "outer") => 1,
                pair => panic!("unexpected asset and default textures: {pair:?}"),
            };
            self.calls.lock().unwrap().push(format!("texture{index}"));
            self.textures[index]
        }
        fn armor_layer_model(
            &self,
            _: &dyn ClientHost,
            _: &mod_api::ItemStackView<'_>,
            _: EquipmentLayerType,
            _: &EquipmentLayer<'_>,
            index: u32,
        ) -> ArmorModelRef {
            self.calls.lock().unwrap().push(format!("model{index}"));
            self.models[index as usize]
        }
    }
    #[test]
    fn two_shapes_use_only_requested_reload_textures_and_one_trim() {
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new("armor_test", Box::new(Empty))].into(),
            || {
                use mod_api::prelude::ItemsHost;
                let mut host = mod_api::testing::FakeHost::new();
                let id = mod_api::Identifier::parse("armor_test:chest").unwrap();
                host.registries.resolve_items([id.clone()]).unwrap();
                let item = host.item_key(&id).unwrap();
                let stack = mod_api::ItemStack::of(item);
                let mut owners = ItemExtensionOwners::default();
                let reload_lookups = std::cell::Cell::new(0);
                let mut registry = Registration {
                    state: Default::default(),
                    texture_keys: FxHashMap::default(),
                    published: None,
                    sprite_uv: &|name| {
                        reload_lookups.set(reload_lookups.get() + 1);
                        match name {
                            "custom_inner" => Some([0.0, 0.0, 0.25, 0.25]),
                            "custom_outer" => Some([0.25, 0.25, 0.5, 0.5]),
                            "inner" | "unrelated" => Some([0.75, 0.75, 1.0, 1.0]),
                            _ => None,
                        }
                    },
                    owners: &mut owners,
                };
                let _owner = owner("armor_test");
                let models = [
                    registry
                        .register_model("armor_test:inner", &tree(0.0))
                        .unwrap(),
                    registry
                        .register_model("armor_test:outer", &tree(16.0))
                        .unwrap(),
                ];
                let textures = [
                    registry.register_texture("custom_inner").unwrap(),
                    registry.register_texture("custom_outer").unwrap(),
                ];
                assert_eq!(
                    registry.register_texture("custom_inner").unwrap(),
                    textures[0]
                );
                let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
                registry
                    .client_item_extensions(
                        item,
                        Arc::new(TwoShapes {
                            models,
                            textures,
                            calls: calls.clone(),
                        }),
                    )
                    .unwrap();
                let layers = [
                    EquipmentAssetLayer {
                        texture: "inner".into(),
                        dyeable: false,
                        use_player_texture: false,
                        color_when_undyed: None,
                    },
                    EquipmentAssetLayer {
                        texture: "outer".into(),
                        dyeable: false,
                        use_player_texture: false,
                        color_when_undyed: None,
                    },
                    EquipmentAssetLayer {
                        texture: "skipped".into(),
                        dyeable: false,
                        use_player_texture: false,
                        color_when_undyed: None,
                    },
                ];
                let piece = EquipmentPiece {
                    asset: "armor_test:chest".into(),
                    slot: ArmorSlot::Chest,
                    kind: "humanoid",
                    layers: &layers,
                    overlay: 36,
                };
                let state = registry.state;
                assert_eq!(
                    state
                        .textures
                        .iter()
                        .map(|tex| tex.name.as_str())
                        .collect::<Vec<_>>(),
                    ["custom_inner", "custom_outer"],
                );
                let mut default = |name: &str| match name {
                    "inner" => Some(Binding {
                        name: "cit_inner",
                        uv: [0.75, 0.75, 1.0, 1.0],
                    }),
                    "outer" => None,
                    _ => panic!("unexpected per-frame atlas lookup: {name}"),
                };
                let mut glow = |_: &str| None;
                let materials = dispatch_equipment(
                    &host,
                    &state,
                    &state.items[&item],
                    &mod_api::ItemStackView::from_stack(&stack),
                    &piece,
                    EquipmentBindings {
                        trim: Some(Binding {
                            name: "trim",
                            uv: [0.5, 0.5, 1.0, 1.0],
                        }),
                        default: &mut default,
                        emissive: &mut glow,
                    },
                )
                .unwrap();
                assert_eq!(reload_lookups.get(), 2);
                assert_eq!(
                    *calls.lock().unwrap(),
                    [
                        "dye", "tint0", "texture0", "model0", "tint1", "texture1", "model1",
                        "tint2"
                    ]
                );
                assert_eq!(
                    materials
                        .iter()
                        .map(|m| (m.binding.name, m.overlay))
                        .collect::<Vec<_>>(),
                    [("custom_inner", 0), ("custom_outer", 0), ("trim", 36)]
                );
                assert_eq!(
                    materials[0].color, 0xFF56_3412,
                    "ARGB becomes renderer ABGR"
                );
                assert!(materials[0].final_texture && materials[1].final_texture);
                let selected = materials
                    .iter()
                    .map(|m| m.model.unwrap())
                    .collect::<Vec<_>>();
                assert!(Arc::ptr_eq(selected[0], &state.models[0].geometry));
                assert!(Arc::ptr_eq(selected[1], &state.models[1].geometry));
                assert!(
                    Arc::ptr_eq(selected[2], selected[1]),
                    "one trim on the last material's tree"
                );
                let mut vertices = Vec::new();
                for m in materials {
                    let model = m.model.unwrap();
                    WornModel::Override(model, piece.slot).submit(
                        &mut None,
                        glam::Mat4::IDENTITY,
                        &Poses::default(),
                        Paint {
                            uv: m.binding.uv,
                            scroll: 0.0,
                            color: m.color,
                            light: 0xF0F0,
                            overlay: m.overlay,
                        },
                        &mut vertices,
                    );
                }
                assert_eq!(
                    vertices.len(),
                    12,
                    "one face per material and one trim, no double painting"
                );
                assert_eq!(vertices[0].pos[0], 0.0);
                assert_eq!(vertices[4].pos[0], 1.0);
                assert_eq!(vertices[8].pos[0], 1.0);
                assert_eq!(vertices[0].uv, [0.0, 0.0]);
                assert_eq!(vertices[4].uv, [0.25, 0.25]);
                assert_eq!(vertices[8].uv, [0.5, 0.5]);
            },
        );
    }
    #[test]
    fn model_registration_accepts_other_namespaces() {
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new("armor_test", Box::new(Empty))].into(),
            || {
                let mut owners = ItemExtensionOwners::default();
                let mut registry = Registration {
                    state: Default::default(),
                    texture_keys: FxHashMap::default(),
                    published: None,
                    sprite_uv: &|_| None,
                    owners: &mut owners,
                };
                let _owner = owner("armor_test");
                for name in ["other:chest", "minecraft:chest"] {
                    registry.register_model(name, &tree(0.0)).unwrap();
                }
                assert_eq!(
                    registry
                        .state
                        .models
                        .iter()
                        .map(|model| model.name.as_str())
                        .collect::<Vec<_>>(),
                    ["other:chest", "minecraft:chest"]
                );
            },
        );
    }

    struct DefaultProvider;
    impl ClientItemExtensions for DefaultProvider {}

    #[test]
    fn unknown_bindings_and_duplicate_providers_are_refused_by_name() {
        mod_platform::testing::with_mods(
            vec![
                mod_api::ModEntry::new("armor_test", Box::new(Empty)),
                mod_api::ModEntry::new("other", Box::new(Empty)),
            ]
            .into(),
            || {
                use mod_api::prelude::ItemsHost;
                let mut host = mod_api::testing::FakeHost::new();
                let id = mod_api::Identifier::parse("armor_test:chest").unwrap();
                host.registries.resolve_items([id.clone()]).unwrap();
                let item = host.item_key(&id).unwrap();
                let mut owners = ItemExtensionOwners::default();
                let mut registry = Registration::new(&|_| None, &mut owners);
                let missing = registry.register_texture("armor_test:missing").unwrap_err();
                assert!(missing.contains("armor_test:missing"));
                {
                    let _owner = owner("armor_test");
                    let mut invalid = tree(0.0);
                    invalid.x = f32::NAN;
                    let invalid = registry
                        .register_model("armor_test:invalid", &invalid)
                        .unwrap_err();
                    assert!(invalid.contains("armor_test:invalid"));
                    registry
                        .client_item_extensions(item, Arc::new(DefaultProvider))
                        .unwrap();
                }
                let _owner = owner("other");
                let duplicate = registry
                    .client_item_extensions(item, Arc::new(DefaultProvider))
                    .unwrap_err();
                assert!(duplicate.contains("armor_test") && duplicate.contains("other"));
            },
        );
    }

    #[test]
    fn default_dye_color_reads_the_live_override_without_projecting() {
        use mod_api::prelude::ItemsHost;
        use std::{cell::Cell, num::NonZeroU32};

        let mut host = mod_api::testing::FakeHost::new();
        let id = mod_api::Identifier::parse("armor_test:chest").unwrap();
        host.registries.resolve_items([id.clone()]).unwrap();
        let item = host.item_key(&id).unwrap();
        let dye = Cell::new(Some(0x12_34_56));
        let project = |_| panic!("dye lookup projected the whole stack");
        let read_override = |id: &str| {
            assert_eq!(id, "minecraft:dyed_color");
            dye.get().map(mod_api::Tag::Int)
        };
        let view = mod_api::ItemStackView::resident_with_components(
            (item, NonZeroU32::MIN, None),
            &project,
            &read_override,
        );
        assert_eq!(DefaultProvider.default_dye_color(&host, &view), 0xFF12_3456);
        dye.set(None);
        assert_eq!(DefaultProvider.default_dye_color(&host, &view), 0);
    }

    #[test]
    #[ignore = "daily"]
    fn invalid_equipped_stack_warns_without_disabling_later_draws() {
        use crate::game::native_entity_refs::tests::{run_fresh_cases, with_client_game};
        use std::sync::atomic::AtomicUsize;
        use sunrise_items::items::ItemStack;

        if run_fresh_cases(
            "game::client_side::client_item_extensions::tests::invalid_equipped_stack_warns_without_disabling_later_draws",
            "SUNRISE_INVALID_EQUIPPED_STACK_CHILD",
            &["empty"],
        ) {
            return;
        }
        static WARNINGS: AtomicUsize = AtomicUsize::new(0);
        struct Capture;
        impl log::Log for Capture {
            fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
                metadata.level() == log::Level::Warn
            }
            fn log(&self, record: &log::Record<'_>) {
                if self.enabled(record.metadata())
                    && record.args().to_string()
                        == "Sunrise: client item extensions for armor_test: invalid equipped stack: Empty"
                {
                    WARNINGS.fetch_add(1, Ordering::Relaxed);
                }
            }
            fn flush(&self) {}
        }
        struct RegistersDefault;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for RegistersDefault {
            fn register_client_item_extensions(
                &self,
                _: &mod_api::RegistrationContext,
                event: &mut mod_api::RegisterClientItemExtensionsEvent<'_>,
            ) {
                let item = mod_api::__private::item_key("minecraft:iron_chestplate").unwrap();
                event
                    .extensions
                    .client_item_extensions(item, Arc::new(DefaultProvider))
                    .unwrap();
            }
        }
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new(
                "armor_test",
                Box::new(RegistersDefault),
            )]
            .into(),
            || {
                with_client_game(|game| {
                    game.resolve_client_item_extensions();
                    assert!(!game.client.gui.atlas.sprites.is_empty());
                    assert!(
                        game.client
                            .armor_stand_assets
                            .0
                            .extensions
                            .textures
                            .is_empty(),
                        "no sprite was requested"
                    );
                    let game = &*game;
                    let layers = [EquipmentAssetLayer {
                        texture: "asset".into(),
                        dyeable: true,
                        use_player_texture: false,
                        color_when_undyed: None,
                    }];
                    let piece = EquipmentPiece {
                        asset: "minecraft:iron".into(),
                        slot: ArmorSlot::Chest,
                        kind: "humanoid",
                        layers: &layers,
                        overlay: 0,
                    };
                    let binding = Binding {
                        name: "cit_texture",
                        uv: [0.0, 0.0, 1.0, 1.0],
                    };
                    let mut stack = ItemStack::new("minecraft:iron_chestplate", 1);
                    stack.data_mut().dyed_color = Some(0x12_34_56);
                    stack.count = 0;

                    log::set_logger(&Capture).unwrap();
                    log::set_max_level(log::LevelFilter::Warn);
                    let draw = |stack: &ItemStack| {
                        game.extended_equipment_materials(
                            &piece,
                            &EquipmentRead::resident(stack),
                            None,
                            |_| Some(binding),
                            |_| None,
                        )
                    };
                    assert!(
                        draw(&stack).is_none(),
                        "invalid stack selects the vanilla draw"
                    );
                    assert!(
                        draw(&stack).is_none(),
                        "a second invalid draw is also refused"
                    );
                    assert_eq!(
                        WARNINGS.load(Ordering::Relaxed),
                        2,
                        "every refusal reports the mod and invalid stack",
                    );
                    stack.count = 1;
                    let materials = draw(&stack).unwrap();
                    assert_eq!(materials.len(), 1);
                    assert_eq!(materials[0].binding.name, "cit_texture");
                    assert_eq!(materials[0].color, 0xFF56_3412);
                    assert!(!materials[0].final_texture);
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn mob_worn_callbacks_read_gear_and_submit_reload_bound_uvs() {
        use crate::game::native_entity_refs::tests::{run_fresh_cases, with_client_game};
        use mod_api::{Component, ItemStackView, Tag};
        use sunrise_client::{hud::Rect, mobs::render};
        use sunrise_core::{equipment_slot::EquipmentSlot, geometry::Vec3};
        use sunrise_items::items::ItemStack;
        use sunrise_sim::mobs::{Mob, finalize};

        if run_fresh_cases(
            "game::client_side::client_item_extensions::tests::mob_worn_callbacks_read_gear_and_submit_reload_bound_uvs",
            "SUNRISE_WORN_MOB_BINDINGS_CHILD",
            &["mob"],
        ) {
            return;
        }

        const SPRITE: &str = "armor_test:entity/equipment/selected";
        struct Protection;
        impl Component for Protection {
            type Value = i32;
            fn id(&self) -> &str {
                "minecraft:enchantments"
            }
            fn decode(&self, tag: &Tag) -> Option<i32> {
                match tag.get("minecraft:protection")? {
                    Tag::Int(level) => Some(*level),
                    _ => None,
                }
            }
            fn encode(&self, level: i32) -> Tag {
                Tag::compound([("minecraft:protection", Tag::Int(level))])
            }
        }

        struct Provider(ArmorTextureRef, ArmorModelRef);
        impl ClientItemExtensions for Provider {
            fn default_dye_color(&self, _: &dyn ClientHost, stack: &ItemStackView<'_>) -> u32 {
                assert_eq!(stack.get(&Protection), Some(3));
                0
            }

            fn armor_texture(
                &self,
                _: &dyn ClientHost,
                _: &ItemStackView<'_>,
                _: EquipmentLayerType,
                _: &EquipmentLayer<'_>,
                _: &str,
            ) -> ArmorTextureRef {
                self.0
            }

            fn armor_layer_model(
                &self,
                _: &dyn ClientHost,
                _: &ItemStackView<'_>,
                _: EquipmentLayerType,
                _: &EquipmentLayer<'_>,
                _: u32,
            ) -> ArmorModelRef {
                self.1
            }
        }

        struct Register;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Register {
            fn register_client_item_extensions(
                &self,
                _: &mod_api::RegistrationContext,
                event: &mut mod_api::RegisterClientItemExtensionsEvent<'_>,
            ) {
                let texture = event.extensions.register_texture(SPRITE).unwrap();
                let model = event
                    .extensions
                    .register_model("armor_test:chest", &tree(0.0))
                    .unwrap();
                let item = mod_api::__private::item_key("minecraft:iron_chestplate").unwrap();
                event
                    .extensions
                    .client_item_extensions(item, Arc::new(Provider(texture, model)))
                    .unwrap();
            }
        }

        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new("armor_test", Box::new(Register))].into(),
            || {
                with_client_game(|game| {
                    game.client.gui.atlas.sprites.insert(
                        SPRITE.into(),
                        Rect {
                            u0: 0.25,
                            v0: 0.375,
                            u1: 0.5,
                            v1: 0.75,
                            w: 16,
                            h: 16,
                        },
                    );
                    game.resolve_client_item_extensions();
                    let kind = game
                        .res
                        .mob_types()
                        .get("minecraft:zombie")
                        .unwrap()
                        .clone();
                    let mut mob = Mob::new(1, kind, Vec3::ZERO, 0.0);
                    let slot = EquipmentSlot::Chest;
                    mob.extra.set_slot_stack(
                        slot as usize,
                        Some(ItemStack::new("minecraft:iron_chestplate", 1)),
                    );
                    finalize::set_slot_enchantments(
                        &mut mob,
                        slot as usize,
                        vec![("minecraft:protection".into(), 3)],
                    );
                    let read = finalize::equipment_slot(&mob, slot).unwrap();
                    assert!(read.base().enchantments().is_empty());
                    let metadata =
                        sunrise_gameplay_common::game::api_values::stack_read_metadata(read.base())
                            .unwrap();
                    sunrise_gameplay_common::game::native_equipment_read::with_equipment_view(
                        &read,
                        metadata,
                        |view, _| {
                            assert_eq!(view.project().unwrap().get(&Protection), Some(3));
                        },
                    );

                    let mut stack = read.base().clone();
                    // An unrelated malformed component defeats a full projection, but not the
                    // provider's single-component read of the live enchantment overlay.
                    stack
                        .data_mut()
                        .set_other("!minecraft:damage", Some(Tag::Int(7)));
                    mob.extra.set_slot_stack(slot as usize, Some(stack));
                    let read = finalize::equipment_slot(&mob, slot).unwrap();
                    let metadata =
                        sunrise_gameplay_common::game::api_values::stack_read_metadata(read.base())
                            .unwrap();
                    sunrise_gameplay_common::game::native_equipment_read::with_equipment_view(
                        &read,
                        metadata,
                        |view, _| {
                            assert!(view.project().is_none());
                        },
                    );
                    let atlas = |name: &str| {
                        assert_ne!(name, SPRITE, "the mob submission already has the bound UVs");
                        Some([0.0, 0.0, 1.0, 1.0])
                    };
                    let mut vertices = Vec::new();
                    render::render(
                        &mob,
                        0.0,
                        glam::DVec3::ZERO,
                        &atlas,
                        0xF0F0,
                        game,
                        &mut vertices,
                    );
                    let worn = vertices
                        .last_chunk::<4>()
                        .expect("the worn model submitted a quad");
                    assert!(worn.iter().all(|v| v.layer == render::ARMOR_GLINT));
                    let uvs = worn.each_ref().map(|v| v.uv);
                    assert_eq!(
                        uvs,
                        [[0.25, 0.375], [0.5, 0.375], [0.5, 0.75], [0.25, 0.75]]
                    );
                })
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn invalid_registrations_do_not_discard_valid_reload_entries() {
        if crate::game::native_entity_refs::tests::run_fresh_cases(
            "game::client_side::client_item_extensions::tests::invalid_registrations_do_not_discard_valid_reload_entries",
            "SUNRISE_INVALID_REGISTRATIONS_CHILD",
            &["case"],
        ) {
            return;
        }
        use mod_api::{Hooks, RegisterClientItemExtensionsEvent, RegistrationContext};
        use std::sync::Mutex;

        #[derive(Default)]
        struct Results {
            accepted_model: Option<ArmorModelRef>,
            errors: Vec<String>,
        }
        struct Registers(Arc<Mutex<Results>>);
        #[mod_api::__subscriptions]
        impl Hooks for Registers {
            fn register_client_item_extensions(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterClientItemExtensionsEvent<'_>,
            ) {
                let registrar = &mut event.extensions;
                let model = registrar
                    .register_model("armor_test:valid", &tree(0.0))
                    .unwrap();
                let mut invalid = tree(0.0);
                invalid.x = f32::NAN;
                let errors = vec![
                    registrar
                        .register_model("armor_test:invalid", &invalid)
                        .unwrap_err(),
                    registrar
                        .register_model("armor_test:valid", &tree(0.0))
                        .unwrap_err(),
                    registrar
                        .register_texture("armor_test:missing")
                        .unwrap_err(),
                ];
                let item = mod_api::__private::item_key("minecraft:iron_chestplate").unwrap();
                registrar
                    .client_item_extensions(item, Arc::new(DefaultProvider))
                    .unwrap();
                *self.0.lock().unwrap() = Results {
                    accepted_model: Some(model),
                    errors,
                };
            }
        }
        struct Duplicate(Arc<Mutex<Results>>);
        #[mod_api::__subscriptions]
        impl Hooks for Duplicate {
            fn register_client_item_extensions(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterClientItemExtensionsEvent<'_>,
            ) {
                let item = mod_api::__private::item_key("minecraft:iron_chestplate").unwrap();
                let error = event
                    .extensions
                    .client_item_extensions(item, Arc::new(DefaultProvider))
                    .unwrap_err();
                self.0.lock().unwrap().errors.push(error);
            }
        }

        let results = Arc::new(Mutex::new(Results::default()));
        mod_platform::testing::with_mods(
            vec![
                mod_api::ModEntry::new("armor_test", Box::new(Registers(results.clone()))),
                mod_api::ModEntry::new("other", Box::new(Duplicate(results.clone()))),
            ]
            .into(),
            || {
                crate::game::native_entity_refs::tests::with_client_game(|game| {
                    game.resolve_client_item_extensions();

                    let results = results.lock().unwrap();
                    assert_eq!(
                        results.errors,
                        [
                            "armour model armor_test:invalid: non-finite model pose",
                            "duplicate armour model armor_test:valid",
                            "unknown armour texture armor_test:missing",
                            "duplicate client item extensions for minecraft:iron_chestplate: armor_test and other",
                        ]
                    );
                    let extensions = &game.client.armor_stand_assets.0.extensions;
                    let (generation, index) = results.accepted_model.unwrap().__parts();
                    assert_eq!(extensions.generation, generation);
                    assert_eq!(extensions.models[index as usize].name, "armor_test:valid");
                    let item = mod_api::__private::item_key("minecraft:iron_chestplate").unwrap();
                    assert_eq!(extensions.items[&item].owner().id(), "armor_test");
                });
            },
        );
    }

    #[test]
    fn stale_refs_fall_back_to_vanilla_bindings_without_disabling_the_provider() {
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new("armor_test", Box::new(Empty))].into(),
            || {
                use mod_api::prelude::ItemsHost;
                struct Stale(u64);
                impl ClientItemExtensions for Stale {
                    fn armor_texture(
                        &self,
                        _: &dyn ClientHost,
                        _: &mod_api::ItemStackView<'_>,
                        _: EquipmentLayerType,
                        _: &EquipmentLayer<'_>,
                        _: &str,
                    ) -> ArmorTextureRef {
                        ArmorTextureRef::__new(self.0, 0)
                    }
                    fn armor_layer_model(
                        &self,
                        _: &dyn ClientHost,
                        _: &mod_api::ItemStackView<'_>,
                        _: EquipmentLayerType,
                        _: &EquipmentLayer<'_>,
                        _: u32,
                    ) -> ArmorModelRef {
                        ArmorModelRef::__new(self.0, 0)
                    }
                }
                let mut host = mod_api::testing::FakeHost::new();
                let id = mod_api::Identifier::parse("armor_test:chest").unwrap();
                host.registries.resolve_items([id.clone()]).unwrap();
                let item = host.item_key(&id).unwrap();
                let old = ArmorExtensions::default();
                let mut owners = ItemExtensionOwners::default();
                let mut registry = Registration {
                    state: Default::default(),
                    texture_keys: FxHashMap::default(),
                    published: None,
                    sprite_uv: &|_| Some([0.5, 0.5, 1.0, 1.0]),
                    owners: &mut owners,
                };
                let _owner = owner("armor_test");
                registry.register_texture("other").unwrap();
                registry
                    .register_model("armor_test:other", &tree(0.0))
                    .unwrap();
                registry
                    .client_item_extensions(item, Arc::new(Stale(old.generation)))
                    .unwrap();
                let layers = [EquipmentAssetLayer {
                    texture: "original".into(),
                    dyeable: false,
                    use_player_texture: false,
                    color_when_undyed: None,
                }];
                let piece = EquipmentPiece {
                    asset: "armor_test:chest".into(),
                    slot: ArmorSlot::Chest,
                    kind: "humanoid",
                    layers: &layers,
                    overlay: 0,
                };
                let binding = Binding {
                    name: "original",
                    uv: [0.0, 0.0, 1.0, 1.0],
                };
                let part = &registry.state.items[&item];
                for _ in 0..2 {
                    let mut default = |_: &str| Some(binding);
                    let mut glow = |_: &str| None;
                    let out = dispatch_equipment(
                        &host,
                        &registry.state,
                        part,
                        &mod_api::ItemStackView::from_stack(&mod_api::ItemStack::of(item)),
                        &piece,
                        EquipmentBindings {
                            trim: None,
                            default: &mut default,
                            emissive: &mut glow,
                        },
                    )
                    .unwrap();
                    assert_eq!(out.len(), 1);
                    assert_eq!(out[0].binding.name, "original");
                    assert!(out[0].model.is_none());
                    assert!(!out[0].final_texture);
                }
                assert!(part.owner().is_enabled());
            },
        );
    }
    #[test]
    fn callback_panic_discards_the_piece_and_disables_only_its_mod_and_item_across_reloads() {
        mod_platform::testing::with_mods(
            vec![
                mod_api::ModEntry::new("armor_test", Box::new(Empty)),
                mod_api::ModEntry::new("other", Box::new(Empty)),
            ]
            .into(),
            || {
                use mod_api::prelude::ItemsHost;
                struct FailsOnSecond(Arc<std::sync::atomic::AtomicUsize>);
                impl ClientItemExtensions for FailsOnSecond {
                    fn armor_layer_tint_color(
                        &self,
                        _: &dyn ClientHost,
                        _: &mod_api::ItemStackView<'_>,
                        _: &EquipmentLayer<'_>,
                        index: u32,
                        _: u32,
                    ) -> u32 {
                        self.0.fetch_add(1, Ordering::Relaxed);
                        assert_ne!(index, 1, "provider panic");
                        0xFFFFFFFF
                    }
                }
                let mut host = mod_api::testing::FakeHost::new();
                let id = mod_api::Identifier::parse("armor_test:chest").unwrap();
                let other_id = mod_api::Identifier::parse("armor_test:helmet").unwrap();
                host.registries
                    .resolve_items([id.clone(), other_id.clone()])
                    .unwrap();
                let item = host.item_key(&id).unwrap();
                let other_item = host.item_key(&other_id).unwrap();
                let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
                let layers = std::array::from_fn::<_, 2, _>(|_| EquipmentAssetLayer {
                    texture: "original".into(),
                    dyeable: false,
                    use_player_texture: false,
                    color_when_undyed: None,
                });
                let piece = EquipmentPiece {
                    asset: "armor_test:chest".into(),
                    slot: ArmorSlot::Chest,
                    kind: "humanoid",
                    layers: &layers,
                    overlay: 0,
                };
                let binding = Binding {
                    name: "original",
                    uv: [0.0, 0.0, 1.0, 1.0],
                };
                let mut owners = ItemExtensionOwners::default();
                for mod_id in ["armor_test", "armor_test", "other"] {
                    let mut registry = Registration {
                        state: Default::default(),
                        texture_keys: FxHashMap::default(),
                        published: None,
                        sprite_uv: &|_| None,
                        owners: &mut owners,
                    };
                    {
                        let _owner = owner("armor_test");
                        registry
                            .client_item_extensions(other_item, Arc::new(DefaultProvider))
                            .unwrap();
                    }
                    {
                        let _owner = owner(mod_id);
                        let provider: Arc<dyn ClientItemExtensions> = if mod_id == "armor_test" {
                            Arc::new(FailsOnSecond(calls.clone()))
                        } else {
                            Arc::new(DefaultProvider)
                        };
                        registry.client_item_extensions(item, provider).unwrap();
                    }
                    let draw = |item| {
                        dispatch_equipment(
                            &host,
                            &registry.state,
                            &registry.state.items[&item],
                            &mod_api::ItemStackView::from_stack(&mod_api::ItemStack::of(item)),
                            &piece,
                            EquipmentBindings {
                                trim: None,
                                default: &mut |_| Some(binding),
                                emissive: &mut |_| None,
                            },
                        )
                    };
                    if mod_id == "armor_test" {
                        assert!(draw(item).is_none(), "a panic discards even earlier layers");
                        assert!(draw(item).is_none(), "the callback stays disabled");
                    } else {
                        assert_eq!(draw(item).unwrap().len(), 2, "another mod is unaffected");
                    }
                    assert_eq!(
                        draw(other_item).unwrap().len(),
                        2,
                        "another item is unaffected"
                    );
                    assert_eq!(
                        calls.load(Ordering::Relaxed),
                        2,
                        "only the first draw entered the panicking provider"
                    );
                }
            },
        );
    }

    /// Asks the Host for a level in its tint callback.
    struct ReadsLevel(Arc<std::sync::atomic::AtomicUsize>);
    impl ClientItemExtensions for ReadsLevel {
        fn armor_layer_tint_color(
            &self,
            host: &dyn ClientHost,
            _: &mod_api::ItemStackView<'_>,
            _: &EquipmentLayer<'_>,
            _: u32,
            _: u32,
        ) -> u32 {
            assert!(host.level("minecraft:overworld").is_none());
            self.0.fetch_add(1, Ordering::Relaxed);
            0xFFFF_FFFF
        }
    }

    /// A world read during Setup or Reloading refuses the current piece without poisoning the
    /// provider. The same registered callback can answer a subsequent draw in World.
    #[test]
    #[ignore = "daily"]
    fn client_item_extensions_can_draw_without_a_level() {
        if crate::game::native_entity_refs::tests::run_fresh_cases(
            "game::client_side::client_item_extensions::tests::client_item_extensions_can_draw_without_a_level",
            "SUNRISE_DRAW_WITHOUT_LEVEL_CHILD",
            &["case"],
        ) {
            return;
        }
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new("reads_level", Box::new(Empty))].into(),
            || {
                crate::game::native_entity_refs::tests::with_client_game(|game| {
                    let item = mod_api::__private::item_key("minecraft:iron_chestplate").unwrap();
                    let layers = [EquipmentAssetLayer {
                        texture: "original".into(),
                        dyeable: false,
                        use_player_texture: false,
                        color_when_undyed: None,
                    }];
                    let piece = EquipmentPiece {
                        asset: "minecraft:iron".into(),
                        slot: ArmorSlot::Chest,
                        kind: "humanoid",
                        layers: &layers,
                        overlay: 0,
                    };
                    let binding = Binding {
                        name: "original",
                        uv: [0.0, 0.0, 1.0, 1.0],
                    };
                    let answered = Arc::new(std::sync::atomic::AtomicUsize::new(0));
                    let mut owners = ItemExtensionOwners::default();
                    let mut registry = Registration {
                        state: Default::default(),
                        texture_keys: FxHashMap::default(),
                        published: None,
                        sprite_uv: &|_| None,
                        owners: &mut owners,
                    };
                    {
                        let _owner = owner("reads_level");
                        registry
                            .client_item_extensions(item, Arc::new(ReadsLevel(answered.clone())))
                            .unwrap();
                    }
                    let part = &registry.state.items[&item];
                    let draw = || {
                        dispatch_equipment(
                            &*game,
                            &registry.state,
                            part,
                            &mod_api::ItemStackView::from_stack(&mod_api::ItemStack::of(item)),
                            &piece,
                            EquipmentBindings {
                                trim: None,
                                default: &mut |_| Some(binding),
                                emissive: &mut |_| None,
                            },
                        )
                    };
                    assert_eq!(draw().unwrap().len(), 1);
                    assert_eq!(answered.load(Ordering::Relaxed), 1);
                });
            },
        );
    }

    #[test]
    fn dropped_stack_motion_takes_the_item_answers_and_vanilla_after_a_panic() {
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new("flat_test", Box::new(Empty))].into(),
            || {
                use mod_api::prelude::ItemsHost;
                struct Flat;
                impl ClientItemExtensions for Flat {
                    fn should_bob_as_entity(
                        &self,
                        _: &dyn ClientHost,
                        _: &mod_api::ItemStack,
                    ) -> bool {
                        false
                    }
                    fn should_spread_as_entity(
                        &self,
                        _: &dyn ClientHost,
                        _: &mod_api::ItemStack,
                    ) -> bool {
                        false
                    }
                }
                struct Spreads;
                impl ClientItemExtensions for Spreads {
                    fn should_bob_as_entity(
                        &self,
                        _: &dyn ClientHost,
                        _: &mod_api::ItemStack,
                    ) -> bool {
                        false
                    }
                }
                struct Panics;
                impl ClientItemExtensions for Panics {
                    fn should_bob_as_entity(
                        &self,
                        _: &dyn ClientHost,
                        _: &mod_api::ItemStack,
                    ) -> bool {
                        panic!("bob refused")
                    }
                    fn should_spread_as_entity(
                        &self,
                        _: &dyn ClientHost,
                        _: &mod_api::ItemStack,
                    ) -> bool {
                        panic!("spread refused")
                    }
                }
                let mut host = mod_api::testing::FakeHost::new();
                let ids = ["flat_test:picture", "flat_test:spread", "flat_test:panics"]
                    .map(|id| mod_api::Identifier::parse(id).unwrap());
                host.registries.resolve_items(ids.clone()).unwrap();
                let [flat, spread, panics] = ids.map(|id| host.item_key(&id).unwrap());
                let mut owners = ItemExtensionOwners::default();
                let mut registry = Registration::new(&|_| Some([0.0, 0.0, 1.0, 1.0]), &mut owners);
                let _owner = owner("flat_test");
                registry
                    .client_item_extensions(flat, Arc::new(Flat))
                    .unwrap();
                registry
                    .client_item_extensions(spread, Arc::new(Spreads))
                    .unwrap();
                registry
                    .client_item_extensions(panics, Arc::new(Panics))
                    .unwrap();
                let motion = |item| {
                    let stack = mod_api::ItemStack::of(item);
                    dispatch_dropped_stack_motion(&host, &registry.state.items[&item], &stack)
                };
                assert_eq!(
                    motion(flat),
                    DroppedStackMotion {
                        bob: false,
                        spread: false
                    }
                );
                assert_eq!(
                    motion(spread),
                    DroppedStackMotion {
                        bob: false,
                        spread: true
                    }
                );
                assert_eq!(motion(panics), DroppedStackMotion::VANILLA);
                let _ = mod_api::__private::take_hook_panics();
            },
        );
    }
}
