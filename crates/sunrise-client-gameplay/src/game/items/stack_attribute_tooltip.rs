//! A stack tooltip's attribute lines (`ItemStack.addAttributeTooltips`, NeoForge's
//! `AttributeUtil.addAttributeTooltips`): the stack's modifiers read with the player's bases and
//! each attribute's declared style, then the lines mods add.
use crate::game::ClientGame;
use sunrise_client::item_tooltip::{
    AttributeStyle, AttributeTooltipContext, AttributeTooltipListener, NoAttributeListener,
    attribute_tooltip,
};
use sunrise_game_data::text::Text;
use sunrise_items::items::ItemStack;
use sunrise_world::attributes::Tooltip;

impl ClientGame<'_> {
    /// The attribute modifier lines of `stack`'s tooltip and the lines mods add after them.
    /// `shown` is whether the stack's `minecraft:tooltip_display` shows the modifiers; a
    /// `nested` stack (a bundle's, a crossbow's projectile) has no player.
    pub(in crate::game::client_side) fn stack_attribute_tooltip(
        &self,
        stack: &ItemStack,
        shown: bool,
        nested: bool,
        advanced: bool,
        creative: bool,
        mod_callbacks: bool,
    ) -> (Vec<(String, u32)>, Vec<(Text, u32)>) {
        // The item's own modifiers, then its enchantments' for each slot
        // group (`ItemStack.forEachModifier`).
        let mut modifiers =
            sunrise_sim::item_attributes::attribute_modifiers(stack, self.items()).into_owned();
        modifiers.extend(self.active_enchantment_definitions().modifier_rows(stack));
        let player_base = (!nested).then(|| {
            [
                self.player_attribute_base("minecraft:attack_damage", 1.0),
                self.player_attribute_base(
                    "minecraft:attack_speed",
                    sunrise_sim::attack_strength::BASE_ATTACK_SPEED,
                ),
            ]
        });
        let context = AttributeTooltipContext {
            gather_when_hidden: false,
            player_base,
            advanced,
            creative,
        };
        // `AttributeUtil.addAttributeTooltips`: the one builder the recipe search shares,
        // with the mods' skipped modifiers and added lines. A nested stack has no player.
        let listener = crate::game::native_hooks::HookedAttributeTooltips::new(
            self,
            (!nested).then(|| (self.own_uuid(), self.own_entity_id())),
        );
        let listener: &dyn AttributeTooltipListener = if mod_callbacks {
            &listener
        } else {
            &NoAttributeListener
        };
        let style = |attribute: &str| self.attribute_style(attribute, !nested);
        let translate = |k: &str| self.common.pack.names.display_get(k);
        attribute_tooltip(
            stack, &modifiers, &context, shown, listener, &translate, &style,
        )
    }

    /// How `attribute`'s modifier lines read, from its declaration in the attribute catalog;
    /// none for vanilla's look, so the vanilla attributes clone and look up nothing. `player`
    /// adds the local player's base value for a declared base modifier's total line; the
    /// recipe search has no player.
    pub(in crate::game) fn attribute_style(
        &self,
        attribute: &str,
        player: bool,
    ) -> Option<AttributeStyle> {
        let catalog = &self.common.motion.attributes;
        let registered = catalog.attribute(catalog.id(attribute)?);
        if registered.tooltip == Tooltip::default() {
            return None;
        }
        Some(AttributeStyle {
            tooltip: registered.tooltip.clone(),
            player_base: (player && registered.tooltip.base_modifier.is_some()).then(|| {
                self.player_attribute_base(attribute, registered.definition.default_value())
            }),
        })
    }

    /// `Player.getAttributeBaseValue`: the attribute's default where the player has none.
    fn player_attribute_base(&self, name: &str, default: f64) -> f64 {
        self.common
            .player
            .as_ref()
            .and_then(|player| {
                let key = player.attributes.key(name)?;
                let handle = player.attributes.lookup(Some(&key))?;
                player
                    .attributes
                    .instance(&handle)
                    .ok()
                    .map(|value| value.base_value())
            })
            .unwrap_or(default)
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
