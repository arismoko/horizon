#[allow(unused_imports)]
use super::Target;
use crate::game::commands::compute::ComputeContext;
use crate::game::commands::compute::context_entities;
use serde_json::Value;
use sunrise_sim::advancements::{self as adv};

impl crate::game::ServerGame<'_> {
    /// Leaf predicates that read the command's world or entity context.
    pub(in crate::game::server_side) fn compute_leaf(
        &mut self,
        cond: &Value,
        c: &ComputeContext,
    ) -> bool {
        let Some(o) = cond.as_object() else {
            return false;
        };
        let kind = o
            .get("type")
            .or_else(|| o.get("condition"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim_start_matches("minecraft:");
        match kind {
            "match_block" | "block_state_property" => {
                let Some(p) = c.block else {
                    return false;
                };
                let state = self.common.world.block(p[0], p[1], p[2]);
                let Some(s) = self.common.pack.assets.registry.state(state) else {
                    return false;
                };
                let name = s.name.to_string();
                let blocks_ok = match o.get("blocks").or_else(|| o.get("block")) {
                    Some(Value::String(b)) => match b.strip_prefix('#') {
                        Some(tag) => self
                            .common
                            .res
                            .items()
                            .block_tags
                            .get(tag.trim_start_matches("minecraft:"))
                            .is_some_and(|set| {
                                set.contains(&*name)
                                    || set.contains(name.trim_start_matches("minecraft:"))
                            }),
                        None => *b == name,
                    },
                    Some(Value::Array(list)) => list.iter().any(|b| b.as_str() == Some(&name)),
                    _ => true,
                };
                let props_ok = o
                    .get("state")
                    .or_else(|| o.get("properties"))
                    .and_then(Value::as_object)
                    .into_iter()
                    .flatten()
                    .all(|(k, v)| s.properties.get(k).map(String::as_str) == v.as_str());
                blocks_ok && props_ok
            }
            _ => {
                let source = self
                    .server
                    .command_source
                    .clone()
                    .unwrap_or_else(|| self.player_source());
                // Scores are read from the holder's board only where
                // an `entity_scores` leaf may ask.
                let scored = adv::asks_scores(cond);
                let view = |game: &mut Self, t| {
                    if scored {
                        game.entity_view_scored(t, cond)
                    } else {
                        game.entity_view_asked(t, cond)
                    }
                };
                let this = source
                    .entity
                    .filter(|_| c.without_this.is_none())
                    .map(|t| view(self, t));
                let target = c.target.map(|t| view(self, t));
                let interacting = c.interacting.map(|t| view(self, t));
                let entities = context_entities(
                    c,
                    this.as_ref(),
                    source.pos,
                    target.as_ref(),
                    interacting.as_ref(),
                );
                self.conditions_hold_in(cond, &entities)
            }
        }
    }
    /// A registered provider's definition.
    pub(in crate::game::server_side) fn provider_file(
        &self,
        registry: &str,
        id: &str,
    ) -> Option<Value> {
        let id = sunrise_command::args::normalized(id);
        match registry {
            "minecraft:predicate" => self.server.fns.0.store.predicates.get(id.as_ref()),
            _ => self.active_providers().registry(registry)?.get(id.as_ref()),
        }
        .cloned()
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;
