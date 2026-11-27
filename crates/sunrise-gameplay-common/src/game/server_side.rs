#[path = "world/world_loading.rs"]
pub mod world_loading;

#[path = "world/wake_time.rs"]
pub mod wake_time;

#[path = "world/view_pass.rs"]
pub mod view_pass;

#[path = "world/server_travel.rs"]
pub mod server_travel;

#[path = "world/levels.rs"]
pub mod levels;

#[path = "world/carrier_transfers.rs"]
pub mod carrier_transfers;

#[path = "network/lan_trading.rs"]
pub(in crate::game) mod lan_trading;

#[path = "network/lan_entities.rs"]
pub(in crate::game) mod lan_entities;

#[path = "network/lan/mod.rs"]
pub mod lan;

#[path = "modding/native_unspawned_mob.rs"]
pub mod native_unspawned_mob;

#[path = "modding/native_sound_refs.rs"]
pub mod native_sound_refs;

#[path = "modding/native_scoreboard.rs"]
pub mod native_scoreboard;

#[path = "modding/native_merchant_refs.rs"]
pub mod native_merchant_refs;

#[path = "modding/native_menus.rs"]
pub mod native_menus;

#[path = "modding/native_living_refs.rs"]
pub mod native_living_refs;

#[path = "modding/native_item_places.rs"]
pub mod native_item_places;

#[path = "modding/native_harvest_queries.rs"]
pub mod native_harvest_queries;

#[path = "modding/native_entity_riding.rs"]
pub(in crate::game) mod native_entity_riding;

#[path = "modding/native_entity_motion.rs"]
pub mod native_entity_motion;

#[path = "modding/native_command_reads.rs"]
pub mod native_command_reads;

#[path = "modding/native_block_entity_refs.rs"]
pub mod native_block_entity_refs;

#[path = "mobs/piglins.rs"]
pub mod piglins;

#[path = "mobs/natural_spawning.rs"]
pub mod natural_spawning;

#[path = "mobs/mounts.rs"]
pub mod mounts;

#[path = "mobs/living_attributes.rs"]
pub mod living_attributes;

#[path = "mobs/dragon.rs"]
pub mod dragon;

#[path = "items/trident.rs"]
pub(in crate::game) mod trident;

#[path = "items/consume.rs"]
pub(in crate::game) mod consume;

#[path = "interface/dedicated/mod.rs"]
pub mod dedicated;

#[path = "interface/building_players.rs"]
pub mod building_players;

#[path = "interface/advancements/mod.rs"]
pub mod advancements;

#[path = "entities/tnt.rs"]
pub mod tnt;

#[path = "entities/explosion.rs"]
pub mod explosion;

#[path = "entities/entity.rs"]
pub mod entity_server;

#[path = "entities/boats/mod.rs"]
pub mod boats;

#[path = "blocks/spill.rs"]
pub(in crate::game) mod spill;

#[path = "blocks/nether_portal.rs"]
pub(in crate::game) mod nether_portal;

#[path = "blocks/machines/mod.rs"]
pub mod machines;

#[path = "blocks/dispenser.rs"]
pub mod dispenser;

#[path = "blocks/block_entity_ticks.rs"]
pub mod block_entity_ticks;

#[path = "blocks/block_entity_anims.rs"]
pub mod block_entity_anims;

#[path = "blocks/block_edits.rs"]
pub mod block_edits;

#[path = "administration/commands/mod.rs"]
pub mod commands;
