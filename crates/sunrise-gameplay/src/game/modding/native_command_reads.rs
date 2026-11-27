//! What a server mod reads beyond a command's arguments (NeoForge's
//! `ICommandSourceStackExtension`): where the command executes, a player's game profile and a
//! loaded advancement.
use crate::game::ServerGame;
use mod_api::{
    AdvancementDisplay, AdvancementFrame, AdvancementInfo, CommandArgument, CommandInvocation,
    GameProfile, Identifier, PlayerRef,
};
use sunrise_gameplay_common::game::api_values::TryIntoApi;
use sunrise_sim::advancements::{Advancement, Frame, Tree};

impl ServerGame<'_> {
    /// A parsed native command with the level, position and rotation it executes at (after
    /// `execute in/at/positioned/rotated/facing`, or the console's overworld spawn corner) and
    /// the entity it executes as (after `execute as/on`). The entity is the leaf source's own:
    /// one that no longer resolves is None, never the player in place.
    pub(in crate::game) fn native_invocation(
        &mut self,
        literals: Vec<String>,
        arguments: Vec<CommandArgument>,
    ) -> CommandInvocation {
        let invocation = CommandInvocation::new(
            literals,
            arguments,
            self.native_level_key(),
            self.source_position(),
            self.source_rotation(),
        );
        let Some(target) = self.source_entity() else {
            return invocation;
        };
        if let Some(player) = self.native_command_player_target(target) {
            return invocation.with_player(player);
        }
        match self.native_command_entity_target(target) {
            Some(entity) => invocation.with_entity(entity),
            None => invocation,
        }
    }

    /// `ServerPlayer.getGameProfile`: the name `player_name` reads, and the properties the
    /// player logged in with. None for a stale or unavailable player.
    pub(in crate::game) fn player_profile(&self, player: PlayerRef) -> Option<GameProfile<'_>> {
        let name = self.player_name(player)?;
        let properties = self
            .online_player_record(player)
            .map(|guest| guest.properties.as_slice())
            .unwrap_or_default();
        Some(mod_api::__private::game::game_profile(
            player.uuid(),
            name,
            properties,
        ))
    }

    /// `ServerAdvancementManager.get(id)` over the loaded tree, read once on first use and
    /// replaced by `/reload`.
    pub(in crate::game) fn advancement(&mut self, id: &Identifier) -> Option<AdvancementInfo> {
        advancement_in(&self.advancement_tree(), id)
    }
}

/// The advancement `id` in `tree`; None when the tree has no such advancement.
fn advancement_in(tree: &Tree, id: &Identifier) -> Option<AdvancementInfo> {
    advancement_info(&tree.list[*tree.by_id.get(&id.to_string())?])
}

/// A loaded advancement as the API's snapshot. None when its id, parent, texts or icon don't
/// cross, which a tree built from valid files never holds.
fn advancement_info(advancement: &Advancement) -> Option<AdvancementInfo> {
    let display = match &advancement.display {
        Some(d) => Some(AdvancementDisplay::new(
            sunrise_game_data::scoreboard::comp_text(&d.title)
                .try_into_api()
                .ok()?,
            sunrise_game_data::scoreboard::comp_text(&d.description)
                .try_into_api()
                .ok()?,
            d.icon.try_into_api().ok()?,
            match d.frame {
                Frame::Task => AdvancementFrame::Task,
                Frame::Goal => AdvancementFrame::Goal,
                Frame::Challenge => AdvancementFrame::Challenge,
            },
            d.show_toast,
            d.announce,
            d.hidden,
        )),
        None => None,
    };
    let parent = match &advancement.parent {
        Some(parent) => Some(Identifier::parse(parent).ok()?),
        None => None,
    };
    Some(AdvancementInfo::new(
        Identifier::parse(&advancement.id).ok()?,
        parent,
        display,
        advancement
            .criteria
            .iter()
            .map(|c| c.name.clone())
            .collect(),
        advancement.requirements.clone(),
    ))
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

#[cfg(test)]
mod tests {
    use sunrise_gameplay_common::game::server_side::native_command_reads::tests::mine_stone;

    use super::*;
    #[cfg(test)]
    #[test]
    fn a_loaded_advancement_crosses_with_its_parent_frame_icon_and_criteria() {
        sunrise_gameplay_common::game::api_values::install_parts(&[]).unwrap();
        mod_api::__private::resolve_items(["minecraft:wooden_pickaxe"]).unwrap();
        let info = advancement_info(&mine_stone()).unwrap();
        assert_eq!(
            info.id,
            Identifier::parse("minecraft:story/mine_stone").unwrap()
        );
        assert_eq!(
            info.parent,
            Some(Identifier::parse("minecraft:story/root").unwrap())
        );
        assert_eq!(info.criteria, ["get_stone", "get_cobbled_deepslate"]);
        assert_eq!(
            info.requirements,
            vec![vec![
                "get_stone".to_owned(),
                "get_cobbled_deepslate".to_owned()
            ]]
        );
        let display = info.display.unwrap();
        assert_eq!(display.frame, AdvancementFrame::Task);
        assert_eq!(
            display.icon.item(),
            mod_api::__private::item_key("minecraft:wooden_pickaxe").unwrap()
        );
        let components = display
            .icon
            .components()
            .expect("the icon keeps its components");
        assert!(
            components
                .as_nbt()
                .keys()
                .any(|key| key.to_string_lossy() == "minecraft:enchantment_glint_override"),
            "{components:?}"
        );
    }

    #[test]
    fn an_unknown_advancement_reads_as_none() {
        sunrise_gameplay_common::game::api_values::install_parts(&[]).unwrap();
        mod_api::__private::resolve_items(["minecraft:wooden_pickaxe"]).unwrap();
        let mut tree = Tree::default();
        tree.by_id.insert("minecraft:story/mine_stone".into(), 0);
        tree.list.push(mine_stone());
        let known = Identifier::parse("minecraft:story/mine_stone").unwrap();
        assert_eq!(advancement_in(&tree, &known).map(|a| a.id), Some(known));
        for unknown in ["minecraft:story/no_such", "mymod:story/mine_stone"] {
            assert_eq!(
                advancement_in(&tree, &Identifier::parse(unknown).unwrap()),
                None,
                "{unknown}"
            );
        }
    }
}
