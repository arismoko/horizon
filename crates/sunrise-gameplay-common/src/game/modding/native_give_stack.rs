//! SNBT and JSON item parsing/encoding for mods (`item_from_snbt`, `item_to_json`, `item_from_json`).
use crate::game::api_values::{StackReadError, TryIntoApi, stack_value};
use mod_api::{ItemJsonError, StackError, StackWriteError};
use sunrise_items::item_nbt;
use sunrise_world::component_value::json::JsonError;

fn map_json_error(error: JsonError) -> ItemJsonError {
    match error {
        JsonError::Syntax(reason) => ItemJsonError::InvalidJson { reason },
        JsonError::Field { field, reason } => ItemJsonError::InvalidField { field, reason },
        JsonError::Value(reason) => ItemJsonError::InvalidValue { reason },
    }
}

fn map_stack_error(reason: StackError) -> ItemJsonError {
    match &reason {
        StackError::InvalidComponent { id } => ItemJsonError::InvalidField {
            field: id.as_str().to_owned(),
            reason: reason.to_string(),
        },
        StackError::InvalidDamage => ItemJsonError::InvalidField {
            field: "minecraft:damage".into(),
            reason: reason.to_string(),
        },
        _ => ItemJsonError::InvalidField {
            field: "components".into(),
            reason: reason.to_string(),
        },
    }
}

pub trait NativeGiveStackGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// `TagParser.parseCompoundFully` followed by `ItemStack.CODEC`. This is text, not
    /// binary input to `NbtAccounter`.
    fn item_from_snbt(&self, snbt: &str) -> Option<mod_api::ItemStack> {
        let allow_custom = self.side_allow_custom_components();
        item_nbt::item_from_snbt_with(
            snbt,
            |id| allow_custom && sunrise_protocol::item::custom::id(id).is_some(),
            |id| self.items().get(id).is_some(),
        )
        .and_then(|stack| {
            self.items()
                .get(&stack.item)
                .ok_or_else(|| format!("unknown item {}", stack.item))?;
            stack.try_into_api().map_err(|error| match error {
                StackReadError::UnknownItem => format!("unresolved item key {}", stack.item),
                StackReadError::Empty => "empty item stack".into(),
                StackReadError::InvalidComponents(reason) => reason.to_string(),
            })
        })
        .inspect_err(|error| {
            let caller = mod_api::content::crash::current_native_mod().unwrap_or("game");
            log::warn!("{caller}: item_from_snbt: {error}");
        })
        .ok()
    }

    /// `ItemStack.OPTIONAL_CODEC` (`ExtraCodecs.optionalEmptyMap(ItemStack.CODEC)`) over `JsonOps`.
    fn item_to_json(&self, stack: Option<&mod_api::ItemStack>) -> Result<String, ItemJsonError> {
        let Some(stack) = stack else {
            return Ok("{}".to_owned());
        };
        let engine = stack_value(stack).map_err(|error| match error {
            StackWriteError::UnknownItem => ItemJsonError::InvalidField {
                field: "id".into(),
                reason: "unknown item".into(),
            },
            StackWriteError::InvalidEffectPayload => ItemJsonError::InvalidField {
                field: "components".into(),
                reason: "invalid registered consume effect payload".into(),
            },
            StackWriteError::InvalidComponents { reason } => map_stack_error(reason),
            _ => ItemJsonError::InvalidValue {
                reason: "the stack can't be written".into(),
            },
        })?;
        let allow_custom = self.side_allow_custom_components();
        let value = item_nbt::item_to_json_with(
            Some(&engine),
            self.items(),
            |id| allow_custom && sunrise_protocol::item::custom::id(id).is_some(),
            |id| self.items().get(id).is_some(),
        )
        .map_err(map_json_error)?;
        serde_json::to_string(&value).map_err(|error| ItemJsonError::InvalidValue {
            reason: error.to_string(),
        })
    }

    /// `ItemStack.OPTIONAL_CODEC` (`ExtraCodecs.optionalEmptyMap(ItemStack.CODEC)`) over `JsonOps`.
    fn item_from_json(&self, json: &str) -> Result<Option<mod_api::ItemStack>, ItemJsonError> {
        let allow_custom = self.side_allow_custom_components();
        let Some(stack) = item_nbt::item_from_json_with(
            json,
            |id| allow_custom && sunrise_protocol::item::custom::id(id).is_some(),
            |id| self.items().get(id).is_some(),
        )
        .map_err(map_json_error)?
        else {
            return Ok(None);
        };
        stack.try_into_api().map(Some).map_err(|error| match error {
            StackReadError::UnknownItem => ItemJsonError::InvalidField {
                field: "id".into(),
                reason: format!("unresolved item key {}", stack.item),
            },
            StackReadError::Empty => ItemJsonError::InvalidValue {
                reason: "empty item stack".into(),
            },
            StackReadError::InvalidComponents(reason) => map_stack_error(reason),
        })
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> NativeGiveStackGameExt1<'__sg> for G {}

use crate::game::side_ops::modapi_modding::ModapiModdingSideGameExt as _;
use crate::game::tables::GameItems as _;
