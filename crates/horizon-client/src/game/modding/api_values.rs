// game-module-visibility: pub(crate)
//! The game-owned conversion boundary between API records and engine values.
//! Rust's orphan rule prevents foreign From/TryFrom implementations here.
use horizon_core::nbt::{Compound, JavaString, Tag};
use horizon_items::items::{ItemStack as EngineStack, Items};
use mod_api::item_stack::{ItemStack as ApiStack, StackWriteError};

/// Registry access for generated stack write-back in full and narrowed hook hosts.
pub(in crate::game) trait ItemBoundary {
    fn boundary_items(&self) -> Option<&Items>;
}
impl ItemBoundary for super::Game<'_> {
    fn boundary_items(&self) -> Option<&Items> {
        Some(self.items())
    }
}
impl ItemBoundary for super::GameLevel<'_> {
    fn boundary_items(&self) -> Option<&Items> {
        Some(self.items)
    }
}
impl ItemBoundary for super::native_hooks::HitLevel<'_> {
    fn boundary_items(&self) -> Option<&Items> {
        self.0.tags()
    }
}

/// Normalize an existing game write's answer without changing its implementation.
/// Generated API forwarders add the checked-input refusal around successful native answers.
pub(in crate::game) trait IntoWriteResult {
    type Value;
    type Error;
    fn into_write_result(self) -> Result<Self::Value, Self::Error>;
}
impl<T> IntoWriteResult for Option<T> {
    type Value = Self;
    type Error = StackWriteError;
    fn into_write_result(self) -> Result<Self, Self::Error> {
        Ok(self)
    }
}
impl<T, E> IntoWriteResult for Result<T, E> {
    type Value = T;
    type Error = E;
    fn into_write_result(self) -> Self {
        self
    }
}
macro_rules! write_answers {
    ($($ty:ty),* $(,)?) => { $(
        impl IntoWriteResult for $ty {
            type Value = Self;
            type Error = StackWriteError;
            fn into_write_result(self) -> Result<Self, Self::Error> { Ok(self) }
        }
    )* };
}
write_answers!(bool, u32, ());

/// Convert an engine value into an owned API value after resolving registry identities.
pub(in crate::game) trait TryIntoApi {
    type Api;
    type Error;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error>;
}

/// Tests use the production snapshot conversion, never a second fixture codec.
#[cfg(test)]
pub(in crate::game) fn test_snapshot<T: TryIntoApi + ?Sized>(value: &T) -> T::Api
where
    T::Error: std::fmt::Debug,
{
    value
        .try_into_api()
        .expect("fixture snapshot has resolved item keys and a nonempty count")
}

/// Validate a mod value against the game's registry before admitting it to engine state.
pub(in crate::game) trait TryIntoEngine<Context: ?Sized = Items> {
    type Engine;
    type Error;
    fn try_into_engine(&self, context: &Context) -> Result<Self::Engine, Self::Error>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::game) enum StackReadError {
    UnknownItem,
    Empty,
    InvalidComponents(mod_api::StackError),
}

impl StackReadError {
    pub(in crate::game) fn into_write_error(self) -> StackWriteError {
        match self {
            Self::UnknownItem => StackWriteError::UnknownItem,
            Self::Empty => StackWriteError::EmptyEngineStack,
            Self::InvalidComponents(reason) => StackWriteError::InvalidComponents { reason },
        }
    }
}

/// Execute a native write and preserve its answer, including unit and original errors.
pub(in crate::game) fn call_write<W: IntoWriteResult>(
    call: impl FnOnce() -> W,
) -> Result<W::Value, W::Error> {
    call().into_write_result()
}

/// Convert a successful stack-bearing write answer through its original error channel.
pub(in crate::game) fn call_stack_write<W>(
    call: impl FnOnce() -> W,
) -> Result<<W::Value as TryIntoApi>::Api, W::Error>
where
    W: IntoWriteResult,
    W::Value: TryIntoApi<Error = StackReadError>,
    W::Error: From<StackWriteError>,
{
    call_write(call)?
        .try_into_api()
        .map_err(|error| error.into_write_error().into())
}

macro_rules! scalar_values {
    ($($ty:ty),* $(,)?) => { $(
        impl TryIntoApi for $ty {
            type Api = Self;
            type Error = StackReadError;
            fn try_into_api(&self) -> Result<Self, Self::Error> { Ok(*self) }
        }
        impl TryIntoEngine for $ty {
            type Engine = Self;
            type Error = StackWriteError;
            fn try_into_engine(&self, _: &Items) -> Result<Self, Self::Error> { Ok(*self) }
        }
    )* };
}
scalar_values!(u32, i32, bool, ());

impl TryIntoApi for String {
    type Api = Self;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self, Self::Error> {
        Ok(self.clone())
    }
}

impl<T: TryIntoApi + ?Sized> TryIntoApi for &T {
    type Api = T::Api;
    type Error = T::Error;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        (*self).try_into_api()
    }
}
impl<T: TryIntoEngine + ?Sized> TryIntoEngine for &T {
    type Engine = T::Engine;
    type Error = T::Error;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        (*self).try_into_engine(items)
    }
}

impl<T: TryIntoApi> TryIntoApi for Option<T> {
    type Api = Option<T::Api>;
    type Error = T::Error;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        self.as_ref().map(TryIntoApi::try_into_api).transpose()
    }
}
impl<T: TryIntoEngine> TryIntoEngine for Option<T> {
    type Engine = Option<T::Engine>;
    type Error = T::Error;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        self.as_ref()
            .map(|value| value.try_into_engine(items))
            .transpose()
    }
}
impl<T: TryIntoApi, E: Clone> TryIntoApi for Result<T, E> {
    type Api = Result<T::Api, E>;
    type Error = T::Error;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        match self {
            Ok(value) => value.try_into_api().map(Ok),
            Err(error) => Ok(Err(error.clone())),
        }
    }
}

impl<T: TryIntoApi, const N: usize> TryIntoApi for [T; N] {
    type Api = [T::Api; N];
    type Error = T::Error;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        let mut output: [Option<T::Api>; N] = std::array::from_fn(|_| None);
        for (target, source) in output.iter_mut().zip(self) {
            *target = Some(source.try_into_api()?);
        }
        Ok(output.map(|value| value.expect("every array element converted")))
    }
}
impl<T: TryIntoEngine, const N: usize> TryIntoEngine for [T; N] {
    type Engine = [T::Engine; N];
    type Error = T::Error;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        let mut output: [Option<T::Engine>; N] = std::array::from_fn(|_| None);
        for (target, source) in output.iter_mut().zip(self) {
            *target = Some(source.try_into_engine(items)?);
        }
        Ok(output.map(|value| value.expect("every array element converted")))
    }
}
impl<T: TryIntoApi> TryIntoApi for [T] {
    type Api = Vec<T::Api>;
    type Error = T::Error;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        self.iter().map(TryIntoApi::try_into_api).collect()
    }
}
impl<T: TryIntoApi> TryIntoApi for std::sync::Arc<[T]> {
    type Api = std::sync::Arc<[T::Api]>;
    type Error = T::Error;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(self.as_ref().try_into_api()?.into())
    }
}
impl<T: TryIntoApi> TryIntoApi for Vec<T> {
    type Api = Vec<T::Api>;
    type Error = T::Error;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        self.iter().map(TryIntoApi::try_into_api).collect()
    }
}
impl<T: TryIntoEngine> TryIntoEngine for Vec<T> {
    type Engine = Vec<T::Engine>;
    type Error = T::Error;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        self.as_slice().try_into_engine(items)
    }
}
impl<T: TryIntoEngine> TryIntoEngine for [T] {
    type Engine = Vec<T::Engine>;
    type Error = T::Error;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        self.iter()
            .map(|value| value.try_into_engine(items))
            .collect()
    }
}
impl<A: TryIntoApi, B: TryIntoApi<Error = A::Error>> TryIntoApi for (A, B) {
    type Api = (A::Api, B::Api);
    type Error = A::Error;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok((self.0.try_into_api()?, self.1.try_into_api()?))
    }
}
impl<A: TryIntoEngine, B: TryIntoEngine<Error = A::Error>> TryIntoEngine for (A, B) {
    type Engine = (A::Engine, B::Engine);
    type Error = A::Error;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        Ok((
            self.0.try_into_engine(items)?,
            self.1.try_into_engine(items)?,
        ))
    }
}

impl TryIntoApi for super::spill::BlockItemDrop {
    type Api = mod_api::NativeItemDrop;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::NativeItemDrop::new(
            self.stack.try_into_api()?,
            self.position,
            self.velocity,
            self.pickup_delay,
        ))
    }
}
impl TryIntoEngine for mod_api::NativeItemDrop {
    type Engine = super::spill::BlockItemDrop;
    type Error = StackWriteError;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        Ok(Self::Engine {
            stack: self.stack.try_into_engine(items)?,
            position: self.position,
            velocity: self.velocity,
            pickup_delay: self.pickup_delay,
        })
    }
}

impl TryIntoEngine for mod_api::NativeItemDisplayContext {
    type Engine = horizon_assets::item_model::DisplayContext;
    type Error = ();
    fn try_into_engine(&self, _: &Items) -> Result<Self::Engine, Self::Error> {
        use horizon_assets::item_model::DisplayContext as Engine;
        use mod_api::NativeItemDisplayContext as Api;
        Ok(match self {
            Api::None => Engine::None,
            Api::ThirdPersonLeftHand => Engine::ThirdPersonLeftHand,
            Api::ThirdPersonRightHand => Engine::ThirdPersonRightHand,
            Api::FirstPersonLeftHand => Engine::FirstPersonLeftHand,
            Api::FirstPersonRightHand => Engine::FirstPersonRightHand,
            Api::Head => Engine::Head,
            Api::Gui => Engine::Gui,
            Api::Ground => Engine::Ground,
            Api::Fixed => Engine::Fixed,
            Api::OnShelf => Engine::OnShelf,
            _ => return Err(()),
        })
    }
}

impl TryIntoApi for horizon_client::merchant::ItemCost {
    type Api = mod_api::ItemCost;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::ItemCost::new(self.stack.try_into_api()?))
    }
}
impl TryIntoEngine for mod_api::ItemCost {
    type Engine = horizon_client::merchant::ItemCost;
    type Error = StackWriteError;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        Ok(Self::Engine {
            stack: self.stack.try_into_engine(items)?,
        })
    }
}
impl TryIntoApi for horizon_client::merchant::MerchantOffer {
    type Api = mod_api::MerchantOffer;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        let mut offer = mod_api::MerchantOffer::new(
            self.base_cost_a.try_into_api()?,
            self.cost_b.try_into_api()?,
            self.result.try_into_api()?,
            self.max_uses,
            self.xp,
            self.price_multiplier,
        );
        offer.uses = self.uses;
        offer.reward_exp = self.reward_exp;
        offer.special_price_diff = self.special_price_diff;
        offer.demand = self.demand;
        Ok(offer)
    }
}
impl TryIntoEngine for mod_api::MerchantOffer {
    type Engine = horizon_client::merchant::MerchantOffer;
    type Error = StackWriteError;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        Ok(Self::Engine {
            base_cost_a: self.base_cost_a.try_into_engine(items)?,
            cost_b: self.cost_b.try_into_engine(items)?,
            result: self.result.try_into_engine(items)?,
            uses: self.uses,
            max_uses: self.max_uses,
            reward_exp: self.reward_exp,
            special_price_diff: self.special_price_diff,
            demand: self.demand,
            price_multiplier: self.price_multiplier,
            xp: self.xp,
        })
    }
}

impl TryIntoApi for EngineStack {
    type Api = ApiStack;
    type Error = StackReadError;

    fn try_into_api(&self) -> Result<ApiStack, Self::Error> {
        let key = mod_api::__private::item_key(&self.item).ok_or(StackReadError::UnknownItem)?;
        let mut result = ApiStack::new(key, self.count).ok_or(StackReadError::Empty)?;
        if self.carries_component(None, "minecraft:damage") {
            result.set_damage(self.damage);
        }
        if let Some(Tag::Compound(patch)) = horizon_items::item_data::encode_extra_components(self)
        {
            result
                .set_components(patch)
                .map_err(StackReadError::InvalidComponents)?;
        }
        Ok(result)
    }
}

impl TryIntoApi for horizon_assets::item_model::StackView {
    type Api = ApiStack;
    type Error = StackReadError;

    fn try_into_api(&self) -> Result<ApiStack, StackReadError> {
        let count = u32::try_from(self.count)
            .ok()
            .filter(|&count| count > 0)
            .ok_or(StackReadError::Empty)?;
        let components = Tag::compound(self.patch.iter().map(|(name, value)| match value {
            Some(value) => (
                horizon_core::nbt::JavaString::from(name.as_str()),
                horizon_client_core::scoreboard::json_to_tag(value),
            ),
            None => (
                horizon_core::nbt::JavaString::from(format!("!{name}").as_str()),
                Tag::Compound(Compound::new()),
            ),
        }));
        let key = mod_api::__private::item_key(&self.item).ok_or(StackReadError::UnknownItem)?;
        let mut stack = ApiStack::new(key, count).ok_or(StackReadError::Empty)?;
        let Tag::Compound(patch) = components else {
            unreachable!("model patch is constructed as a compound")
        };
        stack
            .set_components(patch)
            .map_err(StackReadError::InvalidComponents)?;
        Ok(stack)
    }
}

impl TryIntoEngine for mod_api::ItemResource {
    type Engine = EngineStack;
    type Error = StackWriteError;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        self.as_stack().try_into_engine(items)
    }
}

/// Encode an API value for a read-only engine predicate, without imposing resident maxima.
/// The write conversion below adds authoritative registry-dependent validation.
pub(in crate::game) fn stack_value(value: &ApiStack) -> Result<EngineStack, StackWriteError> {
    let id = mod_api::__private::item_key_id(value.item()).ok_or(StackWriteError::UnknownItem)?;
    Ok(encode_stack(value, id.into()))
}

fn encode_stack(value: &ApiStack, item: std::sync::Arc<str>) -> EngineStack {
    let mut patch = value.components().map(|patch| patch.as_nbt().clone());
    // Let the existing component codec preserve explicit zero versus prototype inheritance.
    if value.damage_component() == Some(0) {
        patch
            .get_or_insert_with(Default::default)
            .insert(JavaString::from("minecraft:damage"), Tag::Int(0));
    }
    let patch = patch.map(Tag::Compound);
    let (_, extra) = horizon_items::item_data::decode_components(patch.as_ref());
    EngineStack {
        item,
        count: value.count(),
        damage: value.damage(),
        extra,
    }
}

impl TryIntoEngine for ApiStack {
    type Engine = EngineStack;
    type Error = StackWriteError;

    fn try_into_engine(&self, items: &Items) -> Result<EngineStack, Self::Error> {
        let id =
            mod_api::__private::item_key_id(self.item()).ok_or(StackWriteError::UnknownItem)?;
        let (id, definition) = items
            .items
            .get_key_value(id)
            .ok_or(StackWriteError::UnknownItem)?;
        let stack = encode_stack(self, id.clone());
        let effective = |name: &str, default: u32| -> Result<u32, StackWriteError> {
            match stack.data().and_then(|data| data.other(name)) {
                Some(Tag::Int(value)) => {
                    u32::try_from(*value).map_err(|_| StackWriteError::InvalidMaximum)
                }
                Some(_) => Err(StackWriteError::InvalidMaximum),
                None => Ok(default),
            }
        };
        let maximum = if stack.carries_component(Some(definition), "minecraft:max_stack_size") {
            effective("minecraft:max_stack_size", definition.max_stack)?
        } else {
            1
        };
        if stack.count > maximum {
            return Err(StackWriteError::CountExceeded { maximum });
        }
        let maximum = if stack.carries_component(Some(definition), "minecraft:max_damage") {
            effective("minecraft:max_damage", definition.max_damage)?
        } else {
            0
        };
        let damage = self.damage_component().map_or_else(
            || {
                u32::try_from(stack.effective_damage(Some(definition))).map_err(|_| {
                    StackWriteError::InvalidComponents {
                        reason: mod_api::StackError::InvalidDamage,
                    }
                })
            },
            Ok,
        )?;
        if damage > maximum {
            return Err(StackWriteError::DamageExceeded { maximum });
        }
        if i32::try_from(damage).is_err() {
            return Err(StackWriteError::InvalidComponents {
                reason: mod_api::StackError::InvalidDamage,
            });
        }
        Ok(stack)
    }
}

impl TryIntoApi for crate::crafting::RecipeRule {
    type Api = mod_api::RecipeRule;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(match self {
            Self::Stonecutting { input, result } => mod_api::RecipeRule::Stonecutting {
                input: input.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::SmithingTransform {
                template,
                base,
                addition,
                result,
            } => mod_api::RecipeRule::SmithingTransform {
                template: template.try_into_api()?,
                base: base.try_into_api()?,
                addition: addition.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::SmithingTrim {
                template,
                base,
                addition,
                pattern,
            } => mod_api::RecipeRule::SmithingTrim {
                template: template.try_into_api()?,
                base: base.try_into_api()?,
                addition: addition.try_into_api()?,
                pattern: pattern.try_into_api()?,
            },
            Self::BannerDuplicate { banner, result } => mod_api::RecipeRule::BannerDuplicate {
                banner: banner.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::ShieldDecoration {
                banner,
                target,
                result,
            } => mod_api::RecipeRule::ShieldDecoration {
                banner: banner.try_into_api()?,
                target: target.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::Transmute {
                input,
                material,
                min,
                max,
                add_count,
                result,
            } => mod_api::RecipeRule::Transmute {
                input: input.try_into_api()?,
                material: material.try_into_api()?,
                min: *min,
                max: *max,
                add_count: *add_count,
                result: result.try_into_api()?,
            },
            Self::Dye {
                target,
                dye,
                result,
            } => mod_api::RecipeRule::Dye {
                target: target.try_into_api()?,
                dye: dye.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::Imbue {
                source,
                material,
                result,
            } => mod_api::RecipeRule::Imbue {
                source: source.try_into_api()?,
                material: material.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::BookCloning {
                source,
                material,
                min,
                max,
                result,
            } => mod_api::RecipeRule::BookCloning {
                source: source.try_into_api()?,
                material: material.try_into_api()?,
                min: *min,
                max: *max,
                result: result.try_into_api()?,
            },
            Self::FireworkRocket {
                shell,
                fuel,
                star,
                result,
            } => mod_api::RecipeRule::FireworkRocket {
                shell: shell.try_into_api()?,
                fuel: fuel.try_into_api()?,
                star: star.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::FireworkStar {
                dye,
                fuel,
                trail,
                twinkle,
                shapes,
                result,
            } => mod_api::RecipeRule::FireworkStar {
                dye: dye.try_into_api()?,
                fuel: fuel.try_into_api()?,
                trail: trail.try_into_api()?,
                twinkle: twinkle.try_into_api()?,
                shapes: shapes.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::FireworkStarFade {
                dye,
                target,
                result,
            } => mod_api::RecipeRule::FireworkStarFade {
                dye: dye.try_into_api()?,
                target: target.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::MapExtending {
                map,
                material,
                result,
            } => mod_api::RecipeRule::MapExtending {
                map: map.try_into_api()?,
                material: material.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::DecoratedPot {
                back,
                left,
                right,
                front,
                result,
            } => mod_api::RecipeRule::DecoratedPot {
                back: back.try_into_api()?,
                left: left.try_into_api()?,
                right: right.try_into_api()?,
                front: front.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::RepairItem => mod_api::RecipeRule::RepairItem,
        })
    }
}

impl TryIntoApi for crate::crafting::Ingredient {
    type Api = mod_api::Ingredient;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::Ingredient {
            items: self.items.clone(),
            order: self.order.clone(),
            custom: self.custom.clone(),
            children: self
                .children
                .as_ref()
                .map(|(shape, kids)| Ok((shape.clone(), kids.try_into_api()?)))
                .transpose()?,
        })
    }
}
impl TryIntoEngine for mod_api::Ingredient {
    type Engine = crate::crafting::Ingredient;
    type Error = StackWriteError;
    fn try_into_engine(&self, _items: &Items) -> Result<Self::Engine, Self::Error> {
        Ok(crate::crafting::Ingredient {
            items: self.items.clone(),
            order: self.order.clone(),
            custom: self.custom.clone(),
            children: self
                .children
                .as_ref()
                .map(|(shape, kids)| {
                    Ok((
                        shape.clone(),
                        kids.iter()
                            .map(|k| k.try_into_engine(_items))
                            .collect::<Result<_, _>>()?,
                    ))
                })
                .transpose()?,
        })
    }
}
impl TryIntoApi for crate::crafting::RecipeTemplate {
    type Api = mod_api::RecipeTemplate;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::RecipeTemplate::new(
            self.item.clone(),
            self.count,
            self.components.clone(),
        ))
    }
}
impl TryIntoApi for crate::crafting::RecipeTrimPattern {
    type Api = mod_api::RecipeTrimPattern;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(match self {
            Self::Reference(id) => mod_api::RecipeTrimPattern::Reference(id.clone()),
            Self::Inline {
                asset_id,
                description,
                decal,
            } => mod_api::RecipeTrimPattern::Inline {
                asset_id: asset_id.clone(),
                description: description.clone(),
                decal: *decal,
            },
        })
    }
}
impl TryIntoApi for crate::crafting::FireworkShapeIngredient {
    type Api = mod_api::FireworkShapeIngredient;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::FireworkShapeIngredient::new(
            self.shape,
            self.ingredient.try_into_api()?,
        ))
    }
}
impl TryIntoApi for crate::crafting::RecipeSnapshot {
    type Api = mod_api::Recipe;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        let mut result =
            mod_api::Recipe::new(self.id.clone(), self.kind.clone(), self.json.clone());
        result.group = self.group.clone();
        result.ingredients = self.ingredients.try_into_api()?;
        result.results = self.results.try_into_api()?;
        result.cooking_time = self.cooking_time;
        result.experience = self.experience;
        result.rule = self.rule.try_into_api()?;
        Ok(result)
    }
}

/// Install the engine-facing callbacks; snapshots never enter engine storage.
pub(crate) fn install_stack_hooks() {
    crate::native_stack_hooks::install(crate::native_stack_hooks::StackHooks {
        ingredient_test: |context, kind, data, stack, accept| {
            if !mod_api::__private::listened(mod_api::HookId::IngredientTest) {
                return;
            }
            if let Ok(stack) = stack.try_into_api() {
                mod_api::crafting::native_ingredient_test(context, kind, data, &stack, accept);
            }
        },
        enchantments: |stack, target, value| {
            if !mod_api::__private::listened(mod_api::HookId::GetEnchantmentLevel) {
                return;
            }
            if let Ok(stack) = stack.try_into_api() {
                mod_api::enchantments::native_get_enchantment_level(&stack, target, value);
            }
        },
        attributes: |stack, value| {
            if !mod_api::__private::listened(mod_api::HookId::ItemAttributeModifier) {
                return;
            }
            if let Ok(stack) = stack.try_into_api() {
                mod_api::item_attributes::native_item_attribute_modifier(&stack, value);
            }
        },
    });
}

struct ApiSlot<'a>(&'a mut dyn crate::menu::SlotView);
impl mod_api::menu::SlotView for ApiSlot<'_> {
    fn item(&self) -> Option<ApiStack> {
        self.0.item().and_then(|s| s.try_into_api().ok())
    }
    fn may_place(&self, stack: &ApiStack) -> bool {
        stack
            .try_into_engine(self.0.items())
            .is_ok_and(|s| self.0.may_place(&s))
    }
    fn may_pickup(&self) -> bool {
        self.0.may_pickup()
    }
    fn allow_modification(&self) -> bool {
        self.0.allow_modification()
    }
    fn safe_insert(&mut self, stack: Option<ApiStack>, count: u32) -> Option<ApiStack> {
        let value = match stack
            .as_ref()
            .map(|s| s.try_into_engine(self.0.items()))
            .transpose()
        {
            Ok(s) => s,
            Err(_) => return stack,
        };
        self.0
            .safe_insert(value, count)
            .and_then(|s| s.try_into_api().ok())
    }
    fn safe_take(&mut self, count: u32, max: u32) -> Option<ApiStack> {
        self.0
            .safe_take(count, max)
            .and_then(|s| s.try_into_api().ok())
    }
}
/// Adapt a synchronous slot event; engine slot rules still own every write.
pub(crate) fn menu_slot_rules(
    context: &horizon_world::native_load_context::LoopContext,
    menu: &str,
    slot: usize,
    stack: &EngineStack,
    may_place: &mut bool,
    max_stack: &mut u32,
) {
    if !mod_api::__private::has_part_hook(mod_api::HookId::MenuSlotRules) {
        return;
    }
    let Ok(stack) = stack.try_into_api() else {
        return;
    };
    let context = mod_api::OwningLoop::new(context);
    mod_api::dispatch_menu_slot_rules(&context, menu, slot, &stack, may_place, max_stack);
}

pub(crate) fn stacked_on_other(
    context: &horizon_world::native_load_context::LoopContext,
    event: &mut crate::menu::StackedOnOther<'_>,
    canceled: &mut bool,
) {
    if !mod_api::__private::listened(mod_api::HookId::ItemStackedOnOther) {
        return;
    }
    let Ok(mut carried) = event.carried.0.try_into_api() else {
        return;
    };
    let mut slot = ApiSlot(event.clicked);
    let mut api = mod_api::menu::StackedOnOther {
        player: event.player,
        player_entity_id: event.player_entity_id,
        slot: event.slot,
        container: event.container,
        container_slot: event.container_slot,
        primary: event.primary,
        carried: mod_api::menu::CarriedAccess(&mut carried),
        clicked: &mut slot,
        cancellation_result: event.cancellation_result,
    };
    mod_api::dispatch_item_stacked_on_other(&mod_api::OwningLoop::new(context), &mut api, canceled);
    event.cancellation_result = api.cancellation_result;
    if let Ok(value) = carried
        .as_ref()
        .map(|s| s.try_into_engine(slot.0.items()))
        .transpose()
    {
        *event.carried.0 = value;
    }
}

impl TryIntoApi for crate::stations::AnvilResult {
    type Api = mod_api::stations::AnvilResult;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::stations::AnvilResult {
            result: self.result.try_into_api()?,
            cost: self.cost,
            repair_item_count_cost: self.repair_item_count_cost,
            only_renaming: self.only_renaming,
        })
    }
}
impl TryIntoEngine for mod_api::stations::AnvilResult {
    type Engine = crate::stations::AnvilResult;
    type Error = StackWriteError;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        Ok(crate::stations::AnvilResult {
            result: self.result.try_into_engine(items)?,
            cost: self.cost,
            repair_item_count_cost: self.repair_item_count_cost,
            only_renaming: self.only_renaming,
        })
    }
}
impl TryIntoApi for crate::menu::AnvilCraft {
    type Api = mod_api::menu::AnvilCraft;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::menu::AnvilCraft {
            left: self.left.try_into_api()?,
            right: self.right.try_into_api()?,
            output: self.output.try_into_api()?,
        })
    }
}
impl TryIntoEngine for mod_api::menu::AnvilCraft {
    type Engine = crate::menu::AnvilCraft;
    type Error = StackWriteError;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        Ok(crate::menu::AnvilCraft {
            left: self.left.try_into_engine(items)?,
            right: self.right.try_into_engine(items)?,
            output: self.output.try_into_engine(items)?,
        })
    }
}
impl TryIntoApi for crate::menu::GrindstonePlace {
    type Api = mod_api::menu::GrindstonePlace;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::menu::GrindstonePlace {
            top: self.top.try_into_api()?,
            bottom: self.bottom.try_into_api()?,
            vanilla_output: self.vanilla_output.try_into_api()?,
            output: self.output.try_into_api()?,
            xp: self.xp,
        })
    }
}
impl TryIntoEngine for mod_api::menu::GrindstonePlace {
    type Engine = crate::menu::GrindstonePlace;
    type Error = StackWriteError;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        Ok(crate::menu::GrindstonePlace {
            top: self.top.try_into_engine(items)?,
            bottom: self.bottom.try_into_engine(items)?,
            vanilla_output: self.vanilla_output.try_into_engine(items)?,
            output: self.output.try_into_engine(items)?,
            xp: self.xp,
        })
    }
}
impl TryIntoApi for crate::menu::GrindstoneTake {
    type Api = mod_api::menu::GrindstoneTake;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::menu::GrindstoneTake {
            top: self.top.try_into_api()?,
            bottom: self.bottom.try_into_api()?,
            xp: self.xp,
            new_top: self.new_top.try_into_api()?,
            new_bottom: self.new_bottom.try_into_api()?,
        })
    }
}
impl TryIntoEngine for mod_api::menu::GrindstoneTake {
    type Engine = crate::menu::GrindstoneTake;
    type Error = StackWriteError;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        Ok(crate::menu::GrindstoneTake {
            top: self.top.try_into_engine(items)?,
            bottom: self.bottom.try_into_engine(items)?,
            xp: self.xp,
            new_top: self.new_top.try_into_engine(items)?,
            new_bottom: self.new_bottom.try_into_engine(items)?,
        })
    }
}

impl TryIntoApi for crate::menu::AnvilUpdate {
    type Api = mod_api::menu::AnvilUpdate;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        let vanilla = mod_api::stations::AnvilResult {
            result: self.vanilla.output.try_into_api()?,
            cost: self.vanilla.xp_cost,
            repair_item_count_cost: self.vanilla.material_cost.max(0) as u32,
            only_renaming: false,
        };
        let mut result = mod_api::menu::AnvilUpdate::new(
            self.left.try_into_api()?,
            self.right.try_into_api()?,
            self.name.clone(),
            &vanilla,
        );
        result.output = self.output.try_into_api()?;
        result.xp_cost = self.xp_cost;
        result.material_cost = self.material_cost;
        Ok(result)
    }
}

impl TryIntoEngine for mod_api::menu::AnvilUpdate {
    type Engine = crate::menu::AnvilUpdate;
    type Error = StackWriteError;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        Ok(crate::menu::AnvilUpdate {
            left: self.left.try_into_engine(items)?,
            right: self.right.try_into_engine(items)?,
            name: self.name.clone(),
            vanilla: crate::menu::AnvilVanilla {
                output: self.vanilla.output.try_into_engine(items)?,
                xp_cost: self.vanilla.xp_cost,
                material_cost: self.vanilla.material_cost,
            },
            output: self.output.try_into_engine(items)?,
            xp_cost: self.xp_cost,
            material_cost: self.material_cost,
        })
    }
}
impl super::Game<'_> {
    pub(in crate::game) fn api_anvil_update(
        &mut self,
        player: u128,
        entity: i32,
        pos: [i32; 3],
        value: &mut crate::menu::AnvilUpdate,
        cancel: &mut bool,
    ) {
        let Ok(mut api) = value.try_into_api() else {
            return;
        };
        self.native_anvil_update(player, entity, pos, &mut api, cancel);
        match api.try_into_engine(self.items()) {
            Ok(result) => *value = result,
            Err(_) => *cancel = true,
        }
    }
    pub(in crate::game) fn api_grindstone_place(
        &mut self,
        player: u128,
        entity: i32,
        pos: [i32; 3],
        value: &mut crate::menu::GrindstonePlace,
        cancel: &mut bool,
    ) {
        let Ok(mut api) = value.try_into_api() else {
            return;
        };
        self.native_grindstone_place(player, entity, pos, &mut api, cancel);
        match api.try_into_engine(self.items()) {
            Ok(result) => *value = result,
            Err(_) => *cancel = true,
        }
    }
    pub(in crate::game) fn api_grindstone_take(
        &mut self,
        player: u128,
        entity: i32,
        pos: [i32; 3],
        value: &mut crate::menu::GrindstoneTake,
        cancel: &mut bool,
    ) {
        let Ok(mut api) = value.try_into_api() else {
            return;
        };
        self.native_grindstone_take(player, entity, pos, &mut api, cancel);
        match api.try_into_engine(self.items()) {
            Ok(result) => *value = result,
            Err(_) => *cancel = true,
        }
    }
    pub(in crate::game) fn api_anvil_craft_pre(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        value: &crate::menu::AnvilCraft,
        cancel: &mut bool,
    ) {
        if let Ok(api) = value.try_into_api() {
            self.native_anvil_craft_pre(player, name, pos, &api, cancel);
        }
    }
    pub(in crate::game) fn api_anvil_craft_post(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        value: &crate::menu::AnvilCraft,
    ) {
        if let Ok(api) = value.try_into_api() {
            self.native_anvil_craft_post(player, name, pos, &api);
        }
    }
}

#[cfg(test)]
mod attribute_boundary_tests {
    use super::EngineStack as ItemStack;
    use crate::item_attributes::attribute_modifiers_or;
    use horizon_items::item_data::ModifierRow;
    use mod_api::{
        AttributeModifier, AttributeOperation, EquipmentSlotGroup, Hooks, Identifier,
        ItemAttributeModifierEvent, ModEntry, testing::with_mods,
    };
    use std::borrow::Cow;
    /// Makes a netherite chestplate heavier: +0.4 knockback resistance in
    /// the chest slot, and no armor toughness.
    struct Heavy;
    #[mod_api::__subscriptions]
    impl Hooks for Heavy {
        fn item_attribute_modifier(&self, event: &mut ItemAttributeModifierEvent<'_>) {
            if mod_api::__private::item_key_id(event.stack.item())
                != Some("minecraft:netherite_chestplate")
            {
                return;
            }
            let id = |text: &str| Identifier::parse(text).unwrap();
            let heavy = |amount| {
                AttributeModifier::new(id("test:heavy"), amount, AttributeOperation::AddValue)
            };
            let resistance = id("minecraft:knockback_resistance");
            assert!(
                event
                    .modifiers
                    .add(resistance.clone(), heavy(0.4), EquipmentSlotGroup::Chest)
            );
            // The id is taken on that attribute.
            assert!(
                !event
                    .modifiers
                    .add(resistance, heavy(1.0), EquipmentSlotGroup::Chest)
            );
            assert!(
                event
                    .modifiers
                    .remove_all_for(&id("minecraft:armor_toughness"))
            );
        }
    }

    fn chestplate() -> Vec<ModifierRow> {
        ["minecraft:armor", "minecraft:armor_toughness"]
            .map(|attribute| {
                ModifierRow::new(
                    attribute,
                    "minecraft:armor.chestplate",
                    3.0,
                    "add_value",
                    "chest",
                )
            })
            .to_vec()
    }

    /// A mod's edits are what the stack grants; a stack it leaves alone, and
    /// any stack with no mod listening, keeps the item's list borrowed.
    #[test]
    fn a_mod_edits_what_a_stack_grants() {
        super::prepare_stack_boundary_tests();
        let types = chestplate();
        let netherite = ItemStack::new("minecraft:netherite_chestplate", 1);
        let diamond = ItemStack::new("minecraft:diamond_chestplate", 1);
        with_mods(Vec::new().into(), || {
            assert!(matches!(
                attribute_modifiers_or(&netherite, &types),
                Cow::Borrowed(_)
            ));
        });
        with_mods(vec![ModEntry::new("heavy", Box::new(Heavy))].into(), || {
            assert_eq!(
                &*attribute_modifiers_or(&netherite, &types),
                [
                    types[0].clone(),
                    ModifierRow::new(
                        "minecraft:knockback_resistance",
                        "test:heavy",
                        0.4,
                        "add_value",
                        "chest"
                    ),
                ]
            );
            assert!(matches!(
                attribute_modifiers_or(&diamond, &types),
                Cow::Borrowed(_)
            ));
        });
    }
}

#[cfg(test)]
mod enchantment_boundary_tests {
    use super::EngineStack as ItemStack;
    use crate::enchantment_levels::get_enchantment_level;
    use mod_api::{GetEnchantmentLevelEvent, Hooks, Identifier, ModEntry, testing::with_mods};
    use std::borrow::Cow;
    use std::sync::Arc;
    fn id(text: &str) -> Identifier {
        Identifier::parse(text).unwrap()
    }
    /// An iron sword fights as if it had Looting II and no Sharpness.
    struct Keen;
    #[mod_api::__subscriptions]
    impl Hooks for Keen {
        fn get_enchantment_level(&self, event: &mut GetEnchantmentLevelEvent<'_>) {
            if mod_api::__private::item_key_id(event.stack.item()) != Some("minecraft:iron_sword") {
                return;
            }
            let looting = id("minecraft:looting");
            if event.target.is_none_or(|target| *target == looting) {
                event.enchantments.upgrade(looting, 2);
            }
            event.enchantments.set(id("minecraft:sharpness"), 0);
        }
    }

    /// Gameplay reads see the mod's levels; the stack's own component, a
    /// stack the mod leaves alone and every stack with no mod listening
    /// keep theirs, borrowed.
    #[test]
    fn a_mod_changes_the_levels_gameplay_reads() {
        super::prepare_stack_boundary_tests();
        let _ = horizon_items::native_enchantment_hooks::install(get_enchantment_level);
        let mut iron = ItemStack::new("minecraft:iron_sword", 1);
        iron.enchant("minecraft:sharpness", 3);
        let mut diamond = ItemStack::new("minecraft:diamond_sword", 1);
        diamond.enchant("minecraft:sharpness", 3);
        with_mods(Vec::new().into(), || {
            assert!(matches!(iron.all_enchantments(), Cow::Borrowed(_)));
            assert_eq!(iron.enchantment_level("minecraft:sharpness"), 3);
        });
        with_mods(vec![ModEntry::new("keen", Box::new(Keen))].into(), || {
            assert_eq!(
                &*iron.all_enchantments(),
                [(Arc::from("minecraft:looting"), 2)]
            );
            assert_eq!(iron.enchantment_level("minecraft:looting"), 2);
            assert_eq!(iron.enchantment_level("minecraft:sharpness"), 0);
            assert_eq!(iron.tag_enchantment_level("minecraft:sharpness"), 3);
            assert_eq!(iron.enchantments(), [(Arc::from("minecraft:sharpness"), 3)]);
            assert!(matches!(diamond.all_enchantments(), Cow::Borrowed(_)));
            assert_eq!(diamond.enchantment_level("minecraft:sharpness"), 3);
        });
    }
}

#[cfg(test)]
fn prepare_stack_boundary_tests() {
    install_stack_hooks();
    install_parts(&[]).unwrap();
    mod_api::__private::resolve_items([
        "minecraft:iron_sword",
        "minecraft:stick",
        "minecraft:diamond_sword",
        "minecraft:netherite_chestplate",
        "minecraft:diamond_chestplate",
        "minecraft:potion",
    ])
    .unwrap();
}

#[cfg(test)]
mod brewed_boundary_tests {
    use super::*;
    struct NoHost;
    impl mod_api::Host for NoHost {}
    mod_api::impl_host_areas!(NoHost);
    impl mod_api::ServerHost for NoHost {}
    mod_api::impl_server_host_areas!(NoHost);
    struct Watch(std::sync::Arc<std::sync::Mutex<Vec<(u128, i32, String, u32)>>>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Watch {
        fn player_brewed_potion(
            &self,
            _: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::PlayerBrewedPotionEvent<'_>,
        ) {
            self.0.lock().unwrap().push((
                event.player,
                event.player_entity_id,
                mod_api::__private::item_key_id(event.stack.item())
                    .unwrap()
                    .to_owned(),
                event.stack.count(),
            ));
        }
    }
    #[test]
    fn brewed_event_reports_player_and_converted_stack() {
        prepare_stack_boundary_tests();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        mod_api::testing::with_mods(
            vec![mod_api::ModEntry::new(
                "watch",
                Box::new(Watch(seen.clone())),
            )]
            .into(),
            || {
                let stack = EngineStack::new("minecraft:potion", 1)
                    .try_into_api()
                    .unwrap();
                mod_api::dispatch_player_brewed_potion(&mut NoHost, 7, 41, &stack);
            },
        );
        assert_eq!(
            *seen.lock().unwrap(),
            [(7, 41, "minecraft:potion".to_owned(), 1)]
        );
    }
}

impl TryIntoApi for crate::containers::ModBlockEntityFields {
    type Api = mod_api::containers::ModBlockEntityFields;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::containers::ModBlockEntityFields {
            nbt: self.nbt.clone(),
            items: self.items.try_into_api()?,
            data: self.data.clone(),
        })
    }
}
impl TryIntoEngine for mod_api::containers::ModBlockEntityFields {
    type Engine = crate::containers::ModBlockEntityFields;
    type Error = StackWriteError;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        Ok(crate::containers::ModBlockEntityFields {
            nbt: self.nbt.clone(),
            items: self.items.try_into_engine(items)?,
            data: self.data.clone(),
        })
    }
}
impl TryIntoApi for crate::containers::ModBlockEntity {
    type Api = mod_api::ModBlockEntity;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::ModBlockEntity::from_snapshot(
            self.id().to_owned(),
            (**self).try_into_api()?,
            mod_api::containers::ModRevision::from_snapshot(
                self.revision().identity(),
                self.revision().changes(),
            ),
        ))
    }
}

#[derive(Debug, PartialEq, Eq)]
enum BlockEntitySnapshotError {
    InvalidRevision,
    InvalidStack,
}

fn apply_block_entity_snapshot(
    entity: &mut crate::containers::ModBlockEntity,
    snapshot: &mod_api::ModBlockEntity,
    before: mod_api::containers::ModRevision,
    items: &Items,
) -> Result<(), BlockEntitySnapshotError> {
    let current = entity.revision();
    let revision = snapshot.revision();
    if snapshot.id() != entity.id()
        || revision.identity() != current.identity()
        || revision.changes() < current.changes()
    {
        return Err(BlockEntitySnapshotError::InvalidRevision);
    }
    if revision != before {
        let fields = (**snapshot)
            .try_into_engine(items)
            .map_err(|_| BlockEntitySnapshotError::InvalidStack)?;
        entity.apply_snapshot(fields, snapshot.revision().changes());
    }
    Ok(())
}

impl super::Game<'_> {
    pub(in crate::game) fn api_block_entity_tick(
        &mut self,
        pos: [i32; 3],
        entity: &mut crate::containers::ModBlockEntity,
    ) {
        if !mod_api::__private::has_part_hook(mod_api::HookId::BlockEntityTick) {
            return;
        }
        let Ok(mut api) = entity.try_into_api() else {
            return;
        };
        let revision = api.revision();
        self.native_block_entity_tick(pos, &mut api);
        let _ = apply_block_entity_snapshot(entity, &api, revision, self.items());
    }
    pub(in crate::game) fn api_block_entity_update_tag(
        pos: [i32; 3],
        entity: &crate::containers::ModBlockEntity,
        tag: &mut Option<horizon_core::nbt::Compound>,
    ) {
        if !mod_api::__private::has_part_hook(mod_api::HookId::BlockEntityUpdateTag) {
            return;
        }
        if let Ok(api) = entity.try_into_api() {
            Self::native_block_entity_update_tag(pos, &api, tag);
        }
    }
    pub(in crate::game) fn api_block_entity_submit(
        &self,
        pos: [i32; 3],
        block: &str,
        state: horizon_world::registry::BlockStateId,
        entity: &crate::containers::ModBlockEntity,
        draw: &mut super::native_block_entity_draw::BlockEntityDraw,
    ) {
        if !mod_api::__private::has_part_hook(mod_api::HookId::BlockEntitySubmit) {
            return;
        }
        if let Ok(api) = entity.try_into_api() {
            self.native_block_entity_submit(pos, block, state, &api, draw);
        }
    }
    pub(in crate::game) fn api_block_entity_render_bounding_box(
        &self,
        pos: [i32; 3],
        block: &str,
        state: horizon_world::registry::BlockStateId,
        entity: &crate::containers::ModBlockEntity,
        bounds: &mut [f32; 6],
    ) {
        if !mod_api::__private::has_part_hook(mod_api::HookId::BlockEntityRenderBoundingBox) {
            return;
        }
        if let Ok(api) = entity.try_into_api() {
            self.native_block_entity_render_bounding_box(pos, block, state, &api, bounds);
        }
    }
    /// Shared click metadata borrows the already-converted API snapshot for this call.
    pub(in crate::game) fn block_click<'a>(
        &mut self,
        hit: &super::Hit,
        hand: mod_api::Hand,
        stack: Option<&'a ApiStack>,
    ) -> mod_api::NativeBlockClick<'a> {
        let (player, player_entity_id) = self.native_clicker();
        mod_api::NativeBlockClick::from(mod_api::__private::built::NativeBlockClick {
            side: if self.remote.is_some() {
                mod_api::LogicalSide::Client
            } else {
                mod_api::LogicalSide::Server
            },
            dimension: self.dim.level(),
            pos: hit.pos,
            hit: hit.native(),
            hand,
            stack,
            player,
            player_entity_id,
        })
    }

    pub(in crate::game) fn api_menu_button(
        &mut self,
        context: &mod_api::menu::MenuButtonContext,
        entity: &mut crate::containers::ModBlockEntity,
        carried: &mut Option<EngineStack>,
    ) {
        if !mod_api::__private::has_part_hook(mod_api::HookId::MenuButton) {
            return;
        }
        let (Ok(mut api), Ok(mut cursor)) = (entity.try_into_api(), carried.try_into_api()) else {
            return;
        };
        let revision = api.revision();
        self.native_menu_button(context, &mut api, &mut cursor);
        if let Ok(cursor) = cursor.try_into_engine(self.items())
            && apply_block_entity_snapshot(entity, &api, revision, self.items()).is_ok()
        {
            *carried = cursor;
        }
    }
    pub(in crate::game) fn api_menu_quick_move(
        &mut self,
        pos: [i32; 3],
        entity: &mut crate::containers::ModBlockEntity,
        container_slots: usize,
        slot: usize,
        stack: &mut EngineStack,
        handled: &mut bool,
    ) {
        if !mod_api::__private::has_part_hook(mod_api::HookId::MenuQuickMove) {
            return;
        }
        let (Ok(mut api), Ok(mut held)) = (entity.try_into_api(), stack.try_into_api()) else {
            return;
        };
        let revision = api.revision();
        self.native_menu_quick_move(pos, &mut api, container_slots, slot, &mut held, handled);
        if let Ok(held) = held.try_into_engine(self.items())
            && apply_block_entity_snapshot(entity, &api, revision, self.items()).is_ok()
        {
            *stack = held;
        } else {
            *handled = false;
        }
    }
}
impl super::GameLevel<'_> {
    pub(in crate::game) fn api_pre_remove_side_effects(
        &mut self,
        pos: horizon_core::position::BlockPos,
        state: horizon_world::registry::BlockStateId,
        entity: &crate::containers::ModBlockEntity,
        drops: &mut Vec<EngineStack>,
    ) {
        if !mod_api::__private::has_part_hook(mod_api::HookId::PreRemoveSideEffects) {
            return;
        }
        let Ok(api) = entity.try_into_api() else {
            return;
        };
        let mut added = Vec::new();
        self.native_pre_remove_side_effects(pos, state, &api, &mut added);
        drops.extend(added);
    }
}

#[cfg(test)]
mod block_entity_snapshot_tests {
    use super::*;
    #[test]
    fn unchanged_snapshots_keep_revision_and_callback_writes_advance_it() {
        let mut entity =
            crate::containers::ModBlockEntity::new("test:machine".into(), Default::default());
        let first = entity.try_into_api().unwrap();
        let mut second = entity.try_into_api().unwrap();
        assert_eq!(first.revision(), second.revision());
        let before = second.revision();
        second.data.push(7);
        second.data.push(8);
        assert_ne!(before, second.revision());
        apply_block_entity_snapshot(
            &mut entity,
            &second,
            before,
            &Items::for_tests(Default::default()),
        )
        .unwrap();
        let third = entity.try_into_api().unwrap();
        assert_ne!(first.revision(), third.revision());
        assert_eq!(second.revision(), third.revision());
        assert_eq!(third.revision(), entity.try_into_api().unwrap().revision());
        assert_eq!(entity.id(), "test:machine");
        assert_eq!(entity.data, [7, 8]);
        let other = mod_api::ModBlockEntity::new("test:machine".into(), Default::default());
        assert_ne!(other.revision(), third.revision());
        entity.data.push(9);
        let fourth = entity.try_into_api().unwrap();
        assert_ne!(fourth.revision(), second.revision());
        let items = Items::for_tests(Default::default());
        assert_eq!(
            apply_block_entity_snapshot(&mut entity, &second, before, &items),
            Err(BlockEntitySnapshotError::InvalidRevision)
        );
        assert_eq!(
            apply_block_entity_snapshot(&mut entity, &other, before, &items),
            Err(BlockEntitySnapshotError::InvalidRevision)
        );
        assert_eq!(entity.data, [7, 8, 9]);
    }
}

#[cfg(test)]
mod mutable_output_tests {
    use super::*;

    #[test]
    fn array_outputs_are_admitted_only_after_every_stack_passes_resident_limits() {
        prepare_stack_boundary_tests();
        let root =
            std::env::temp_dir().join(format!("horizon-api-array-output-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("items.json"),
            r#"{"minecraft:stick":{"minecraft:max_stack_size":64}}"#,
        )
        .unwrap();
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        let items =
            Items::catalog(&root, &horizon_world::loot::PackOverlay::NONE, &defaults).unwrap();
        std::fs::remove_dir_all(root).unwrap();
        let key = mod_api::__private::item_key("minecraft:stick").unwrap();
        let mut api = [
            ApiStack::new(key, 8).unwrap(),
            ApiStack::new(key, 65).unwrap(),
        ];
        let mut output = [
            EngineStack::new("minecraft:stick", 3),
            EngineStack::new("minecraft:stick", 4),
        ];
        let before = output.clone();
        let converted = api.try_into_engine(&items);
        assert_eq!(
            converted,
            Err(StackWriteError::CountExceeded { maximum: 64 })
        );
        if let Ok(validated) = converted {
            output = validated;
        }
        assert_eq!(
            output, before,
            "a legal first element must not be applied before a rejected second element"
        );

        api[1].set_count(64).unwrap();
        output = api.try_into_engine(&items).unwrap();
        assert_eq!(
            output,
            [
                EngineStack::new("minecraft:stick", 8),
                EngineStack::new("minecraft:stick", 64)
            ]
        );
    }
}

#[cfg(test)]
mod stack_limit_tests {
    use super::*;

    fn catalog() -> Items {
        prepare_stack_boundary_tests();
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "horizon-api-limits-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("items.json"), r#"{
            "minecraft:stick":{"minecraft:max_stack_size":64},
            "minecraft:iron_sword":{"minecraft:max_stack_size":1,"minecraft:max_damage":10,"minecraft:damage":7},
            "minecraft:diamond_sword":{"minecraft:max_stack_size":1,"minecraft:max_damage":100,"minecraft:damage":0}
        }"#).unwrap();
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        let items =
            Items::catalog(&root, &horizon_world::loot::PackOverlay::NONE, &defaults).unwrap();
        std::fs::remove_dir_all(root).unwrap();
        items
    }

    #[test]
    fn strict_damage_limits_include_zero_inheritance_removal_and_nbt_int_range() {
        let mut items = catalog();
        let key = |id| mod_api::__private::item_key(id).unwrap();
        let plain = ApiStack::of(key("minecraft:stick"));
        assert!(plain.try_into_engine(&items).is_ok());
        let mut explicit = plain.clone();
        explicit.set_damage(0);
        let engine = explicit.try_into_engine(&items).unwrap();
        assert!(engine.carries_component(None, "minecraft:damage"));
        assert_eq!(
            horizon_items::item_data::encode_components(&engine)
                .unwrap()
                .get("minecraft:damage"),
            Some(&Tag::Int(0))
        );
        explicit.set_damage(u32::MAX);
        assert_eq!(
            explicit.try_into_engine(&items),
            Err(StackWriteError::DamageExceeded { maximum: 0 })
        );

        let inherited = ApiStack::of(key("minecraft:iron_sword"));
        let definition = items.items.get("minecraft:iron_sword").unwrap();
        assert_eq!(
            inherited
                .try_into_engine(&items)
                .unwrap()
                .effective_damage(Some(definition)),
            7
        );
        let mut lower = inherited.clone();
        lower
            .set_components([(JavaString::from("minecraft:max_damage"), Tag::Int(5))].into())
            .unwrap();
        assert_eq!(
            lower.try_into_engine(&items),
            Err(StackWriteError::DamageExceeded { maximum: 5 })
        );
        let mut removed = inherited;
        removed
            .set_components(
                [(
                    JavaString::from("!minecraft:max_damage"),
                    Tag::Compound(Default::default()),
                )]
                .into(),
            )
            .unwrap();
        assert_eq!(
            removed.try_into_engine(&items),
            Err(StackWriteError::DamageExceeded { maximum: 0 })
        );
        removed.clear_damage();
        let engine = removed.try_into_engine(&items).unwrap();
        assert_eq!(engine.max_damage(Some(definition)), 0);
        assert_eq!(engine.effective_damage(Some(definition)), 0);
        removed.set_damage(0);
        assert!(removed.try_into_engine(&items).is_ok());

        items
            .items
            .get_mut("minecraft:diamond_sword")
            .unwrap()
            .max_damage = u32::MAX;
        let mut oversized = ApiStack::of(key("minecraft:diamond_sword"));
        oversized.set_damage(u32::MAX);
        assert_eq!(
            oversized.try_into_engine(&items),
            Err(StackWriteError::InvalidComponents {
                reason: mod_api::StackError::InvalidDamage
            })
        );
    }

    #[test]
    fn removed_stack_maximum_uses_the_absent_component_limit_of_one() {
        let items = catalog();
        let mut stack =
            ApiStack::new(mod_api::__private::item_key("minecraft:stick").unwrap(), 2).unwrap();
        stack
            .set_components(
                [(
                    JavaString::from("!minecraft:max_stack_size"),
                    Tag::Compound(Default::default()),
                )]
                .into(),
            )
            .unwrap();
        assert_eq!(
            stack.try_into_engine(&items),
            Err(StackWriteError::CountExceeded { maximum: 1 })
        );
        stack.set_count(1).unwrap();
        assert!(!stack.try_into_engine(&items).unwrap().carries_component(
            items.items.get("minecraft:stick"),
            "minecraft:max_stack_size"
        ));
    }

    #[test]
    fn damage_only_snapshots_preserve_presence_without_an_opaque_patch() {
        catalog();
        for damage in [0, 5] {
            let (damage, extra) = horizon_items::item_data::decode_components(Some(
                &Tag::compound([("minecraft:damage", Tag::Int(damage))]),
            ));
            let engine = EngineStack {
                damage,
                extra,
                ..EngineStack::new("minecraft:iron_sword", 1)
            };
            let api = engine.try_into_api().unwrap();
            assert_eq!(api.damage_component(), Some(damage));
            assert!(api.components().is_none());
        }
    }
}

#[cfg(test)]
mod item_model_snapshot_tests {
    use super::*;
    use horizon_assets::item_model::StackView;

    #[test]
    fn model_snapshots_keep_component_patches_without_runtime_bundle_selection() {
        prepare_stack_boundary_tests();
        let mut view = StackView::new("minecraft:stick", 2);
        view.bundle_selected = 3;
        view.patch
            .insert("minecraft:damage".into(), Some(serde_json::json!(7)));
        view.patch.insert(
            "example:charge".into(),
            Some(serde_json::json!({"amount":4})),
        );
        view.patch.insert("minecraft:custom_name".into(), None);
        let snapshot = view.try_into_api().unwrap();
        assert_eq!(
            snapshot.item(),
            mod_api::__private::item_key("minecraft:stick").unwrap()
        );
        assert_eq!(snapshot.count(), 2);
        assert_eq!(snapshot.damage(), 7);
        let patch = snapshot.components().unwrap().as_nbt();
        assert_eq!(
            patch
                .get(&JavaString::from("example:charge"))
                .unwrap()
                .get("amount"),
            Some(&Tag::Int(4))
        );
        assert!(
            patch
                .get(&JavaString::from("!minecraft:custom_name"))
                .is_some()
        );
        assert!(patch.get(&JavaString::from("bundle_selected")).is_none());
        assert_eq!(view.bundle_selected, 3);
    }

    #[test]
    fn empty_model_views_do_not_become_owned_stack_sentinels() {
        prepare_stack_boundary_tests();
        for count in [-1, 0] {
            assert_eq!(
                StackView::new("minecraft:stick", count).try_into_api(),
                Err(StackReadError::Empty)
            );
        }
        assert_eq!(
            StackView::new("example:unknown", 1).try_into_api(),
            Err(StackReadError::UnknownItem)
        );
    }
}

impl TryIntoApi for horizon_mod_package::Side {
    type Api = mod_api::Side;
    type Error = std::convert::Infallible;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(match self {
            Self::Client => mod_api::Side::Client,
            Self::Both => mod_api::Side::Both,
        })
    }
}

fn package_snapshot(
    manifest: &horizon_mod_package::Manifest,
    kind: mod_api::PackageKind,
) -> mod_api::PackageInfo {
    mod_api::PackageInfo::new(
        manifest.id.clone(),
        manifest.version.clone(),
        kind,
        manifest.side.try_into_api().unwrap(),
        std::iter::once(&manifest.id)
            .chain(&manifest.namespaces)
            .cloned()
            .collect(),
        manifest.dependencies.keys().cloned().collect(),
    )
}
impl TryIntoApi for horizon_mod_package::Manifest {
    type Api = mod_api::PackageInfo;
    type Error = std::convert::Infallible;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(package_snapshot(
            self,
            if self.wasm.is_some() {
                mod_api::PackageKind::Code
            } else {
                mod_api::PackageKind::Content
            },
        ))
    }
}
impl TryIntoApi for horizon_mod_package::packages::AdjacentManifest {
    type Api = mod_api::PackageInfo;
    type Error = std::convert::Infallible;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(package_snapshot(&self.manifest, mod_api::PackageKind::Code))
    }
}

pub(in crate::game) fn install_packages(
    native: &[horizon_mod_package::packages::AdjacentManifest],
    content: &[horizon_mod_package::selection::SelectedPackage],
) {
    let native: Vec<_> = native.iter().map(|p| p.try_into_api().unwrap()).collect();
    let content: Vec<_> = content
        .iter()
        .map(|p| p.package.manifest.try_into_api().unwrap())
        .collect();
    mod_api::__private::install_packages(&native, &content);
}

#[cfg(test)]
mod package_snapshot_tests {
    use super::*;
    #[test]
    fn package_metadata_keeps_declared_side_and_namespace_order() {
        for (side, expected) in [
            ("client", mod_api::Side::Client),
            ("both", mod_api::Side::Both),
        ] {
            let manifest = horizon_mod_package::Manifest::parse(&format!(
                "id='test'\nname='Test'\nversion='1'\napi='0.2'\nside='{side}'\ncapabilities=['registries']\n"
            )).unwrap();
            let snapshot = manifest.try_into_api().unwrap();
            assert_eq!(snapshot.side(), expected);
            assert_eq!(snapshot.namespaces().collect::<Vec<_>>(), ["test"]);
            assert_eq!(snapshot.kind(), mod_api::PackageKind::Content);
        }
    }
}

impl TryIntoApi for horizon_client_core::suggestion::Suggestion {
    type Api = mod_api::Suggestion;
    type Error = std::convert::Infallible;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        let mut value = mod_api::Suggestion::new(self.range, self.text.clone());
        value.tooltip.clone_from(&self.tooltip);
        value.int_value = self.int_value;
        Ok(value)
    }
}
impl TryIntoEngine for mod_api::Suggestion {
    type Engine = horizon_client_core::suggestion::Suggestion;
    type Error = StackWriteError;
    fn try_into_engine(&self, _: &Items) -> Result<Self::Engine, Self::Error> {
        Ok(Self::Engine {
            range: self.range,
            text: self.text.clone(),
            tooltip: self.tooltip.clone(),
            int_value: self.int_value,
        })
    }
}
#[cfg(test)]
mod completion_snapshot_tests {
    use super::*;
    #[test]
    fn completion_round_trip_keeps_range_tooltip_and_numeric_order() {
        let source = horizon_client_core::suggestion::Suggestion {
            range: (4, 7),
            text: "42".into(),
            tooltip: Some("count".into()),
            int_value: Some(42),
        };
        let mut api = source.try_into_api().unwrap();
        api.text = "43".into();
        api.int_value = Some(43);
        let engine = api
            .try_into_engine(&Items::for_tests(Default::default()))
            .unwrap();
        assert_eq!(engine.range, (4, 7));
        assert_eq!(engine.tooltip.as_deref(), Some("count"));
        assert_eq!(engine.int_value, Some(43));
        assert_eq!(engine.apply("foo 123 bar"), "foo 43 bar");
        assert_eq!(source.text, "42");
    }
}

// Registration declarations do not need a live item registry.
impl TryIntoEngine<()> for mod_api::BlockEntityTypeDecl {
    type Engine = horizon_mod_package::content::BlockEntityType;
    type Error = std::convert::Infallible;
    fn try_into_engine(&self, _: &()) -> Result<Self::Engine, Self::Error> {
        Ok(Self::Engine {
            id: self.id.clone(),
            blocks: self.blocks.clone(),
        })
    }
}
pub(in crate::game) fn block_entity_declarations()
-> Vec<horizon_mod_package::content::BlockEntityType> {
    mod_api::__private::block_entity_types()
        .iter()
        .map(|decl| decl.try_into_engine(&()).unwrap())
        .collect()
}

impl TryIntoEngine<()> for mod_api::GameRuleDecl {
    type Engine = horizon_client_core::gamerules::Rule;
    type Error = std::convert::Infallible;
    fn try_into_engine(&self, _: &()) -> Result<Self::Engine, Self::Error> {
        Ok(Self::Engine {
            name: self.name.clone(),
            category: self.category.clone(),
            default: horizon_client_core::gamerules::Value::Bool(self.default),
            min: 0,
            max: 0,
        })
    }
}

#[cfg(test)]
mod declaration_snapshot_tests {
    use super::*;
    #[test]
    fn declarations_keep_supported_blocks_and_rule_positions() {
        static KEY: std::sync::OnceLock<mod_api::BoolGameRuleKey> = std::sync::OnceLock::new();
        // Use registration, the owner of declaration validation.
        struct Mod;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Mod {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                r.declare_block_entity_type(
                    "machine",
                    vec!["test:second".into(), "test:first".into()],
                );
                r.game_rule_category("machines").unwrap();
                KEY.set(r.game_rule_bool("active", "test:machines", true).unwrap())
                    .unwrap();
            }
        }
        install_parts(&[mod_api::ModEntry::new("test", Box::new(Mod))]).unwrap();
        let declarations = block_entity_declarations();
        assert_eq!(declarations[0].id, "test:machine");
        assert_eq!(declarations[0].blocks, ["test:second", "test:first"]);
        let registry = game_rule_registry();
        let key = *KEY.get().unwrap();
        assert_eq!(
            mod_api::__private::game_rule_bool_slot(key),
            registry.position("test:active")
        );
        assert_eq!(
            registry.rule("test:active").unwrap().default,
            horizon_client_core::gamerules::Value::Bool(true)
        );
    }
}

impl TryIntoApi for horizon_client_core::text::Text {
    type Api = mod_api::text::Text;
    type Error = std::convert::Infallible;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(captured_text(&self.component()))
    }
}

fn captured_text(value: &serde_json::Value) -> mod_api::text::Text {
    use mod_api::text::Text;
    use serde_json::Value;
    match value {
        Value::Object(fields) => {
            let mut metadata = text_metadata(fields).into_iter().collect();
            let mut text = Text::new(captured_text_content(fields, &mut metadata));
            text.style = captured_text_style(fields);
            text.children = fields
                .get("extra")
                .and_then(Value::as_array)
                .map(|children| children.iter().map(captured_text).collect())
                .unwrap_or_default();
            text.metadata = metadata;
            text
        }
        Value::Array(values) => {
            let Some((first, rest)) = values.split_first() else {
                return Text::default();
            };
            let mut text = captured_text(first);
            text.children.extend(rest.iter().map(captured_text));
            text
        }
        Value::String(value) => Text::literal(value),
        Value::Null => Text::default(),
        Value::Number(_) | Value::Bool(_) => Text::literal(value.to_string()),
    }
}

fn captured_text_content(
    fields: &serde_json::Map<String, serde_json::Value>,
    metadata: &mut std::collections::BTreeMap<String, serde_json::Value>,
) -> mod_api::text::TextContent {
    use horizon_client_core::{
        component,
        text::{InlineObject, Text},
    };
    use mod_api::text::{TextContent as C, TextNbtSource as N, TextObject as O};
    use serde_json::Value;
    let separator = || fields.get("separator").map(|v| Box::new(captured_text(v)));
    // Keep structured fallback data; reuse the engine's profile/hat/atlas defaults.
    if let Some(object) = component::inline_object(fields, &mut |s| Text::literal(s), &mut |_| None)
    {
        let fallback = fields.get("fallback").map(|v| Box::new(captured_text(v)));
        return C::Object(match object {
            InlineObject::Player { profile, hat, .. } => O::Player {
                profile,
                hat,
                fallback,
            },
            InlineObject::Atlas { atlas, sprite, .. } => O::Atlas {
                atlas,
                sprite,
                fallback,
            },
        });
    }
    if let Some(value) = fields.get("text") {
        return C::Literal(
            value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| value.to_string()),
        );
    }
    if let Some(key) = fields.get("translate").and_then(Value::as_str) {
        return C::Translatable {
            key: key.into(),
            fallback: fields
                .get("fallback")
                .and_then(Value::as_str)
                .map(str::to_owned),
            arguments: fields
                .get("with")
                .and_then(Value::as_array)
                .map(|v| v.iter().map(captured_text).collect())
                .unwrap_or_default(),
        };
    }
    if let Some(pattern) = fields.get("selector").and_then(Value::as_str) {
        return C::Selector {
            pattern: pattern.into(),
            separator: separator(),
        };
    }
    if let Some(key) = fields.get("keybind").and_then(Value::as_str) {
        return C::Keybind(key.into());
    }
    if let Some(score) = fields.get("score").and_then(Value::as_object)
        && let (Some(name), Some(objective)) = (
            score.get("name").and_then(Value::as_str),
            score.get("objective").and_then(Value::as_str),
        )
    {
        let extra: serde_json::Map<_, _> = score
            .iter()
            .filter(|(key, _)| !["name", "objective"].contains(&key.as_str()))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        if !extra.is_empty() {
            metadata.insert("score".into(), Value::Object(extra));
        }
        return C::Score {
            name: name.into(),
            objective: objective.into(),
        };
    }
    if let Some(path) = fields.get("nbt").and_then(Value::as_str) {
        let source = [
            ("block", N::Block as fn(String) -> N),
            ("entity", N::Entity),
            ("storage", N::Storage),
        ]
        .into_iter()
        .find_map(|(field, make)| {
            fields
                .get(field)
                .and_then(Value::as_str)
                .map(|v| make(v.into()))
        });
        if let Some(source) = source {
            return C::Nbt {
                path: path.into(),
                interpret: component::style_flag(fields.get("interpret")).unwrap_or(false),
                source,
                separator: separator(),
            };
        }
    }
    C::Literal(String::new())
}

fn captured_click(
    event: &serde_json::Map<String, serde_json::Value>,
) -> Option<mod_api::text::TextClick> {
    use horizon_client_core::{component, text::ClickEvent as E};
    use mod_api::text::TextClick as A;
    Some(match component::parse_click_event(event)? {
        E::OpenUrl(v) => A::OpenUrl(v),
        E::ChangePage(v) => A::ChangePage(v),
        E::SuggestCommand(v) => A::SuggestCommand(v),
        E::RunCommand(v) => A::RunCommand(v),
        E::CopyToClipboard(v) => A::CopyToClipboard(v),
        E::ShowDialog(_) => A::ShowDialog(event["dialog"].clone()),
        E::Custom { id, .. } => A::Custom {
            id,
            payload: event.get("payload").filter(|v| !v.is_null()).cloned(),
        },
        E::OpenFile(_) => return None,
    })
}

fn captured_hover(
    event: &serde_json::Map<String, serde_json::Value>,
) -> Option<mod_api::text::TextHover> {
    use mod_api::text::TextHover as H;
    use serde_json::Value;
    match event.get("action")?.as_str()? {
        "show_text" => Some(H::Text(Box::new(captured_text(
            event
                .get("value")
                .or_else(|| event.get("contents"))
                .or_else(|| event.get("text"))?,
        )))),
        "show_item" => Some(H::Item {
            id: event.get("id")?.as_str()?.into(),
            count: event
                .get("count")
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok())
                .unwrap_or(1),
            components: event.get("components").cloned(),
        }),
        "show_entity" => {
            let uuid = event.get("uuid")?;
            let words = if let Some(text) = uuid.as_str() {
                horizon_client_core::uuid::uuid_from_text(text)?
            } else {
                let values: &[Value; 4] = uuid.as_array()?.as_slice().try_into().ok()?;
                let mut words = [0; 4];
                for (out, value) in words.iter_mut().zip(values) {
                    *out = i32::try_from(value.as_i64()?).ok()?;
                }
                words
            }
            .map(|word| word as u32 as u64);
            Some(H::Entity {
                kind: event.get("id")?.as_str()?.into(),
                id: [words[0] << 32 | words[1], words[2] << 32 | words[3]],
                name: event.get("name").map(|v| Box::new(captured_text(v))),
            })
        }
        _ => None,
    }
}

fn captured_text_style(
    fields: &serde_json::Map<String, serde_json::Value>,
) -> mod_api::text::TextStyle {
    use horizon_client_core::component;
    use serde_json::Value;
    let mut style = mod_api::text::TextStyle::default();
    style.color = fields
        .get("color")
        .and_then(Value::as_str)
        .map(str::to_owned);
    style.shadow_color = fields.get("shadow_color").and_then(component::shadow_color);
    style.bold = component::style_flag(fields.get("bold"));
    style.italic = component::style_flag(fields.get("italic"));
    style.underlined = component::style_flag(fields.get("underlined"));
    style.strikethrough = component::style_flag(fields.get("strikethrough"));
    style.obfuscated = component::style_flag(fields.get("obfuscated"));
    style.font = fields
        .get("font")
        .and_then(Value::as_str)
        .map(str::to_owned);
    style.insertion = fields
        .get("insertion")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let click = component::style_event_object(fields, "click_event", "clickEvent");
    let hover = component::style_event_object(fields, "hover_event", "hoverEvent");
    style.click = click.and_then(captured_click);
    style.hover = hover.and_then(captured_hover);
    for (field, event) in [
        ("click_event", click.filter(|_| style.click.is_some())),
        ("hover_event", hover.filter(|_| style.hover.is_some())),
    ] {
        if let Some(event) = event {
            let metadata = text_event_metadata(event);
            if !metadata.is_empty() {
                style.metadata.insert(field.into(), Value::Object(metadata));
            }
        }
    }
    style
}

impl<C: ?Sized> TryIntoEngine<C> for mod_api::text::Text {
    type Engine = horizon_client_core::text::Text;
    type Error = std::convert::Infallible;
    fn try_into_engine(&self, _: &C) -> Result<Self::Engine, Self::Error> {
        Ok(owned_text_component(self).into())
    }
}

fn text_metadata<'a>(
    metadata: impl IntoIterator<Item = (&'a String, &'a serde_json::Value)>,
) -> serde_json::Map<String, serde_json::Value> {
    let reserved = [
        "type",
        "text",
        "translate",
        "with",
        "fallback",
        "keybind",
        "selector",
        "separator",
        "score",
        "nbt",
        "interpret",
        "block",
        "entity",
        "storage",
        "object",
        "player",
        "hat",
        "atlas",
        "sprite",
        "extra",
        "color",
        "shadow_color",
        "bold",
        "italic",
        "underlined",
        "strikethrough",
        "obfuscated",
        "font",
        "insertion",
        "click_event",
        "hover_event",
        "clickEvent",
        "hoverEvent",
    ];
    metadata
        .into_iter()
        .filter(|(key, _)| !reserved.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn owned_text_component(text: &mod_api::text::Text) -> serde_json::Value {
    use mod_api::text::{TextContent as C, TextNbtSource as N, TextObject as O};
    use serde_json::Value;
    let mut out = text_metadata(&text.metadata);
    match &text.content {
        C::Literal(value) => {
            out.insert("text".into(), value.clone().into());
        }
        C::Translatable {
            key,
            fallback,
            arguments,
        } => {
            let arguments: Vec<horizon_client_core::text::Text> = arguments
                .iter()
                .map(|t| owned_text_component(t).into())
                .collect();
            // Reuse the engine's numeric-argument and fallback construction rules.
            let translated = match fallback {
                Some(fallback) => horizon_client_core::text::Text::translate_with_fallback(
                    key, fallback, &arguments,
                ),
                None => horizon_client_core::text::Text::translate(key, &arguments),
            };
            let Value::Object(fields) = translated.component() else {
                unreachable!("translation constructor returns a component object")
            };
            out.extend(fields);
        }
        C::Keybind(value) => {
            out.insert("keybind".into(), value.clone().into());
        }
        C::Selector { pattern, separator } => {
            out.insert("selector".into(), pattern.clone().into());
            if let Some(separator) = separator {
                out.insert("separator".into(), owned_text_component(separator));
            }
        }
        C::Score { name, objective } => {
            let mut score = text
                .metadata
                .get("score")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            score.insert("name".into(), name.clone().into());
            score.insert("objective".into(), objective.clone().into());
            out.insert("score".into(), Value::Object(score));
        }
        C::Nbt {
            path,
            interpret,
            source,
            separator,
        } => {
            out.insert("nbt".into(), path.clone().into());
            out.insert("interpret".into(), (*interpret).into());
            let (key, value) = match source {
                N::Block(v) => ("block", v),
                N::Entity(v) => ("entity", v),
                N::Storage(v) => ("storage", v),
            };
            out.insert(key.into(), value.clone().into());
            if let Some(separator) = separator {
                out.insert("separator".into(), owned_text_component(separator));
            }
        }
        C::Object(object) => {
            let fallback = match object {
                O::Player {
                    profile,
                    hat,
                    fallback,
                } => {
                    out.extend([
                        ("object".into(), "player".into()),
                        ("player".into(), profile.clone()),
                        ("hat".into(), (*hat).into()),
                    ]);
                    fallback
                }
                O::Atlas {
                    atlas,
                    sprite,
                    fallback,
                } => {
                    out.extend([
                        ("object".into(), "atlas".into()),
                        ("atlas".into(), atlas.clone().into()),
                        ("sprite".into(), sprite.clone().into()),
                    ]);
                    fallback
                }
            };
            if let Some(fallback) = fallback {
                out.insert("fallback".into(), owned_text_component(fallback));
            }
        }
    }
    out.extend(owned_text_style(&text.style));
    if !text.children.is_empty() {
        out.insert(
            "extra".into(),
            text.children.iter().map(owned_text_component).collect(),
        );
    }
    Value::Object(out)
}

fn text_action_metadata(
    style: &mod_api::text::TextStyle,
    field: &str,
) -> serde_json::Map<String, serde_json::Value> {
    style
        .metadata
        .get(field)
        .and_then(serde_json::Value::as_object)
        .map(text_event_metadata)
        .unwrap_or_default()
}

fn text_event_metadata(
    event: &serde_json::Map<String, serde_json::Value>,
) -> serde_json::Map<String, serde_json::Value> {
    let reserved = [
        "action",
        "url",
        "page",
        "command",
        "value",
        "contents",
        "dialog",
        "id",
        "payload",
        "path",
        "count",
        "components",
        "uuid",
        "name",
        "text",
    ];
    event
        .iter()
        .filter(|(key, _)| !reserved.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn text_action(
    action: &str,
    field: &str,
    value: serde_json::Value,
) -> serde_json::Map<String, serde_json::Value> {
    [("action".into(), action.into()), (field.into(), value)]
        .into_iter()
        .collect()
}

fn owned_text_style(
    style: &mod_api::text::TextStyle,
) -> serde_json::Map<String, serde_json::Value> {
    use mod_api::text::{TextClick as C, TextHover as H};
    use serde_json::Value;
    let mut out = text_metadata(&style.metadata);
    for (key, value) in [
        ("color", style.color.clone().map(Value::String)),
        (
            "shadow_color",
            style.shadow_color.map(|c| Value::from(c as i32)),
        ),
        ("bold", style.bold.map(Value::Bool)),
        ("italic", style.italic.map(Value::Bool)),
        ("underlined", style.underlined.map(Value::Bool)),
        ("strikethrough", style.strikethrough.map(Value::Bool)),
        ("obfuscated", style.obfuscated.map(Value::Bool)),
        ("font", style.font.clone().map(Value::String)),
        ("insertion", style.insertion.clone().map(Value::String)),
    ] {
        if let Some(value) = value {
            out.insert(key.into(), value);
        }
    }
    if let Some(click) = &style.click {
        let fields = match click {
            C::OpenUrl(v) => text_action("open_url", "url", v.clone().into()),
            C::ChangePage(v) => text_action("change_page", "page", (*v).into()),
            C::SuggestCommand(v) => text_action("suggest_command", "command", v.clone().into()),
            C::RunCommand(v) => text_action("run_command", "command", v.clone().into()),
            C::CopyToClipboard(v) => text_action("copy_to_clipboard", "value", v.clone().into()),
            C::ShowDialog(v) => text_action("show_dialog", "dialog", v.clone()),
            C::Custom { id, payload } => {
                let mut fields = text_action("custom", "id", id.clone().into());
                if let Some(payload) = payload {
                    fields.insert("payload".into(), payload.clone());
                }
                fields
            }
        };
        let mut action = text_action_metadata(style, "click_event");
        action.extend(fields);
        out.insert("click_event".into(), Value::Object(action));
    }
    if let Some(hover) = &style.hover {
        let mut action = text_action_metadata(style, "hover_event");
        match hover {
            H::Text(value) => {
                action.insert("action".into(), "show_text".into());
                action.insert("value".into(), owned_text_component(value));
            }
            H::Item {
                id,
                count,
                components,
            } => {
                action.insert("action".into(), "show_item".into());
                action.insert("id".into(), id.clone().into());
                action.insert("count".into(), (*count).into());
                if let Some(components) = components {
                    action.insert("components".into(), components.clone());
                }
            }
            H::Entity { kind, id, name } => {
                action.insert("action".into(), "show_entity".into());
                action.insert("id".into(), kind.clone().into());
                action.insert(
                    "uuid".into(),
                    horizon_client_core::uuid::uuid_text([
                        (id[0] >> 32) as i32,
                        id[0] as i32,
                        (id[1] >> 32) as i32,
                        id[1] as i32,
                    ])
                    .into(),
                );
                if let Some(name) = name {
                    action.insert("name".into(), owned_text_component(name));
                }
            }
        }
        out.insert("hover_event".into(), Value::Object(action));
    }
    out
}

#[cfg(test)]
mod text_capture_tests {
    use super::{TryIntoApi, TryIntoEngine};
    use horizon_client_core::text::Text as EngineText;
    use mod_api::text::{TextContent as C, TextHover};
    use serde_json::json;

    #[test]
    fn untrusted_file_action_cannot_become_local_after_a_noop_value_round_trip() {
        let source = json!({"text":"server", "click_event":{"action":"open_file", "path":"test-file"},
            "extra":[{"text":"child", "clickEvent":{"action":"open_file", "value":"test-file"}}]});
        let mut api = EngineText::from(source).try_into_api().unwrap();
        assert!(api.style.click.is_none());
        assert!(!api.metadata.contains_key("click_event"));
        assert!(!api.style.metadata.contains_key("click_event"));
        assert!(api.children[0].style.click.is_none());
        let listener = |_: &mut mod_api::text::Text| {};
        listener(&mut api);
        let engine = api.try_into_engine(&()).unwrap();
        assert!(engine.spans.iter().all(|span| span.style.click.is_none()));
        let component = engine.component();
        assert!(component.get("click_event").is_none());
        assert!(component["extra"][0].get("clickEvent").is_none());
    }

    #[test]
    fn component_tree_capture_keeps_dynamic_contents_and_unknown_metadata() {
        let source = json!({"translate":"test.message", "fallback":"%s", "with":[{"text":"name", "color":"red"}],
            "bold":false, "test:meta":{"n":null}, "click_event":{"action":"custom", "id":"test:open", "payload":{"n":null}, "test:trace":true},
            "extra":[{"nbt":"value", "storage":"test:data", "interpret":true, "separator":{"text":","}},
                {"score":{"name":"@s", "objective":"test", "test:trace":2}}]});
        let api = EngineText::from(source.clone()).try_into_api().unwrap();
        assert!(
            matches!(&api.content, C::Translatable { key, fallback:Some(fallback), arguments } if key == "test.message" && fallback == "%s" && arguments.len() == 1)
        );
        assert_eq!(api.style.bold, Some(false));
        assert!(
            matches!(&api.children[0].content, C::Nbt { path, interpret:true, .. } if path == "value")
        );
        assert_eq!(
            api.style.metadata["click_event"],
            json!({"test:trace":true})
        );
        assert_eq!(api.try_into_engine(&()).unwrap().component(), source);
    }

    #[test]
    fn object_fallback_round_trip_keeps_omitted_styled_and_translatable() {
        for object in [
            json!({"object":"player", "player":{"name":"Test", "properties":[{"name":"textures", "value":"data", "signature":"signed"}]}, "hat":true}),
            json!({"object":"atlas", "atlas":"minecraft:blocks", "sprite":"test:icon"}),
        ] {
            for fallback in [
                None,
                Some(
                    json!({"text":"label", "italic":false, "extra":[{"text":"child", "bold":true}]}),
                ),
                Some(json!({"translate":"test.head", "with":[{"text":"name"}]})),
            ] {
                let mut source = object.clone();
                if let Some(fallback) = fallback {
                    source["fallback"] = fallback;
                }
                let api = EngineText::from(source.clone()).try_into_api().unwrap();
                assert_eq!(api.try_into_engine(&()).unwrap().component(), source);
            }
        }
    }

    #[test]
    fn hover_uuid_forms_keep_identity_and_unresolved_name() {
        for uuid in [
            json!("11223344-5566-7788-99aa-bbccddeeff00"),
            json!([287454020, 1432778632, -1716864052_i64, -571539712]),
        ] {
            let source = json!({"text":"entity", "hover_event":{"action":"show_entity", "id":"minecraft:pig", "uuid":uuid, "name":{"translate":"test.name"}, "test:trace":true}});
            let api = EngineText::from(source).try_into_api().unwrap();
            let Some(TextHover::Entity {
                id,
                name: Some(name),
                ..
            }) = &api.style.hover
            else {
                panic!("entity hover")
            };
            assert_eq!(*id, [0x1122334455667788, 0x99aabbccddeeff00]);
            assert!(matches!(&name.content, C::Translatable { key, .. } if key == "test.name"));
            assert_eq!(
                api.try_into_engine(&()).unwrap().component()["hover_event"],
                json!({"action":"show_entity", "id":"minecraft:pig", "uuid":"11223344-5566-7788-99aa-bbccddeeff00", "name":{"translate":"test.name"}, "test:trace":true})
            );
        }
    }

    #[test]
    fn style_capture_reuses_snbt_flags_and_rgba_shadow_rules() {
        let source = json!({"text":"styled", "bold":0, "italic":1, "shadow_color":[1,0,0.5,1]});
        let api = EngineText::from(source).try_into_api().unwrap();
        assert_eq!(api.style.bold, Some(false));
        assert_eq!(api.style.italic, Some(true));
        assert_eq!(api.style.shadow_color, Some(0xffff007f));
    }
}

#[cfg(test)]
mod text_encoding_tests {
    use super::TryIntoEngine;
    use mod_api::text::{Text, TextClick, TextContent as C, TextHover, TextNbtSource, TextObject};
    use serde_json::json;

    #[test]
    fn owned_text_keeps_unresolved_translation_style_and_ordered_children() {
        let mut text = Text::translate("test.message", vec![Text::literal("7")]);
        text.style.bold = Some(false);
        text.style.color = Some("red".into());
        text.style.shadow_color = Some(0xff001122);
        text.style.click = Some(TextClick::RunCommand("/test".into()));
        text.style
            .metadata
            .insert("click_event".into(), json!({"test:extra":true}));
        text.metadata
            .insert("test:context".into(), json!({"value":null}));
        text.children.push(Text::literal("first"));
        text.children.push(Text::literal("second"));
        let engine: Result<horizon_client_core::text::Text, std::convert::Infallible> =
            text.try_into_engine(&());
        assert_eq!(
            engine.unwrap().component(),
            json!({
                "translate":"test.message", "with":[7], "bold":false, "color":"red",
                "shadow_color":-16772830, "click_event":{"action":"run_command", "command":"/test", "test:extra":true},
                "test:context":{"value":null}, "extra":[{"text":"first"}, {"text":"second"}]
            })
        );
    }

    #[test]
    fn owned_text_retains_dynamic_sources_and_inline_profile_properties() {
        for (content, expected) in [
            (C::Keybind("key.jump".into()), json!({"keybind":"key.jump"})),
            (
                C::Selector {
                    pattern: "@p".into(),
                    separator: Some(Box::new(Text::literal(","))),
                },
                json!({"selector":"@p", "separator":{"text":","}}),
            ),
            (
                C::Score {
                    name: "@s".into(),
                    objective: "test".into(),
                },
                json!({"score":{"name":"@s", "objective":"test"}}),
            ),
            (
                C::Nbt {
                    path: "value".into(),
                    interpret: true,
                    source: TextNbtSource::Storage("test:data".into()),
                    separator: None,
                },
                json!({"nbt":"value", "interpret":true, "storage":"test:data"}),
            ),
            (
                C::Object(TextObject::Player {
                    profile: json!({"name":"Test", "properties":[{"name":"textures", "value":"data", "signature":"signed"}]}),
                    hat: false,
                    fallback: Some(Box::new(Text::literal("head"))),
                }),
                json!({"object":"player", "player":{"name":"Test", "properties":[{"name":"textures", "value":"data", "signature":"signed"}]}, "hat":false, "fallback":{"text":"head"}}),
            ),
        ] {
            let engine = Text::new(content).try_into_engine(&()).unwrap();
            assert_eq!(engine.component(), expected);
        }
    }

    #[test]
    fn object_fallback_encoder_keeps_default_styling_and_translation_tree() {
        let mut styled = Text::literal("styled");
        styled.style.italic = Some(false);
        for player in [true, false] {
            for (fallback, expected) in [
                (None, None),
                (
                    Some(Box::new(styled.clone())),
                    Some(json!({"text":"styled", "italic":false})),
                ),
                (
                    Some(Box::new(Text::translate("test.head", vec![]))),
                    Some(json!({"translate":"test.head"})),
                ),
            ] {
                let object = if player {
                    TextObject::Player {
                        profile: json!({"name":"Test"}),
                        hat: true,
                        fallback,
                    }
                } else {
                    TextObject::Atlas {
                        atlas: "minecraft:blocks".into(),
                        sprite: "test:icon".into(),
                        fallback,
                    }
                };
                let engine = Text::new(C::Object(object)).try_into_engine(&()).unwrap();
                match expected {
                    None => {
                        assert!(engine.component().get("fallback").is_none());
                        assert_eq!(
                            engine.plain(),
                            if player { "[Test head]" } else { "[test:icon]" }
                        );
                    }
                    Some(expected) => assert_eq!(engine.component()["fallback"], expected),
                }
            }
        }
    }

    #[test]
    fn raw_file_action_metadata_does_not_reach_the_engine() {
        let mut text = Text::literal("remote");
        for metadata in [&mut text.metadata, &mut text.style.metadata] {
            for field in ["click_event", "clickEvent"] {
                metadata.insert(
                    field.into(),
                    json!({"action":"open_file", "path":"test-file"}),
                );
            }
        }
        let engine = text.try_into_engine(&()).unwrap();
        assert!(engine.spans.iter().all(|span| span.style.click.is_none()));
        assert!(engine.component().get("click_event").is_none());
        assert!(engine.component().get("clickEvent").is_none());
    }

    #[test]
    fn owned_text_keeps_hover_uuid_name_and_nullable_component_patch() {
        let mut text = Text::literal("entity");
        text.style.hover = Some(TextHover::Entity {
            kind: "minecraft:pig".into(),
            id: [0x1122334455667788, 0x99aabbccddeeff00],
            name: Some(Box::new(Text::literal("name"))),
        });
        let engine = text.try_into_engine(&()).unwrap();
        assert_eq!(
            engine.component()["hover_event"],
            json!({"action":"show_entity", "id":"minecraft:pig", "uuid":"11223344-5566-7788-99aa-bbccddeeff00", "name":{"text":"name"}})
        );
        text.style.hover = Some(TextHover::Item {
            id: "minecraft:stone".into(),
            count: 3,
            components: Some(json!({"minecraft:custom_name":null})),
        });
        let engine = text.try_into_engine(&()).unwrap();
        assert_eq!(
            engine.component()["hover_event"],
            json!({"action":"show_item", "id":"minecraft:stone", "count":3, "components":{"minecraft:custom_name":null}})
        );
    }
}

impl TryIntoApi for horizon_world::level::PlacementContext {
    type Api = mod_api::PlacementContext;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        let item = if self.item.is_empty() {
            None
        } else {
            Some(mod_api::__private::item_key(&self.item).ok_or(StackReadError::UnknownItem)?)
        };
        let mut snapshot = mod_api::PlacementContext::new(self.pos, self.face, item);
        snapshot.hit_pos = self.hit_pos;
        snapshot.hit = [self.hit.x, self.hit.y, self.hit.z];
        snapshot.inside = self.inside;
        snapshot.yaw = self.yaw;
        snapshot.pitch = self.pitch;
        snapshot.sneaking = self.sneaking;
        snapshot.replace_clicked = self.replace_clicked;
        Ok(snapshot)
    }
}

#[cfg(test)]
mod placement_snapshot_tests {
    use super::*;
    #[test]
    fn placement_keeps_click_view_flags_and_empty_hand_without_fallback() {
        install_parts(&[]).unwrap();
        mod_api::__private::resolve_items(["minecraft:stone"]).unwrap();
        let mut engine = horizon_world::level::PlacementContext {
            pos: horizon_core::position::BlockPos::new(1, 2, 3),
            hit_pos: horizon_core::position::BlockPos::new(4, 5, 6),
            face: horizon_core::geometry::Direction::Up,
            hit: horizon_core::geometry::Vec3::new(4.25, 5.5, 6.75),
            inside: true,
            yaw: -45.0,
            pitch: 90.0,
            sneaking: true,
            item: "minecraft:stone".into(),
            replace_clicked: true,
        };
        let snapshot = engine.try_into_api().unwrap();
        assert_eq!(snapshot.pos, horizon_core::position::BlockPos::new(1, 2, 3));
        assert_eq!(
            snapshot.hit_pos,
            horizon_core::position::BlockPos::new(4, 5, 6)
        );
        assert_eq!(snapshot.face, horizon_core::geometry::Direction::Up);
        assert_eq!(snapshot.hit, [4.25, 5.5, 6.75]);
        assert!(snapshot.inside);
        assert_eq!(snapshot.yaw, -45.0);
        assert_eq!(snapshot.pitch, 90.0);
        assert!(snapshot.sneaking && snapshot.replace_clicked);
        assert_eq!(
            snapshot.item,
            mod_api::__private::item_key("minecraft:stone")
        );
        engine.item.clear();
        let mut empty = snapshot;
        empty.item = None;
        assert_eq!(engine.try_into_api(), Ok(empty));
        engine.item = "missing:item".into();
        assert_eq!(engine.try_into_api(), Err(StackReadError::UnknownItem));
    }
}

impl TryIntoApi for horizon_client_core::gamerules::Value {
    type Api = mod_api::GameRuleValue;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(match *self {
            Self::Bool(value) => mod_api::GameRuleValue::Bool(value),
            Self::Int(value) => mod_api::GameRuleValue::Int(value),
        })
    }
}

fn with_rule_schema<T>(f: impl FnOnce(&mod_api::__private::RuleSchema<'_>) -> T) -> T {
    let registry = horizon_client_core::gamerules::rule_registry();
    f(&mod_api::__private::RuleSchema {
        category: &|id| registry.categories().iter().any(|category| category == id),
        position: &|id| registry.position(id),
        rule_count: u32::try_from(registry.available_rules(true).count())
            .expect("vanilla rule count fits u32"),
    })
}
pub(in crate::game) fn install_parts(mods: &[mod_api::ModEntry]) -> Result<(), Vec<String>> {
    with_rule_schema(|schema| mod_api::__private::install_parts(mods, schema))
}
#[cfg(test)]
pub(in crate::game) fn register_game_keys(ids: impl IntoIterator<Item = String>) {
    with_rule_schema(|schema| mod_api::__private::part_tests::register_game_keys(ids, schema));
}

pub(in crate::game) fn game_rule_registry() -> horizon_client_core::gamerules::RuleRegistry {
    let rules: Vec<_> = mod_api::__private::game_rules()
        .iter()
        .map(|rule| rule.try_into_engine(&()).unwrap())
        .collect();
    horizon_client_core::gamerules::RuleRegistry::with_bool_rules(
        mod_api::__private::game_rule_categories(),
        &rules,
    )
    .expect("validated at registration")
}
