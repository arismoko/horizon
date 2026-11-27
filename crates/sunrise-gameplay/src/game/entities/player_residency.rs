//! Specialized authoritative owners that are not homogeneous entity lists.
use super::commands::Target;
use sunrise_gameplay_common::game::fishing_common::Hook;

pub(in crate::game) enum SpecializedSource<'a> {
    Player(&'a sunrise_sim::motion::PlayerMotion),
    Guest(&'a sunrise_sim::lan::guest::Guest),
    Hook(&'a Hook),
}
use sunrise_sim::join_seq::{
    Joined,
    residency::{
        Journal, Kind, Owner, Source,
        table::{Location, SourceEntity},
    },
};

impl SpecializedSource<'_> {
    fn id(&self) -> u32 {
        match self {
            Self::Player(player) => player.entity_id as u32,
            Self::Guest(guest) => guest.entity_id as u32,
            Self::Hook(hook) => hook.id(),
        }
    }
}

impl crate::game::ServerGame<'_> {
    pub(in crate::game::server_side) fn current_player_resident_location(
        &self,
    ) -> Option<Location> {
        match self.server.lan_acting {
            Some(conn) => self.command_guest(conn)?.residency.location(),
            None => self.common.resident_entities.local_player.location(),
        }
    }

    /// Engine UUID consumers retain a specialized owner's immutable identity
    /// even through a real loan. Public I access applies its policy beforehand.
    pub(in crate::game) fn known_resident_entity_uuid_at(
        &self,
        dimension: mod_api::DimensionKey,
        id: u32,
    ) -> Option<[i32; 4]> {
        let mut table = self.common.resident_entities.table.borrow_mut();
        let (at, _, details) =
            table.server_admitted_identity(id, |at| self.locate_server_resident_owner(at))?;
        let uuid = details.known_uuid;
        (at.owner.dimension == dimension).then_some(uuid).flatten()
    }

    pub fn resident_target_id(&self, target: Target) -> Option<u32> {
        match target {
            Target::Player => match self.server.lan_acting {
                Some(conn) => self
                    .command_guest(conn)?
                    .residency
                    .identity()
                    .map(|identity| identity.id.get()),
                None => self
                    .common
                    .resident_entities
                    .local_player
                    .identity()
                    .map(|identity| identity.id.get()),
            },
            Target::Guest(conn) => self
                .command_guest(conn)?
                .residency
                .identity()
                .map(|identity| identity.id.get()),
            _ => sunrise_gameplay_common::game::commands_common::target_id(target),
        }
    }

    pub(in crate::game::server_side) fn acting_player_join(
        &self,
    ) -> sunrise_sim::join_seq::JoinSeq {
        self.server
            .lan_acting
            .and_then(|conn| self.command_guest(conn))
            .map_or_else(
                || self.common.resident_entities.local_player.join(),
                |guest| guest.residency.join(),
            )
    }

    pub fn install_fishing_hook(&mut self, mut hook: Hook) {
        hook.caster = self.server.lan_acting;
        hook.caster_generation = self.acting_player_join();
        self.common.hook.discard();
        self.bind_current_hook_residency();
        self.common.hook.replace(hook);
    }

    pub(in crate::game::server_side) fn bind_current_hook_residency(&mut self) {
        let journal = self.common.resident_entities.table.borrow().producer();
        let (owner, source) = self
            .common
            .hook
            .resident_location()
            .map(|at| (at.owner, at.source))
            .unwrap_or_else(|| {
                let dimension = self
                    .current_player_resident_location()
                    .map_or(self.common.dim.key(), |at| at.owner.dimension);
                (
                    Owner {
                        dimension,
                        kind: Kind::Hook,
                    },
                    Source::Hook {
                        caster: self.server.lan_acting,
                    },
                )
            });
        self.common.hook.bind_residency_specialized(
            &journal,
            owner,
            |hook| (hook.id(), hook.join_seq()),
            source,
            None,
            Some(|hook| Some(hook.uuid)),
        );
    }

    pub(in crate::game::server_side) fn bind_specialized_resident_owners(
        &mut self,
        journal: &Journal,
    ) {
        self.common.resident_entities.local_player.rebind(journal);
        let body_id = self
            .common
            .player
            .as_ref()
            .map(|player| player.entity_id as u32);
        if self.server.lan_acting.is_some()
            || self
                .common
                .resident_entities
                .local_player
                .identity()
                .is_some_and(|identity| Some(identity.id.get()) != body_id)
        {
            self.common.resident_entities.local_player.extract();
        }
        self.bind_current_hook_residency();
        if self.common.hook.resident_location().is_some_and(|at| {
            at.source
                != (Source::Hook {
                    caster: self.server.lan_acting,
                })
        }) {
            self.common.hook.begin_loan();
        }
        if let Some(host) = &mut self.server.lan {
            host.hub
                .guests
                .bind_residency(journal, |id| super::Dim::from_id(id).key());
            for &conn in &self.server.lan_guest_swaps {
                if let Some(index) = host.hub.guests.connection_index(conn) {
                    let guest = &host.hub.guests[index];
                    if self.server.lan_acting != Some(conn)
                        || body_id != Some(guest.entity_id as u32)
                    {
                        guest.residency.extract();
                    }
                }
            }
            for (&conn, play) in &mut host.play {
                let Some(index) = host.hub.guests.connection_index(conn) else {
                    continue;
                };
                let guest = &host.hub.guests[index];
                let (owner, source) = play
                    .hook
                    .resident_location()
                    .map(|at| (at.owner, at.source))
                    .unwrap_or((
                        Owner {
                            dimension: super::Dim::from_id(&guest.dimension).key(),
                            kind: Kind::Hook,
                        },
                        Source::Hook { caster: Some(conn) },
                    ));
                play.hook.bind_residency_specialized(
                    journal,
                    owner,
                    |hook| (hook.id(), hook.join_seq()),
                    source,
                    None,
                    Some(|hook| Some(hook.uuid)),
                );
            }
        }
    }

    pub(in crate::game::server_side) fn with_specialized_resident_slot<R>(
        &mut self,
        at: Location,
        visit: impl FnOnce(&mut sunrise_mod_runtime::attachments::AttachmentLifetime) -> R,
    ) -> super::entity_residency::SlotAccess<R> {
        use super::entity_residency::SlotAccess;
        use sunrise_mod_runtime::attachments::HasAttachmentLifetime;
        match at.source {
            Source::LocalPlayer => SlotAccess::Done(visit(
                self.common.player_state.attachments.attachment_lifetime(),
            )),
            Source::Guest(conn) => {
                if self.server.lan_acting == Some(conn) {
                    return SlotAccess::Done(visit(
                        self.common.player_state.attachments.attachment_lifetime(),
                    ));
                }
                let Some(attachments) = self
                    .server
                    .lan
                    .as_mut()
                    .and_then(|host| host.hub.guests.attachments_mut(conn))
                else {
                    return SlotAccess::Missing;
                };
                SlotAccess::Done(visit(attachments.attachment_lifetime()))
            }
            Source::Hook { caster } => {
                let hook = if caster == self.server.lan_acting {
                    self.common.hook.as_mut()
                } else {
                    caster.and_then(|conn| {
                        self.server.lan.as_mut()?.play.get_mut(&conn)?.hook.as_mut()
                    })
                };
                let Some(hook) = hook else {
                    return SlotAccess::Missing;
                };
                SlotAccess::Done(visit(hook.attachment_lifetime()))
            }
            Source::DragonPart(_) | Source::Ordinary => SlotAccess::Missing,
        }
    }
}

impl crate::game::ServerGame<'_> {
    pub(in crate::game) fn specialized_resident_source(
        &self,
        at: Location,
    ) -> Option<SourceEntity<(SpecializedSource<'_>, Target)>> {
        let (source, id, target) = match (at.owner.kind, at.source) {
            (Kind::Player, Source::LocalPlayer) => {
                if sunrise_gameplay_common::game::Side::lan_acting(&self.server).is_some()
                    || self.common.resident_entities.local_player.location() != Some(at)
                {
                    return None;
                }
                let player = self.common.player.as_ref()?;
                if self
                    .common
                    .resident_entities
                    .local_player
                    .identity()?
                    .id
                    .get()
                    != player.entity_id as u32
                {
                    return None;
                }
                (
                    SpecializedSource::Player(player),
                    player.entity_id as u32,
                    Target::Player,
                )
            }
            (Kind::Player, Source::Guest(conn)) => {
                let host = self.server.lan.as_ref()?;
                let guest = &host.hub.guests[host.hub.guests.connection_index(conn)?];
                if guest.residency.location() != Some(at) || guest.released {
                    return None;
                }
                if sunrise_gameplay_common::game::Side::lan_acting(&self.server) == Some(conn) {
                    let player = self.common.player.as_ref()?;
                    if player.entity_id != guest.entity_id {
                        return None;
                    }
                    (
                        SpecializedSource::Player(player),
                        guest.entity_id as u32,
                        Target::Player,
                    )
                } else {
                    if sunrise_gameplay_common::game::Side::lan_guest_swaps(&self.server)
                        .contains(&conn)
                    {
                        return None;
                    }
                    (
                        SpecializedSource::Guest(guest),
                        guest.entity_id as u32,
                        Target::Guest(conn),
                    )
                }
            }
            (Kind::Hook, Source::Hook { caster }) => {
                let hook = match caster {
                    None => {
                        if sunrise_gameplay_common::game::Side::lan_acting(&self.server).is_some() {
                            return None;
                        }
                        let hook = self.common.hook.as_ref()?;
                        if hook.caster_generation
                            != self.common.resident_entities.local_player.join()
                        {
                            return None;
                        }
                        hook
                    }
                    Some(conn) => {
                        let host = self.server.lan.as_ref()?;
                        let guest = host
                            .hub
                            .guests
                            .connection_index(conn)
                            .map(|index| &host.hub.guests[index]);
                        let hook = if sunrise_gameplay_common::game::Side::lan_acting(&self.server)
                            == Some(conn)
                        {
                            self.common.hook.as_ref()?
                        } else {
                            if sunrise_gameplay_common::game::Side::lan_guest_swaps(&self.server)
                                .contains(&conn)
                            {
                                return None;
                            }
                            host.play.get(&conn)?.hook.as_ref()?
                        };
                        if guest
                            .is_some_and(|guest| hook.caster_generation != guest.residency.join())
                        {
                            return None;
                        }
                        hook
                    }
                };
                if hook.join_seq() != at.join || hook.caster != caster {
                    return None;
                }
                (
                    SpecializedSource::Hook(hook),
                    hook.id(),
                    Target::Hook(hook.id()),
                )
            }
            _ => return None,
        };
        Some(SourceEntity {
            id,
            value: (source, target),
        })
    }

    pub(in crate::game) fn locate_specialized_resident_owner(
        &self,
        at: Location,
    ) -> Option<SourceEntity<(Target, bool)>> {
        let source = self.specialized_resident_source(at)?;
        let (_, target) = source.value;
        Some(SourceEntity {
            id: source.value.0.id(),
            value: (
                target,
                self.resident_source_target_visible(at.owner.dimension, target),
            ),
        })
    }
}

use sunrise_gameplay_common::game::GameGameExt3 as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;
