//! Configured spawner NBT, decoded once and held until the spawn pass finishes.
use super::{Mob, Tag, entity_nbt};
use sunrise_core::fast_hash::FxHashSet;
use sunrise_gameplay_common::game::general::GeneralEntityRecord;

pub struct SpawnTree(Vec<Member>);

struct Member {
    node: Node,
    tag: Tag,
    uuid: [i32; 4],
    parent: Option<usize>,
    resident: bool,
}

enum Node {
    Mob(Box<Mob>),
    General(usize, GeneralEntityRecord),
}

fn spawn_tags(tag: &Tag) -> impl Iterator<Item = String> + '_ {
    let tags = match tag.get("Tags") {
        Some(Tag::List(tags)) => tags.as_slice(),
        _ => &[],
    };
    tags.iter().filter_map(|tag| match tag {
        Tag::String(s) => Some(s.to_string_lossy()),
        _ => None,
    })
}

impl SpawnTree {
    /// `EntityType.loadPassengersRecursive`: read each supported member before
    /// its descendants. A record that cannot be decoded contributes no branch.
    pub(in crate::game) fn decode(game: &crate::game::ServerGame<'_>, tag: Tag) -> Option<Self> {
        let mut members = Vec::new();
        let mut todo = vec![(None, tag)];
        while let Some((parent, mut tag)) = todo.pop() {
            let Tag::Compound(raw) = &mut tag else {
                continue;
            };
            let riders = raw.remove(&"Passengers".into());
            let Some(kind) = entity_nbt::kind(&tag) else {
                continue;
            };
            let (node, uuid) = if let Some(mob_type) = game.res.mob_types().get(&kind) {
                crate::game::ServerGame::normalize_loaded_mob_tags(&mut tag);
                let (random, fresh_uuid) = sunrise_sim::mobs::living_random();
                let mut mob = entity_nbt::mob_from_nbt_deferred(
                    &tag,
                    mob_type.clone(),
                    game.pack.assets.registry,
                    random,
                );
                entity_nbt::read_mob_variants(&mut mob, &tag, game.world_variants());
                mob.extra.spawn_tags = spawn_tags(&tag).collect();
                (
                    Node::Mob(Box::new(mob)),
                    entity_nbt::uuid(&tag).unwrap_or(fresh_uuid),
                )
            } else if let Some((slot, record)) =
                crate::game::ServerGame::decode_general_spawn_passenger(&tag, &kind)
            {
                let uuid = record.uuid;
                (Node::General(slot, record), uuid)
            } else {
                continue;
            };
            let index = members.len();
            members.push(Member {
                node,
                tag,
                uuid,
                parent,
                resident: false,
            });
            if let Some(Tag::List(riders)) = riders {
                todo.extend(riders.into_iter().rev().map(|rider| (Some(index), rider)));
            }
        }
        (!members.is_empty()).then_some(Self(members))
    }

    pub(in crate::game) fn root_mob(&mut self) -> Option<&mut Mob> {
        match &mut self.0[0].node {
            Node::Mob(mob) => Some(mob),
            Node::General(..) => None,
        }
    }

    pub(in crate::game) fn root_mob_mut(&mut self) -> Option<(&mut Mob, &mut Tag)> {
        let root = &mut self.0[0];
        match &mut root.node {
            Node::Mob(mob) => Some((mob, &mut root.tag)),
            Node::General(..) => None,
        }
    }

    pub(in crate::game::server_side) fn root_uuid(&self) -> [i32; 4] {
        self.0[0].uuid
    }

    /// `ServerLevel.tryAddFreshEntityWithPassengers`: compare the whole tree
    /// to the UUIDs already loaded before this attempt. Then offer every node
    /// in `Entity.getSelfAndPassengers` order to `addEntityUuid`, so only the
    /// first occurrence of an internal duplicate joins the level.
    pub(in crate::game::server_side) fn admit(
        &mut self,
        pending: &mut FxHashSet<[i32; 4]>,
        loaded: impl Fn([i32; 4]) -> bool,
    ) -> bool {
        if self
            .0
            .iter()
            .any(|member| pending.contains(&member.uuid) || loaded(member.uuid))
        {
            return false;
        }
        for member in &mut self.0 {
            member.resident = pending.insert(member.uuid);
        }
        true
    }

    // Only mobs of the supported passenger families block construction or
    // null-source collision, and only mob types can be spawner roots here.
    pub(in crate::game::server_side) fn resident_mobs(&self) -> impl Iterator<Item = &Mob> {
        self.0.iter().filter_map(|m| match (&m.node, m.resident) {
            (Node::Mob(mob), true) => Some(mob.as_ref()),
            _ => None,
        })
    }

    pub(in crate::game) fn install(self, game: &mut crate::game::ServerGame<'_>) -> u32 {
        let mut ids = Vec::with_capacity(self.0.len());
        for Member {
            node,
            mut tag,
            uuid,
            parent,
            resident,
        } in self.0
        {
            let mount = parent.and_then(|i| ids[i]);
            let id = match node {
                Node::Mob(mut mob) => {
                    game.server.next_entity += 1;
                    mob.id = game.server.next_entity;
                    mob.attach_mod_parts();
                    let tags = std::mem::take(&mut mob.extra.spawn_tags);
                    if !spawn_tags(&tag).eq(tags.iter().cloned())
                        && let Tag::Compound(raw) = &mut tag
                    {
                        if tags.is_empty() {
                            raw.remove(&"Tags".into());
                        } else {
                            raw.insert(
                                "Tags".into(),
                                Tag::List(
                                    tags.into_iter()
                                        .map(|tag| Tag::String(tag.as_str().into()))
                                        .collect(),
                                ),
                            );
                        }
                    }
                    let id = game.install_loaded_mob(
                        (*mob, uuid),
                        tag,
                        mount,
                        None,
                        false,
                        Some(resident),
                    );
                    if let Some(vehicle) = mount.and_then(|id| game.mob_object_mut(id)) {
                        vehicle.extra.rider.get_or_insert(id);
                    }
                    Some(id)
                }
                Node::General(slot, record) if resident => {
                    Some(game.insert_general_row(slot, record, &tag, mount, None))
                }
                // General rows have no off-level store. Still consider the
                // duplicate's descendants independently, at their loaded pose.
                Node::General(..) => None,
            };
            ids.push(id);
        }
        ids[0].expect("an admitted tree's first member joins the level")
    }
}

impl crate::game::ServerGame<'_> {
    pub(in crate::game) fn install_generated_mobs(
        &mut self,
        pack: Vec<sunrise_sim::vanillagen::GeneratedMob>,
    ) {
        let mut ids = Vec::new();
        for sunrise_sim::vanillagen::GeneratedMob {
            mut body,
            uuid,
            tag,
        } in pack
        {
            let mount = body.extra.mount.and_then(|old| {
                ids.iter()
                    .find_map(|&(provisional, real)| (provisional == old).then_some(real))
            });
            let provisional = body.id;
            body.extra.rider = None;
            self.server.next_entity += 1;
            body.id = self.server.next_entity;
            let id =
                self.install_loaded_mob((body, uuid), Tag::Compound(tag), mount, None, false, None);
            ids.push((provisional, id));
            if let Some(vehicle) = mount.and_then(|id| self.mob_object_mut(id)) {
                vehicle.extra.rider.get_or_insert(id);
            }
        }
    }
}
