//! Bound item tint sources, item model node types, and range/select item model property
//! providers registered at client startup (`register_item_tint_sources`, `register_item_models`,
//! `register_range_select_item_model_property`, `register_select_item_model_property`).
//!
//! Each table follows NeoForge's `ExtraCodecs.LateBoundIdMapper`: any valid `<namespace>:<path>`
//! identifier (including `minecraft:`) is accepted, a later registration for the same id replaces
//! the earlier one (last-writer-wins) with a warning naming both mods, and registered decoders run
//! ahead of built-in decoders so vanilla overrides take effect.
use super::SpecialModelFactories;
use mod_api::{
    __private::game::{
        CallbackOwner, CallbackValue, ItemModelRegistration, ItemTintSourceRegistration,
        RangeSelectPropertyRegistration, SelectPropertyRegistration, new_item_model_config,
    },
    Answered, Identifier, ItemModelChildren, ItemModelDecoder, ItemModelFn, ItemTintSourceDecoder,
    ItemTintSourceFn, RangeSelectPropertyDecoder, RangeSelectPropertyFn, SelectCaseKey,
    SelectPropertyDecoder, SelectPropertyFn,
};
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, OnceLock},
};
use sunrise_assets::{
    DecodeError,
    item_model::{
        DecodedItemModel, ItemModelNode, ModelLeaf, RegisteredItemModel, RegisteredRangeProperty,
        RegisteredSelectKey, RegisteredSelectProperty, RegisteredTintSource, SelectCaseDecodeError,
        optional_transformation,
    },
};
use sunrise_core::nbt::Tag;

/// One installed decode-time factory paired with the mod owner that registered it.
pub struct ItemProviderEntry<D: ?Sized> {
    decode: CallbackValue<Arc<D>>,
}

pub type ItemTintFactories = BTreeMap<String, ItemProviderEntry<ItemTintSourceDecoder>>;
pub type ItemModelFactories = BTreeMap<String, ItemProviderEntry<ItemModelDecoder>>;
pub type RangePropertyFactories = BTreeMap<String, ItemProviderEntry<RangeSelectPropertyDecoder>>;
pub type SelectPropertyFactories = BTreeMap<String, ItemProviderEntry<SelectPropertyDecoder>>;

/// Bound item tint source stored inside [`RegisteredTintSource`].
pub struct BoundTintSource {
    pub kind: Box<str>,
    pub eval: CallbackValue<Arc<ItemTintSourceFn>>,
}

/// Bound item model node stored inside [`RegisteredItemModel`].
pub struct BoundItemModel {
    pub kind: Box<str>,
    pub eval: CallbackValue<Arc<ItemModelFn>>,
}

/// Bound numeric `range_dispatch` property stored inside [`RegisteredRangeProperty`].
pub struct BoundRangeProperty {
    pub kind: Box<str>,
    pub eval: CallbackValue<Arc<RangeSelectPropertyFn>>,
}

/// Bound categorical `select` property stored inside [`RegisteredSelectProperty`].
pub struct BoundSelectProperty {
    pub kind: Box<str>,
    pub eval: CallbackValue<Arc<SelectPropertyFn>>,
}

impl BoundSelectProperty {
    pub fn select(
        &self,
        host: &dyn mod_api::ClientHost,
        input: &mod_api::ItemModelPropertyInput<'_>,
        keys: &mut dyn Iterator<Item = (usize, &RegisteredSelectKey)>,
    ) -> Option<usize> {
        self.eval
            .owner()
            .call(
                "item model select property",
                "item_model_select_property",
                || {
                    let mut keys = Iterator::filter_map(keys, |(index, key)| {
                        key.downcast_ref::<CallbackValue<SelectCaseKey>>()
                            .map(|key| (index, key.value()))
                    });
                    (self.eval.value())(host, input, &mut keys)
                },
            )
            .ok()
            .flatten()
    }
}

/// Decode-time provider tables passed into [`super::ItemRenderer::decode_definition`].
#[derive(Clone, Copy, Default)]
pub struct ItemProviderTables<'a> {
    pub special: Option<&'a SpecialModelFactories>,
    pub tint: Option<&'a ItemTintFactories>,
    pub item_model: Option<&'a ItemModelFactories>,
    pub range_property: Option<&'a RangePropertyFactories>,
    pub select_property: Option<&'a SelectPropertyFactories>,
}

static TINT_FACTORIES: OnceLock<Arc<ItemTintFactories>> = OnceLock::new();
static ITEM_MODEL_FACTORIES: OnceLock<Arc<ItemModelFactories>> = OnceLock::new();
static RANGE_PROPERTY_FACTORIES: OnceLock<Arc<RangePropertyFactories>> = OnceLock::new();
static SELECT_PROPERTY_FACTORIES: OnceLock<Arc<SelectPropertyFactories>> = OnceLock::new();

impl<'a> ItemProviderTables<'a> {
    pub(super) fn installed(special: Option<&'a SpecialModelFactories>) -> Self {
        Self {
            special,
            tint: TINT_FACTORIES.get().map(Arc::as_ref),
            item_model: ITEM_MODEL_FACTORIES.get().map(Arc::as_ref),
            range_property: RANGE_PROPERTY_FACTORIES.get().map(Arc::as_ref),
            select_property: SELECT_PROPERTY_FACTORIES.get().map(Arc::as_ref),
        }
    }
    #[cfg(any(test, feature = "test-resource-cache"))]
    pub(super) fn has_registered() -> bool {
        TINT_FACTORIES.get().is_some()
            || ITEM_MODEL_FACTORIES.get().is_some()
            || RANGE_PROPERTY_FACTORIES.get().is_some()
            || SELECT_PROPERTY_FACTORIES.get().is_some()
    }
}

fn is_builtin_tint_source(kind: &str) -> bool {
    matches!(
        kind,
        "minecraft:constant"
            | "minecraft:custom_model_data"
            | "minecraft:dye"
            | "minecraft:grass"
            | "minecraft:firework"
            | "minecraft:potion"
            | "minecraft:team"
    )
}

fn is_builtin_item_model(kind: &str) -> bool {
    matches!(
        kind,
        "minecraft:empty"
            | "minecraft:model"
            | "minecraft:composite"
            | "minecraft:condition"
            | "minecraft:select"
            | "minecraft:range_dispatch"
            | "minecraft:special"
            | "minecraft:bundle/selected_item"
            | "neoforge:fluid_container"
    )
}

fn is_builtin_range_property(kind: &str) -> bool {
    matches!(
        kind,
        "minecraft:custom_model_data"
            | "minecraft:bundle/fullness"
            | "minecraft:damage"
            | "minecraft:cooldown"
            | "minecraft:time"
            | "minecraft:compass"
            | "minecraft:crossbow/pull"
            | "minecraft:use_cycle"
            | "minecraft:use_duration"
            | "minecraft:count"
    )
}

fn is_builtin_select_property(kind: &str) -> bool {
    matches!(
        kind,
        "minecraft:custom_model_data"
            | "minecraft:main_hand"
            | "minecraft:charge_type"
            | "minecraft:trim_material"
            | "minecraft:block_state"
            | "minecraft:display_context"
            | "minecraft:local_time"
            | "minecraft:context_entity_type"
            | "minecraft:context_dimension"
            | "minecraft:component"
    )
}

fn bind_factories<D: ?Sized>(
    label: &'static str,
    is_builtin: fn(&str) -> bool,
    entries: impl IntoIterator<Item = (&'static str, Identifier, Arc<D>)>,
) -> (BTreeMap<String, ItemProviderEntry<D>>, Vec<String>) {
    let mut bound: BTreeMap<String, ItemProviderEntry<D>> = BTreeMap::new();
    let mut kept = Vec::new();
    let mut seen = BTreeSet::new();
    for (mod_id, id, decode) in entries {
        let Some(owner) = CallbackOwner::new(mod_id) else {
            log::warn!("{label} {id} has no installed owner {mod_id}");
            CallbackOwner::for_mod(mod_id).dispose(label, "drop_decoder", decode);
            continue;
        };
        let key = id.to_string();
        let entry = ItemProviderEntry {
            decode: CallbackValue::new(owner, label, "drop_decoder", decode),
        };
        if let Some(earlier) = bound.insert(key.clone(), entry) {
            log::warn!(
                "Sunrise: {label} {key} registered by {} is replaced by {mod_id}'s registration",
                earlier.decode.owner().id()
            );
        } else if is_builtin(&key) {
            log::warn!(
                "Sunrise: {label} {key} registered by the game is replaced by {mod_id}'s registration"
            );
        }
        if seen.insert(key.clone()) {
            kept.push(key);
        }
    }
    (bound, kept)
}

/// Bind a batch of attributed item tint source registrations without sealing process globals.
pub fn bind_item_tint_factories(
    sources: Vec<Answered<ItemTintSourceRegistration>>,
) -> (ItemTintFactories, Vec<String>) {
    bind_factories(
        "item tint source",
        is_builtin_tint_source,
        sources
            .into_iter()
            .map(|entry| (entry.mod_id, entry.answer.id, entry.answer.decode)),
    )
}

/// Bind a batch of attributed item model registrations without sealing process globals.
pub fn bind_item_model_factories(
    models: Vec<Answered<ItemModelRegistration>>,
) -> (ItemModelFactories, Vec<String>) {
    bind_factories(
        "item model type",
        is_builtin_item_model,
        models
            .into_iter()
            .map(|entry| (entry.mod_id, entry.answer.id, entry.answer.decode)),
    )
}

/// Bind a batch of attributed numeric `range_dispatch` property registrations without sealing
/// process globals.
pub fn bind_range_property_factories(
    properties: Vec<Answered<RangeSelectPropertyRegistration>>,
) -> (RangePropertyFactories, Vec<String>) {
    bind_factories(
        "item model range property",
        is_builtin_range_property,
        properties
            .into_iter()
            .map(|entry| (entry.mod_id, entry.answer.id, entry.answer.decode)),
    )
}

/// Bind a batch of attributed categorical `select` property registrations without sealing process
/// globals.
pub fn bind_select_property_factories(
    properties: Vec<Answered<SelectPropertyRegistration>>,
) -> (SelectPropertyFactories, Vec<String>) {
    bind_factories(
        "item model select property",
        is_builtin_select_property,
        properties
            .into_iter()
            .map(|entry| (entry.mod_id, entry.answer.id, entry.answer.decode)),
    )
}

/// Install the bound item tint source decoders once at client startup, before resources load.
pub fn install_item_tint_factories(
    sources: Vec<Answered<ItemTintSourceRegistration>>,
) -> Vec<String> {
    let (bound, kept) = bind_item_tint_factories(sources);
    if TINT_FACTORIES.set(Arc::new(bound)).is_err() {
        log::warn!("item tint sources were registered already; these are left out");
        return Vec::new();
    }
    kept
}

/// Install the bound item model node decoders once at client startup, before resources load.
pub fn install_item_model_factories(models: Vec<Answered<ItemModelRegistration>>) -> Vec<String> {
    let (bound, kept) = bind_item_model_factories(models);
    if ITEM_MODEL_FACTORIES.set(Arc::new(bound)).is_err() {
        log::warn!("item model types were registered already; these are left out");
        return Vec::new();
    }
    kept
}

/// Install the bound numeric `range_dispatch` property decoders once at client startup, before
/// resources load.
pub fn install_range_property_factories(
    properties: Vec<Answered<RangeSelectPropertyRegistration>>,
) -> Vec<String> {
    let (bound, kept) = bind_range_property_factories(properties);
    if RANGE_PROPERTY_FACTORIES.set(Arc::new(bound)).is_err() {
        log::warn!("item model range properties were registered already; these are left out");
        return Vec::new();
    }
    kept
}

/// Install the bound categorical `select` property decoders once at client startup, before
/// resources load.
pub fn install_select_property_factories(
    properties: Vec<Answered<SelectPropertyRegistration>>,
) -> Vec<String> {
    let (bound, kept) = bind_select_property_factories(properties);
    if SELECT_PROPERTY_FACTORIES.set(Arc::new(bound)).is_err() {
        log::warn!("item model select properties were registered already; these are left out");
        return Vec::new();
    }
    kept
}

fn decode_entry<D, F>(
    factories: Option<&BTreeMap<String, ItemProviderEntry<D>>>,
    kind: &str,
    fields: &Map<String, Value>,
    callback_kind: &'static str,
) -> Result<Option<CallbackValue<Arc<F>>>, String>
where
    D: Fn(&Tag) -> Result<Arc<F>, String> + ?Sized,
    F: ?Sized,
{
    let Some(entry) = factories.and_then(|table| table.get(kind)) else {
        return Ok(None);
    };
    let config = sunrise_core::json_nbt::json_to_compound(fields)
        .map(Tag::Compound)
        .map_err(|error| {
            format!("{callback_kind} {kind}: its JSON doesn't convert to NBT: {error}")
        })?;

    let owner = entry.decode.owner();
    match owner.call(callback_kind, "decode", || {
        (entry.decode.value())(&config)
            .map(|eval| CallbackValue::new(owner.clone(), callback_kind, "drop_evaluator", eval))
    }) {
        Ok(Ok(eval)) => Ok(Some(eval)),
        Ok(Err(reason)) => Err(format!(
            "mod {} {callback_kind} {kind} refused it: {reason}",
            owner.id()
        )),
        Err(error) => Err(format!("{callback_kind} {kind}: {error}")),
    }
}

/// Decode a registered tint source entry into a bound [`RegisteredTintSource`].
pub fn decode_tint_source(
    factories: Option<&ItemTintFactories>,
    kind: &str,
    fields: &Map<String, Value>,
) -> Result<Option<RegisteredTintSource>, String> {
    decode_entry(factories, kind, fields, "item tint source").map(|bound| {
        bound.map(|eval| {
            RegisteredTintSource(Arc::new(BoundTintSource {
                kind: kind.into(),
                eval,
            }))
        })
    })
}

struct ChildrenBuilder<'a> {
    fields: &'a Map<String, Value>,
    decode_child: &'a dyn Fn(&Value) -> Result<ItemModelNode, DecodeError>,
    models: Vec<(String, ItemModelNode)>,
    next_generated: usize,
    transformation: Option<sunrise_assets::math::Mat4>,
}

impl ChildrenBuilder<'_> {
    fn insert(&mut self, name: String, node: ItemModelNode) -> Result<(), String> {
        if self.models.iter().any(|(existing, _)| existing == &name) {
            return Err(format!("duplicate child model name {name:?}"));
        }
        self.models.push((name, node));
        Ok(())
    }
}

impl ItemModelChildren for ChildrenBuilder<'_> {
    fn add_field(&mut self, name: &str, field: &str) -> Result<(), String> {
        let value = self
            .fields
            .get(field)
            .ok_or_else(|| format!("missing child model field {field:?}"))?;
        let node = (self.decode_child)(value).map_err(|e| e.to_string())?;
        self.insert(name.to_owned(), node)
    }

    fn add_list(&mut self, field: &str) -> Result<Vec<String>, String> {
        let items = self
            .fields
            .get(field)
            .and_then(Value::as_array)
            .ok_or_else(|| format!("expected array at child model field {field:?}"))?;
        let mut names = Vec::with_capacity(items.len());
        for value in items {
            let node = (self.decode_child)(value).map_err(|e| e.to_string())?;
            let name = format!("__list_{}", self.next_generated);
            self.next_generated += 1;
            self.insert(name.clone(), node)?;
            names.push(name);
        }
        Ok(names)
    }

    fn add_object(&mut self, field: &str) -> Result<Vec<String>, String> {
        let entries = self
            .fields
            .get(field)
            .and_then(Value::as_object)
            .ok_or_else(|| format!("expected object at child model field {field:?}"))?;
        let mut names = Vec::with_capacity(entries.len());
        for (name, value) in entries {
            let node = (self.decode_child)(value).map_err(|e| e.to_string())?;
            self.insert(name.clone(), node)?;
            names.push(name.clone());
        }
        Ok(names)
    }

    fn add_named_models(&mut self) -> Result<Vec<String>, String> {
        if !self.fields.contains_key("models") {
            return Ok(Vec::new());
        }
        self.add_object("models")
    }

    fn use_transformation(&mut self) -> Result<(), String> {
        self.transformation = optional_transformation(self.fields).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn add_model(&mut self, name: &str, model: Identifier) -> Result<(), String> {
        self.insert(
            name.to_owned(),
            ItemModelNode::Model(ModelLeaf {
                model: model.to_string(),
                transformation: None,
                tints: Vec::new(),
            }),
        )
    }
}

/// Decode a registered item model node into a bound [`DecodedItemModel`].
pub fn decode_item_model(
    factories: Option<&ItemModelFactories>,
    kind: &str,
    fields: &Map<String, Value>,
    decode_child: &dyn Fn(&Value) -> Result<ItemModelNode, DecodeError>,
) -> Result<Option<DecodedItemModel>, String> {
    let Some(entry) = factories.and_then(|table| table.get(kind)) else {
        return Ok(None);
    };
    let config = new_item_model_config(fields);

    let mut children = ChildrenBuilder {
        fields,
        decode_child,
        models: Vec::new(),
        next_generated: 0,
        transformation: None,
    };
    let owner = entry.decode.owner();
    match owner.call("item model", "decode", || {
        (entry.decode.value())(&config, &mut children)
            .map(|eval| CallbackValue::new(owner.clone(), "item model", "drop_evaluator", eval))
    }) {
        Ok(Ok(eval)) => Ok(Some(DecodedItemModel {
            model: RegisteredItemModel(Arc::new(BoundItemModel {
                kind: kind.into(),
                eval,
            })),
            transformation: children.transformation,
            models: children.models,
        })),
        Ok(Err(reason)) => Err(format!(
            "mod {} item model {kind} refused it: {reason}",
            owner.id()
        )),
        Err(error) => Err(format!("item model {kind}: {error}")),
    }
}

/// Decode a registered numeric `range_dispatch` property into a bound [`RegisteredRangeProperty`].
pub fn decode_range_property(
    factories: Option<&RangePropertyFactories>,
    kind: &str,
    fields: &Map<String, Value>,
) -> Result<Option<RegisteredRangeProperty>, String> {
    decode_entry(factories, kind, fields, "item model range property").map(|bound| {
        bound.map(|eval| {
            RegisteredRangeProperty(Arc::new(BoundRangeProperty {
                kind: kind.into(),
                eval,
            }))
        })
    })
}

/// Decode a registered categorical `select` property into a bound [`RegisteredSelectProperty`].
pub fn decode_select_property(
    factories: Option<&SelectPropertyFactories>,
    kind: &str,
    fields: &Map<String, Value>,
) -> Result<Option<RegisteredSelectProperty>, String> {
    const CALLBACK_KIND: &str = "item model select property";
    let Some(entry) = factories.and_then(|table| table.get(kind)) else {
        return Ok(None);
    };
    let config = sunrise_core::json_nbt::json_to_compound(fields)
        .map(Tag::Compound)
        .map_err(|error| {
            format!("{CALLBACK_KIND} {kind}: its JSON doesn't convert to NBT: {error}")
        })?;

    let owner = entry.decode.owner();
    let (decode_case, eval) = match owner.call(CALLBACK_KIND, "decode", || {
        (entry.decode.value())(&config).map(|(decode_case, eval)| {
            (
                CallbackValue::new(
                    owner.clone(),
                    CALLBACK_KIND,
                    "drop_case_decoder",
                    decode_case,
                ),
                CallbackValue::new(owner.clone(), CALLBACK_KIND, "drop_evaluator", eval),
            )
        })
    }) {
        Ok(Ok(pair)) => pair,
        Ok(Err(reason)) => {
            return Err(format!(
                "mod {} {CALLBACK_KIND} {kind} refused it: {reason}",
                owner.id()
            ));
        }
        Err(error) => return Err(format!("{CALLBACK_KIND} {kind}: {error}")),
    };
    let compare_owner = owner.clone();
    let kind_owned: Box<str> = kind.into();
    let case_kind = kind_owned.clone();
    let bound = BoundSelectProperty {
        kind: kind_owned,
        eval,
    };
    Ok(Some(RegisteredSelectProperty::new(
        Arc::new(bound),
        move |value| {
            let tag = sunrise_core::json_nbt::json_to_tag(value).map_err(|error| {
                SelectCaseDecodeError::Mismatch(format!(
                    "{CALLBACK_KIND} {case_kind}: case value doesn't convert to NBT: {error}"
                ))
            })?;

            let owner = decode_case.owner();
            match owner.call(CALLBACK_KIND, "decode_case", || {
                (decode_case.value())(&tag).map(|key| {
                    CallbackValue::new(owner.clone(), CALLBACK_KIND, "drop_case_key", key)
                })
            }) {
                Ok(Ok(key)) => Ok(RegisteredSelectKey::new(key)),
                Ok(Err(reason)) => Err(SelectCaseDecodeError::Mismatch(format!(
                    "mod {} {CALLBACK_KIND} {case_kind} refused case: {reason}",
                    owner.id()
                ))),
                Err(error) => Err(SelectCaseDecodeError::Callback(format!(
                    "{CALLBACK_KIND} {case_kind}: {error}"
                ))),
            }
        },
        move |left, right| {
            compare_owner.call(CALLBACK_KIND, "compare_case", || {
                left.downcast_ref::<CallbackValue<SelectCaseKey>>()
                    .map(CallbackValue::value)
                    == right
                        .downcast_ref::<CallbackValue<SelectCaseKey>>()
                        .map(CallbackValue::value)
            })
        },
    )))
}

#[cfg(test)]
pub(super) fn decode_definition_without_specials(
    text: &str,
    providers: &ItemProviderTables<'_>,
) -> Result<sunrise_assets::item_model::ClientItem, sunrise_assets::DecodeError> {
    use sunrise_assets::item_model::{
        ClientItem, ItemModelDecoders, ItemModelLoader, RangePropertyLoader, SelectPropertyLoader,
        TintSourceLoader,
    };
    let tint =
        |kind: &str, fields: &Map<String, Value>| decode_tint_source(providers.tint, kind, fields);
    let item_model =
        |kind: &str,
         fields: &Map<String, Value>,
         decode_child: &dyn Fn(&Value) -> Result<ItemModelNode, DecodeError>| {
            decode_item_model(providers.item_model, kind, fields, decode_child)
        };
    let range_property = |kind: &str, fields: &Map<String, Value>| {
        decode_range_property(providers.range_property, kind, fields)
    };
    let select_property = |kind: &str, fields: &Map<String, Value>| {
        decode_select_property(providers.select_property, kind, fields)
    };
    let tint_ref: &TintSourceLoader<'_> = &tint;
    let item_model_ref: &ItemModelLoader<'_> = &item_model;
    let range_property_ref: &RangePropertyLoader<'_> = &range_property;
    let select_property_ref: &SelectPropertyLoader<'_> = &select_property;
    let decoders = ItemModelDecoders {
        special: None,
        tint: providers.tint.is_some().then_some(tint_ref),
        item_model: providers.item_model.is_some().then_some(item_model_ref),
        range_property: providers
            .range_property
            .is_some()
            .then_some(range_property_ref),
        select_property: providers
            .select_property
            .is_some()
            .then_some(select_property_ref),
    };
    ClientItem::from_json_with_decoders(text, &decoders)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::{
        __private::{game::RegistrationContextNew, parts_installation::item_key, take_hook_panics},
        ClientHost, Hooks, ItemStack, ItemTintInput, ModEntry, RegisterItemTintSourcesEvent,
        RegistrationContext,
        testing::FakeHost,
    };
    use sunrise_assets::item_model::{
        ClientItem, Context, DisplayContext, ItemEnv, ItemModelNode, ItemModels, LayerContent,
        StackView, TintSource,
    };

    fn id(text: &str) -> Identifier {
        Identifier::parse(text).unwrap()
    }

    #[test]
    fn tint_registration_accepts_minecraft_overrides_builtins_and_last_writer_wins() {
        struct First;
        #[mod_api::__subscriptions]
        impl Hooks for First {
            fn register_item_tint_sources(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterItemTintSourcesEvent<'_>,
            ) {
                event.register(id("minecraft:constant"), |_| {
                    Ok(|_: &dyn ClientHost, _: &ItemTintInput<'_>| 0xFF11_1111)
                });
                event.register(id("minecraft:custom_tint"), |_| {
                    Ok(|_: &dyn ClientHost, _: &ItemTintInput<'_>| 0xFF22_2222)
                });
            }
        }
        struct Second;
        #[mod_api::__subscriptions]
        impl Hooks for Second {
            fn register_item_tint_sources(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterItemTintSourcesEvent<'_>,
            ) {
                event.register(id("minecraft:constant"), |_| {
                    Ok(|_: &dyn ClientHost, _: &ItemTintInput<'_>| 0xFF33_3333)
                });
            }
        }

        let mods: mod_api::Mods = vec![
            ModEntry::new("first", Box::new(First)),
            ModEntry::new("second", Box::new(Second)),
        ]
        .into();
        mod_platform::testing::with_mods(mods, || {
            let mut sources = Vec::new();
            mod_api::dispatch_register_item_tint_sources(&RegistrationContext::new(), &mut sources);
            let (factories, kept) = bind_item_tint_factories(sources);
            assert_eq!(kept, ["minecraft:constant", "minecraft:custom_tint"]);

            let providers = ItemProviderTables {
                tint: Some(&factories),
                ..Default::default()
            };
            let item = decode_definition_without_specials(
                r#"{"model":{"type":"minecraft:model","model":"minecraft:item/stick","tints":[
                    {"type":"minecraft:constant","value":0},
                    {"type":"minecraft:custom_tint"}
                ]}}"#,
                &providers,
            )
            .unwrap();
            let ItemModelNode::Model(leaf) = item.model else {
                panic!("expected model leaf");
            };
            let stack = ItemStack::new(item_key(0), 1).unwrap();
            let input = ItemTintInput::new(&stack, -1);
            let host = FakeHost::new();
            let colors: Vec<u32> = leaf
                .tints
                .iter()
                .map(|tint| {
                    let TintSource::Registered(registered) = tint else {
                        panic!("expected bound tint source");
                    };
                    let bound = registered.0.downcast_ref::<BoundTintSource>().unwrap();
                    (bound.eval.value())(&host, &input)
                })
                .collect();
            assert_eq!(colors, [0xFF33_3333, 0xFF22_2222]);
        });
    }

    #[test]
    fn decode_error_does_not_latch_and_decode_panic_poisons_only_its_owner() {
        struct Refusing;
        #[mod_api::__subscriptions]
        impl Hooks for Refusing {
            fn register_item_tint_sources(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterItemTintSourcesEvent<'_>,
            ) {
                event.register(id("provtest:strict"), |config| match config.get("color") {
                    Some(Tag::Int(color)) => {
                        let color = *color as u32;
                        Ok(move |_: &dyn ClientHost, _: &ItemTintInput<'_>| color)
                    }
                    _ => Err("missing integer color".into()),
                });
            }
        }
        struct Panicking;
        #[mod_api::__subscriptions]
        impl Hooks for Panicking {
            fn register_item_tint_sources(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterItemTintSourcesEvent<'_>,
            ) {
                event.register(
                    id("provtest:boom"),
                    |_| -> Result<fn(&dyn ClientHost, &ItemTintInput<'_>) -> u32, String> {
                        panic!("decoder boom")
                    },
                );
            }
        }

        let mods: mod_api::Mods = vec![
            ModEntry::new("refusing", Box::new(Refusing)),
            ModEntry::new("panicking", Box::new(Panicking)),
        ]
        .into();
        mod_platform::testing::with_mods(mods, || {
            let _ = take_hook_panics();
            let mut sources = Vec::new();
            mod_api::dispatch_register_item_tint_sources(&RegistrationContext::new(), &mut sources);
            let (factories, _) = bind_item_tint_factories(sources);
            let providers = ItemProviderTables {
                tint: Some(&factories),
                ..Default::default()
            };

            // A decode refusal rejects that item definition without latching or triggering a panic notice.
            assert!(
                decode_definition_without_specials(
                    r#"{"model":{"type":"minecraft:model","model":"minecraft:item/stick","tints":[{"type":"provtest:strict"}]}}"#,
                    &providers,
                )
                .is_err()
            );
            assert!(take_hook_panics().is_empty());

            // A nested null fails JSON->NBT validation without latching.
            assert!(
                decode_definition_without_specials(
                    r#"{"model":{"type":"minecraft:model","model":"minecraft:item/stick","tints":[{"type":"provtest:strict","color":32768,"extra":null}]}}"#,
                    &providers,
                )
                .is_err()
            );

            // A valid definition still decodes cleanly afterward.
            assert!(
                decode_definition_without_specials(
                    r#"{"model":{"type":"minecraft:model","model":"minecraft:item/stick","tints":[{"type":"provtest:strict","color":32768}]}}"#,
                    &providers,
                )
                .is_ok()
            );

            // A panic in a decoder poisons that mod's CallbackOwner and queues a panic notice.
            assert!(
                decode_definition_without_specials(
                    r#"{"model":{"type":"minecraft:model","model":"minecraft:item/stick","tints":[{"type":"provtest:boom"}]}}"#,
                    &providers,
                )
                .is_err()
            );
            assert_eq!(
                take_hook_panics()
                    .iter()
                    .map(|notice| (notice.mod_id, notice.hook))
                    .collect::<Vec<_>>(),
                [("panicking", "decode")]
            );
            // Subsequent calls stay disabled without panicking again, and other mods still decode.
            assert!(
                decode_definition_without_specials(
                    r#"{"model":{"type":"minecraft:model","model":"minecraft:item/stick","tints":[{"type":"provtest:boom"}]}}"#,
                    &providers,
                )
                .is_err()
            );
            assert!(take_hook_panics().is_empty());
            assert!(
                decode_definition_without_specials(
                    r#"{"model":{"type":"minecraft:model","model":"minecraft:item/stick","tints":[{"type":"provtest:strict","color":65536}]}}"#,
                    &providers,
                )
                .is_ok()
            );
        });
    }

    /// A registered replacement for `minecraft:composite` decodes unmodified vanilla JSON (whose
    /// `"models"` field is a JSON array, not an object) via `ItemModelChildren::add_list` and
    /// resolves its layers without errors.
    #[test]
    fn replacing_vanilla_composite_decodes_unmodified_vanilla_array_schema() {
        use mod_api::{ItemModelLayer, RegisterItemModelsEvent};
        use sunrise_assets::item_model::{LayerTransform, ModelChoice};

        struct CompositeOverride;
        #[mod_api::__subscriptions]
        impl Hooks for CompositeOverride {
            fn register_item_models(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterItemModelsEvent<'_>,
            ) {
                event.register(id("minecraft:composite"), |_, children| {
                    children.use_transformation()?;
                    let names = children.add_list("models")?;
                    Ok(
                        move |_: &dyn ClientHost,
                              _: &mod_api::ItemModelUpdateInput<'_>,
                              layers: &mut Vec<ItemModelLayer>| {
                            for name in names.iter().rev() {
                                layers.push(ItemModelLayer::new(name.clone()));
                            }
                        },
                    )
                });
            }
        }

        struct LocalEnv;
        impl ItemEnv for LocalEnv {
            fn item_model_update(
                &self,
                model: &RegisteredItemModel,
                _stack: &StackView,
                _ctx: &Context,
                out: &mut Vec<ModelChoice>,
            ) {
                let bound = model.0.downcast_ref::<BoundItemModel>().unwrap();
                let stack = ItemStack::new(item_key(0), 1).unwrap();
                let ctx =
                    mod_api::ItemModelContext::new(mod_api::ItemDisplayContext::Gui, None, None, 0);
                let input = mod_api::ItemModelUpdateInput::new(&stack, -1, &ctx);
                let mut layers = Vec::new();
                (bound.eval.value())(&FakeHost::new(), &input, &mut layers);
                out.extend(layers.into_iter().map(|layer| ModelChoice {
                    model: layer.model,
                    transform: LayerTransform::Model,
                }));
            }
        }

        let mods: mod_api::Mods =
            vec![ModEntry::new("comp_override", Box::new(CompositeOverride))].into();
        mod_platform::testing::with_mods(mods, || {
            let mut models = Vec::new();
            mod_api::dispatch_register_item_models(&RegistrationContext::new(), &mut models);
            let (factories, _) = bind_item_model_factories(models);
            let providers = ItemProviderTables {
                item_model: Some(&factories),
                ..Default::default()
            };
            // Unmodified vanilla minecraft:composite JSON: `models` is an array.
            let item = decode_definition_without_specials(
                r#"{"model":{"type":"minecraft:composite","transformation":[1,0,0,1,0,1,0,2,0,0,1,3,0,0,0,1],"models":[
                    {"type":"minecraft:model","model":"minecraft:item/stick"},
                    {"type":"minecraft:model","model":"minecraft:item/apple"}
                ]}}"#,
                &providers,
            )
            .unwrap();
            let mut defs = ItemModels::default();
            defs.definitions.insert("minecraft:stick".into(), item);
            let mut stack = StackView::new("minecraft:stick", 1);
            stack.patch.insert(
                "minecraft:item_model".into(),
                Some(serde_json::json!("minecraft:stick")),
            );
            let resolved = defs.resolve(
                &stack,
                &Context {
                    display: DisplayContext::Gui,
                    owner: None,
                    level: None,
                    seed: 0,
                },
                &LocalEnv,
            );
            assert!(
                resolved
                    .layers
                    .iter()
                    .all(|layer| layer.transform.rows().map(|row| row[3]) == [1.0, 2.0, 3.0, 1.0])
            );
            let layer_models: Vec<&str> = resolved
                .layers
                .iter()
                .map(|layer| match &layer.content {
                    LayerContent::Model { model } => model.as_str(),
                    other => panic!("expected model layer, got {other:?}"),
                })
                .collect();
            assert_eq!(
                layer_models,
                ["minecraft:item/apple", "minecraft:item/stick"]
            );
        });
    }

    struct SelectEnv;
    impl ItemEnv for SelectEnv {
        fn item_model_select_property(
            &self,
            property: &RegisteredSelectProperty,
            stack: &StackView,
            _ctx: &Context,
            keys: &mut dyn Iterator<Item = (usize, &RegisteredSelectKey)>,
        ) -> Option<usize> {
            let bound = property.payload.downcast_ref::<BoundSelectProperty>()?;
            let api_stack = ItemStack::new(item_key(0), stack.count as u32).unwrap();
            let ctx =
                mod_api::ItemModelContext::new(mod_api::ItemDisplayContext::Gui, None, None, 0);
            let input = mod_api::ItemModelPropertyInput::new(&api_stack, -1, &ctx);
            bound.select(&FakeHost::new(), &input, keys)
        }
    }

    fn selected_model(item: ClientItem, count: i32) -> String {
        let mut defs = ItemModels::default();
        defs.definitions.insert("minecraft:stick".into(), item);
        let mut stack = StackView::new("minecraft:stick", count);
        stack.patch.insert(
            "minecraft:item_model".into(),
            Some(serde_json::json!("minecraft:stick")),
        );
        let resolved = defs.resolve(
            &stack,
            &Context {
                display: DisplayContext::Gui,
                owner: None,
                level: None,
                seed: 0,
            },
            &SelectEnv,
        );
        match &resolved.layers[0].content {
            LayerContent::Model { model } => model.clone(),
            other => panic!("expected model layer, got {other:?}"),
        }
    }

    /// A registered replacement for `minecraft:component` supplies a typed case decoder so
    /// non-string vanilla `when` values (such as integers) decode and match cleanly.
    #[test]
    fn replacing_vanilla_component_select_supports_typed_numeric_when_cases() {
        use mod_api::RegisterSelectItemModelPropertyEvent;

        struct ComponentOverride;
        #[mod_api::__subscriptions]
        impl Hooks for ComponentOverride {
            fn register_select_item_model_property(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterSelectItemModelPropertyEvent<'_>,
            ) {
                event.register(id("minecraft:component"), |_| {
                    let decode_case = |when: &Tag| match when {
                        Tag::Byte(v) => Ok(i32::from(*v)),
                        Tag::Short(v) => Ok(i32::from(*v)),
                        Tag::Int(v) => Ok(*v),
                        _ => Err("expected integer case".into()),
                    };
                    let eval = |_: &dyn ClientHost, input: &mod_api::ItemModelPropertyInput<'_>| {
                        Some(input.stack.count() as i32)
                    };
                    Ok((decode_case, eval))
                });
            }
        }

        let mods: mod_api::Mods =
            vec![ModEntry::new("comp_select", Box::new(ComponentOverride))].into();
        mod_platform::testing::with_mods(mods, || {
            let mut properties = Vec::new();
            mod_api::dispatch_register_select_item_model_property(
                &RegistrationContext::new(),
                &mut properties,
            );
            let (factories, _) = bind_select_property_factories(properties);
            let providers = ItemProviderTables {
                select_property: Some(&factories),
                ..Default::default()
            };
            let item = decode_definition_without_specials(
                r#"{"model":{
                    "type":"minecraft:select",
                    "property":"minecraft:component",
                    "component":"minecraft:damage",
                    "cases":[
                        {"when":1,"model":{"type":"minecraft:model","model":"minecraft:item/stick"}},
                        {"when":[2,3],"model":{"type":"minecraft:model","model":"minecraft:item/apple"}}
                    ],
                    "fallback":{"type":"minecraft:model","model":"minecraft:item/bone"}
                }}"#,
                &providers,
            )
            .unwrap();
            // A non-integer case is refused by the registered case decoder.
            assert!(
                decode_definition_without_specials(
                    r#"{"model":{
                        "type":"minecraft:select",
                        "property":"minecraft:component",
                        "component":"minecraft:damage",
                        "cases":[{"when":"not_a_number","model":{"type":"minecraft:empty"}}]
                    }}"#,
                    &providers,
                )
                .is_err()
            );
            assert_eq!(selected_model(item.clone(), 1), "minecraft:item/stick");
            assert_eq!(selected_model(item.clone(), 2), "minecraft:item/apple");
            assert_eq!(selected_model(item, 4), "minecraft:item/bone");
        });
    }

    #[test]
    fn replacement_item_model_owns_its_input_schema() {
        use mod_api::RegisterItemModelsEvent;

        struct EmptyOverride;
        #[mod_api::__subscriptions]
        impl Hooks for EmptyOverride {
            fn register_item_models(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterItemModelsEvent<'_>,
            ) {
                event.register(id("minecraft:empty"), |config, children| {
                    #[derive(serde::Deserialize)]
                    struct Config<'a> {
                        transformation: &'a str,
                    }
                    let config: Config<'_> = config.decode()?;
                    if config.transformation != "spin" {
                        return Err("expected spin".into());
                    }
                    children.add_field("child", "model")?;
                    Ok(
                        |_: &dyn ClientHost,
                         _: &mod_api::ItemModelUpdateInput<'_>,
                         _: &mut Vec<mod_api::ItemModelLayer>| {},
                    )
                });
            }
        }
        mod_platform::testing::with_mods(
            vec![ModEntry::new("schema_test", Box::new(EmptyOverride))].into(),
            || {
                let mut registered = Vec::new();
                mod_api::dispatch_register_item_models(
                    &RegistrationContext::new(),
                    &mut registered,
                );
                let (factories, _) = bind_item_model_factories(registered);
                let providers = ItemProviderTables {
                    item_model: Some(&factories),
                    ..Default::default()
                };
                let item = decode_definition_without_specials(
                    r#"{"model":{"type":"minecraft:empty","transformation":"spin","ignored":null,
                    "model":{"type":"minecraft:model","model":"minecraft:item/apple"}}}"#,
                    &providers,
                )
                .unwrap();
                let ItemModelNode::Registered {
                    transformation,
                    models,
                    ..
                } = item.model
                else {
                    panic!("expected replacement model");
                };
                assert!(transformation.is_none());
                assert!(matches!(&models[..], [(name, ItemModelNode::Model(leaf))]
                if name == "child" && leaf.model == "minecraft:item/apple"));
            },
        );
    }

    #[test]
    fn a_select_supports_both_single_array_keys_and_arrays_of_keys() {
        use mod_api::RegisterSelectItemModelPropertyEvent;

        struct VectorCases;
        #[mod_api::__subscriptions]
        impl Hooks for VectorCases {
            fn register_select_item_model_property(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterSelectItemModelPropertyEvent<'_>,
            ) {
                event.register(id("selecttest:vector"), |_| {
                    Ok((
                        |when: &Tag| match when {
                            Tag::List(values) => match values.as_slice() {
                                [Tag::Byte(x), Tag::Byte(y)] => Ok([*x, *y]),
                                _ => Err("expected two coordinates".into()),
                            },
                            _ => Err("expected vector".into()),
                        },
                        |_: &dyn ClientHost, input: &mod_api::ItemModelPropertyInput<'_>| {
                            let count = input.stack.count() as i8;
                            Some([count, count + 1])
                        },
                    ))
                });
            }
        }
        mod_platform::testing::with_mods(
            vec![ModEntry::new("vector_cases", Box::new(VectorCases))].into(),
            || {
                let mut registered = Vec::new();
                mod_api::dispatch_register_select_item_model_property(
                    &RegistrationContext::new(),
                    &mut registered,
                );
                let (factories, _) = bind_select_property_factories(registered);
                let providers = ItemProviderTables {
                    select_property: Some(&factories),
                    ..Default::default()
                };
                let item = decode_definition_without_specials(
                r#"{"model":{"type":"minecraft:select","property":"selecttest:vector","cases":[
                    {"when":[1,2],"model":{"type":"minecraft:model","model":"minecraft:item/stick"}},
                    {"when":[[2,3],[3,4]],"model":{"type":"minecraft:model","model":"minecraft:item/apple"}}],
                    "fallback":{"type":"minecraft:model","model":"minecraft:item/bone"}}}"#,
                &providers,
            ).unwrap();
                assert_eq!(selected_model(item.clone(), 1), "minecraft:item/stick");
                assert_eq!(selected_model(item.clone(), 2), "minecraft:item/apple");
                assert_eq!(selected_model(item.clone(), 3), "minecraft:item/apple");
                assert_eq!(selected_model(item, 4), "minecraft:item/bone");
            },
        );
    }

    #[test]
    fn a_select_key_equality_panic_is_contained_during_load_and_draw() {
        use mod_api::RegisterSelectItemModelPropertyEvent;

        #[derive(Debug)]
        struct PanicKey;
        impl PartialEq for PanicKey {
            fn eq(&self, _: &Self) -> bool {
                panic!("arbitrary mod equality")
            }
        }
        impl Eq for PanicKey {}

        struct PanicCases;
        #[mod_api::__subscriptions]
        impl Hooks for PanicCases {
            fn register_select_item_model_property(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterSelectItemModelPropertyEvent<'_>,
            ) {
                for id_str in ["selecttest:panic_in_load", "selecttest:panic_in_draw"] {
                    event.register(id(id_str), |_| {
                        Ok((
                            |_: &Tag| Ok(PanicKey),
                            |_: &dyn ClientHost, _: &mod_api::ItemModelPropertyInput<'_>| {
                                Some(PanicKey)
                            },
                        ))
                    });
                }
            }
        }
        mod_platform::testing::with_mods(
            vec![ModEntry::new("eq_panicker", Box::new(PanicCases))].into(),
            || {
                let mut registered = Vec::new();
                mod_api::dispatch_register_select_item_model_property(
                    &RegistrationContext::new(),
                    &mut registered,
                );
                let (factories, _) = bind_select_property_factories(registered);
                let providers = ItemProviderTables {
                    select_property: Some(&factories),
                    ..Default::default()
                };
                let broken = r#"{"model":{"type":"minecraft:select","property":"selecttest:panic_in_load",
                "cases":[{"when":[1,2],"model":{"type":"minecraft:empty"}}]}}"#;
                let error = decode_definition_without_specials(broken, &providers)
                    .unwrap_err()
                    .to_string();
                assert!(
                    error.contains("eq_panicker") && error.contains("panicked"),
                    "{error}"
                );
                let load_owner = factories["selecttest:panic_in_load"].decode.owner();
                let draw_owner = factories["selecttest:panic_in_draw"].decode.owner();
                assert!(!load_owner.is_enabled());
                assert!(draw_owner.is_enabled());
                let item = decode_definition_without_specials(
                r#"{"model":{"type":"minecraft:select","property":"selecttest:panic_in_draw",
                    "cases":[{"when":1,"model":{"type":"minecraft:model","model":"minecraft:item/apple"}}],
                    "fallback":{"type":"minecraft:model","model":"minecraft:item/bone"}}}"#,
                &providers,
            ).unwrap();
                assert_eq!(selected_model(item.clone(), 1), "minecraft:item/bone");
                assert!(!draw_owner.is_enabled());
                assert_eq!(selected_model(item, 1), "minecraft:item/bone");
            },
        );
    }
}
