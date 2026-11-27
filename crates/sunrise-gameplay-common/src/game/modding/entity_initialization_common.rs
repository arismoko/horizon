//! Shared `entity_constructing` tracking used on both server and client roles.
use crate::game::native_hooks::entity_constructing::GameHooks as _;
use mod_api::Holder;
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub struct EntityInitialization {
    /// The object holders already told (entity IDs and separate player UUIDs). An entity is forgotten when it leaves the world (`forget_entity`:
    /// death, a chunk unloading), so the next one made from its data is
    /// told again, as vanilla constructs a new entity for each of those.
    pub initialized: HashSet<Holder>,
    /// Each LAN guest's connection when its player was last told: a new connection is a new
    /// `ServerPlayer`.
    pub players: HashMap<u128, u32>,
    /// The object being told now: its attachments are reachable before it is in the level.
    pub active: Option<Construction>,
}

#[derive(Clone, Copy)]
pub enum Construction {
    Holder(Holder),
    ServerPlayer { uuid: u128, conn: u32 },
}

impl Construction {
    pub fn holder(self) -> Holder {
        match self {
            Self::Holder(holder) => holder,
            Self::ServerPlayer { uuid, .. } => Holder::Player(uuid),
        }
    }
}

impl EntityInitialization {
    pub fn is_constructing(&self, holder: Holder) -> bool {
        self.active.is_some_and(|object| object.holder() == holder)
    }

    pub fn constructing_player_connection(&self, uuid: u128) -> Option<u32> {
        match self.active {
            Some(Construction::ServerPlayer { uuid: player, conn }) if player == uuid => Some(conn),
            _ => None,
        }
    }
}

impl crate::game::CommonGame<'_> {
    pub fn forget_entity_initialization(&mut self, holder: Holder) {
        if let Holder::Entity(id) = holder
            && let Some(mob) = self.mobs.iter_mut().find(|mob| mob.id == id)
        {
            mob.accepted_hit = None;
        }
        self.entity_initialization.initialized.remove(&holder);
    }
}

pub trait EntityInitializationCommonGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// Tell the listeners about a new entity of type `kind`, unless it was already told.
    fn initialize_entity(&mut self, holder: Holder, kind: &str) {
        if matches!(holder, Holder::Entity(_) | Holder::Player(_)) {
            self.initialize_construction(Construction::Holder(holder), kind);
        }
    }

    /// Mark before dispatch so a listener's attachment access doesn't tell the same object again.
    fn initialize_construction(&mut self, object: Construction, kind: &str) {
        let holder = object.holder();
        if !mod_api::__private::listened(mod_api::HookId::EntityConstructing)
            || !self.entity_initialization.initialized.insert(holder)
        {
            return;
        }
        let previous = self.entity_initialization.active.replace(object);
        let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.native_entity_constructing(holder, kind);
            if let Holder::Entity(id) = holder {
                self.side_after_entity_constructing_fault(id);
            }
        }));
        self.entity_initialization.active = previous;
        if let Err(panic) = run {
            std::panic::resume_unwind(panic);
        }
        if let Holder::Entity(id) = holder {
            self.side_bind_constructed_entity_attachments(id);
        }
    }

    /// `initialize_entity` for a holder whose attachments are about to be read.
    fn initialize_entity_on_access(&mut self, holder: Holder) {
        if !mod_api::__private::listened(mod_api::HookId::EntityConstructing) {
            return;
        }
        match holder {
            Holder::Entity(id) => {
                let Some(kind) = self.entity(id).and_then(|entity| {
                    self.entity_kind(entity)?
                        .id(<Self::State as crate::game::Side>::as_host(self))
                }) else {
                    return;
                };
                self.initialize_entity(holder, &kind);
            }
            Holder::Player(_) => self.initialize_entity(holder, "minecraft:player"),
            _ => {}
        }
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> EntityInitializationCommonGameExt1<'__sg> for G {}

use crate::game::side_ops::modapi_modding::ModapiModdingSideGameExt as _;

use crate::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
