#[allow(unused_macros)]
macro_rules! server_world_items {
    ($s:ident) => {
        match (&$s.common.server_items, $s.server.world_tables.0.as_ref()) {
            (Some(items), _) => &**items,
            (None, Some(t)) => &t.items,
            (None, None) => $s.common.res.items(),
        }
    };
}

#[allow(unused_macros)]
macro_rules! server_world_recipes {
    ($s:ident) => {
        match $s.server.world_tables.0.as_ref() {
            Some(t) => &t.recipes,
            None => $s.common.res.recipes(),
        }
    };
}

#[allow(unused_macros)]
macro_rules! server_world_stations {
    ($s:ident) => {
        match $s.server.world_tables.0.as_ref() {
            Some(t) => &t.stations,
            None => $s.common.res.stations(),
        }
    };
}

/// Direct `self.server` borrows leave sibling fields available for ordered ticks.
#[allow(unused_macros)]
macro_rules! server_game_side {
    ($s:expr) => {{
        let level = $s.common.native_lifecycle.epoch().and_then(|epoch| {
            let key = $s.common.dim.key();
            let id = mod_api::__private::dimension_id(key)?;
            Some(mod_api::LevelRef::with_generation(
                epoch,
                key,
                $s.server.levels.0.runtime.generation_of(id),
            ))
        });
        let player_uuid = $s.server.lan_acting_uuid;
        let session_lock = $s.server.session_lock.0.is_some();
        sunrise_gameplay_common::game_side!(
            $s.common,
            (player_uuid, level, session_lock),
            $s.common.level.enabled_features.as_arc(),
            (session_lock && !$s.common.options.execution_role.is_client())
                .then(|| $s.common.res.items())
        )
    }};
}

macro_rules! world_paintings {
    ($s:ident) => {
        match $s.server.world_tables.0.as_ref() {
            Some(tables) => &tables.painting_variants,
            None => &$s.common.painting_variants,
        }
    };
}

macro_rules! world_variants {
    ($s:ident) => {
        match $s.server.world_tables.0.as_ref() {
            Some(tables) => &*tables.variants,
            None => $s.common.res.variants(),
        }
    };
}

macro_rules! world_items {
    ($s:ident) => {
        server_world_items!($s)
    };
}

macro_rules! world_recipes {
    ($s:ident) => {
        server_world_recipes!($s)
    };
}

macro_rules! world_stations {
    ($s:ident) => {
        server_world_stations!($s)
    };
}

macro_rules! game_side {
    ($s:expr) => {
        server_game_side!($s)
    };
}
