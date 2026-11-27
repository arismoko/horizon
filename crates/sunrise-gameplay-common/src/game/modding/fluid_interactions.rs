//! What a liquid block turns into beside another fluid or block (NeoForge's
//! `FluidInteractionRegistry`): vanilla's two lava rules for each fluid of the lava tag, the interactions native mods declare
//! (`Registrar::fluid_interaction`) and the files of content mods
//! (`data/<id>/sunrise/fluid_interaction/`), resolved against the open world's tags and
//! installed on its behaviours for `LiquidBlock.shouldSpreadLiquid`.
use std::sync::Arc;
use sunrise_items::items::Items;
use sunrise_mod_package::content::{
    FluidInteraction,
    fluid_interaction::{BlockMatch, Neighbour},
};
use sunrise_world::{
    fluid::FluidKind,
    level::{
        Behaviours,
        data::BlockSet,
        fluid_interactions::{
            BlockMatch as Resolved, Entry, FluidInteraction as Rule, FluidInteractions,
            Neighbour as Walk,
        },
    },
};

/// The fluids of a fluid tag; an unknown tag has none.
pub(in crate::game) fn fluid_tag(items: &Items, tag: &str) -> Vec<FluidKind> {
    items
        .fluid_tags
        .get(tag)
        .into_iter()
        .flatten()
        .filter_map(|name| FluidKind::from_name(name))
        .collect()
}

/// What a native mod declared, as the files say it: the names behind its keys.
pub(in crate::game) fn declared(
    interaction: &mod_api::FluidInteraction,
) -> Result<FluidInteraction, String> {
    let fluid = |key| {
        mod_api::__private::fluid_id(key)
            .map(str::to_owned)
            .ok_or_else(|| "a fluid key with no name".to_owned())
    };
    let block = |key| {
        mod_api::__private::block_key_id(key)
            .map(str::to_owned)
            .ok_or_else(|| "a block key with no name".to_owned())
    };
    let tag = |key| {
        mod_api::__private::with_tag_name(key, |name| name.map(str::to_owned))
            .ok_or_else(|| "a tag key with no name".to_owned())
    };
    let state = |state: &Option<mod_api::PlacedState>| {
        state
            .as_ref()
            .map(|state| {
                let properties: Vec<_> = state
                    .properties
                    .iter()
                    .map(|p| format!("{}={}", p.name, p.value))
                    .collect();
                let name = block(state.block)?;
                Ok::<_, String>(if properties.is_empty() {
                    name
                } else {
                    format!("{name}[{}]", properties.join(","))
                })
            })
            .transpose()
    };
    Ok(FluidInteraction {
        source: fluid(interaction.source)?,
        neighbour: match interaction.neighbour {
            mod_api::FluidNeighbour::Fluid(key) => Neighbour::Fluid(fluid(key)?),
            mod_api::FluidNeighbour::FluidTag(key) => Neighbour::FluidTag(tag(key)?),
            mod_api::FluidNeighbour::Block(key) => Neighbour::Block(block(key)?),
            mod_api::FluidNeighbour::BlockTag(key) => Neighbour::BlockTag(tag(key)?),
        },
        below: match interaction.below {
            None => None,
            Some(mod_api::BlockSelector::Block(key)) => Some(BlockMatch::Block(block(key)?)),
            Some(mod_api::BlockSelector::Tag(key)) => Some(BlockMatch::BlockTag(tag(key)?)),
        },
        becomes: sunrise_mod_package::content::fluid_interaction::Becomes {
            source: state(&interaction.becomes.source)?,
            flowing: state(&interaction.becomes.flowing)?,
        },
    })
}

impl crate::game::CommonGame<'_> {
    /// [`Self::install_fluid_interactions`] against `items`' tags.
    pub fn install_fluid_interactions_from(&self, items: &Items) -> Result<(), String> {
        use mod_api::__private::RegisteredFluidInteraction;
        let behaviours = self.res.behaviours();
        let table = |declaration: &FluidInteraction| {
            self.resolve_fluid_interaction(behaviours, items, declaration)
                .map(|(kind, rule)| (kind, Entry::Table(rule)))
        };
        let mut entries = Vec::new();
        // A native handler is named by its index among the registrations, which live for the
        // session: `native_fluid_interaction` finds it again there.
        for (index, registration) in mod_api::__private::declared_fluid_interactions()
            .iter()
            .enumerate()
        {
            entries.push(match registration {
                RegisteredFluidInteraction::Declared(interaction) => {
                    table(&declared(interaction)?)?
                }
                RegisteredFluidInteraction::Handler { source, .. } => {
                    let name = mod_api::__private::fluid_id(*source)
                        .ok_or_else(|| "a fluid key with no name".to_owned())?;
                    let kind = FluidKind::from_name(name)
                        .ok_or_else(|| format!("fluid interaction: unknown fluid {name}"))?;
                    let index = u32::try_from(index)
                        .map_err(|_| "fluid interaction: too many registrations".to_owned())?;
                    (kind, Entry::Handler(index))
                }
            });
        }
        for declaration in sunrise_sim::mod_content::fluid_interactions() {
            entries.push(table(&declaration)?);
        }
        let builtin = behaviours.vanilla_fluid_interactions(
            &fluid_tag(items, "minecraft:lava"),
            &fluid_tag(items, "minecraft:water"),
        );
        behaviours.set_fluid_interactions(FluidInteractions::new(builtin, entries));
        Ok(())
    }

    pub(in crate::game) fn resolve_fluid_interaction(
        &self,
        behaviours: &Behaviours,
        items: &Items,
        declaration: &FluidInteraction,
    ) -> Result<(FluidKind, Rule), String> {
        let fluid = |name: &str| {
            FluidKind::from_name(name)
                .ok_or_else(|| format!("fluid interaction: unknown fluid {name}"))
        };
        let block = |name: &str| {
            behaviours
                .table
                .block_by_name(name)
                .ok_or_else(|| format!("fluid interaction: unknown block {name}"))
        };
        let tag = |name: &str| {
            let mut set = BlockSet::with_capacity(behaviours.table.block_count());
            for id in items.block_tags.get(name).into_iter().flatten() {
                if let Some(id) = behaviours.table.block_by_name(id) {
                    set.insert(id);
                }
            }
            Resolved::Tag(Arc::new(set))
        };
        let state = |text: &Option<String>| {
            text.as_deref()
                .map(|text| {
                    self.block_state_id(text)
                        .ok_or_else(|| format!("fluid interaction: unknown block state {text}"))
                })
                .transpose()
        };
        let matched = |matching: &BlockMatch| match matching {
            BlockMatch::Block(name) => block(name).map(Resolved::Block),
            BlockMatch::BlockTag(name) => Ok(tag(name)),
        };
        Ok((
            fluid(&declaration.source)?,
            Rule {
                neighbour: match &declaration.neighbour {
                    Neighbour::Fluid(name) => Walk::Fluid(fluid(name)?),
                    Neighbour::FluidTag(name) => Walk::FluidTag(fluid_tag(items, name).into()),
                    Neighbour::Block(name) => Walk::Blocks(Resolved::Block(block(name)?)),
                    Neighbour::BlockTag(name) => Walk::Blocks(tag(name)),
                },
                below: declaration.below.as_ref().map(matched).transpose()?,
                source: state(&declaration.becomes.source)?,
                flowing: state(&declaration.becomes.flowing)?,
            },
        ))
    }

    /// The open world's fluid interaction table rules, read for one neighbour and the block below.
    pub fn fluid_interaction_result(
        &self,
        source: mod_api::FluidState,
        neighbour: sunrise_world::registry::BlockStateId,
        below: sunrise_world::registry::BlockStateId,
    ) -> Option<sunrise_world::registry::BlockStateId> {
        let registry = &self.pack.assets.registry;
        registry.state(neighbour)?;
        registry.state(below)?;
        let source = super::native_block_states::engine_fluid_state(source)?;
        self.res
            .behaviours()
            .fluid_interaction_result(source, neighbour, below)
    }
}

/// The native fluid interaction a mod registered as `handler`, with its owner, when its owner is
/// still enabled.
pub fn native_fluid_interaction(
    handler: u32,
) -> Option<(
    &'static mod_api::__private::game::CallbackOwner,
    &'static dyn mod_api::FluidInteractionHandler,
)> {
    match mod_api::__private::declared_fluid_interactions().get(usize::try_from(handler).ok()?)? {
        mod_api::__private::RegisteredFluidInteraction::Handler { owner, handler, .. } => {
            Some((owner, handler.as_ref()))
        }
        mod_api::__private::RegisteredFluidInteraction::Declared(_) => None,
    }
}
