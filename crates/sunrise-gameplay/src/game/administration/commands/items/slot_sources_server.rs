use super::{R, err};
use crate::game::commands::items::slot_sources::expand;
use crate::game::commands::items::slot_sources::id;
use crate::game::commands::items::slot_sources::typed_source;
use serde_json::Value;

impl crate::game::ServerGame<'_> {
    /// Literal ranges retain priority; all remaining consumers resolve the same holder snapshot.
    pub(in crate::game::server_side) fn resolved_slot_source(&self, raw: &str) -> R<Option<Value>> {
        if sunrise_command::args::slot_ranges()
            .iter()
            .any(|(name, _)| name == raw)
        {
            return Ok(None);
        }
        let value = if self.server.fns.0.store.slot_sources.contains_key(&id(raw)) {
            Value::String(id(raw))
        } else {
            let (tag, _) = sunrise_core::nbt::parse_snbt(raw)
                .map_err(|error| err("argument.resource_or_id.failed_to_parse", &[&error]))?;
            typed_source(&tag)
        };
        expand(&self.server.fns.0.store, &value)
            .map(Some)
            .map_err(|error| err("argument.resource_or_id.failed_to_parse", &[&error]))
    }
}
