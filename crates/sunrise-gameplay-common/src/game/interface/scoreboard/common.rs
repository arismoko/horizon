use crate::game::commands_common::Target;
use sunrise_game_data::scoreboard::{self as sb, Scoreboard};
use sunrise_sim::{lan::playerdata::ints_uuid, save::uuid_text};

/// What `ServerPlayer` last recorded for the criteria that follow the player.
#[derive(Default)]
pub struct Recorded {
    /// `lastRecordedHealthAndAbsorption` (compared as a float).
    pub health_and_absorption: Option<f32>,
    pub(in crate::game) food: Option<i32>,
    pub(in crate::game) air: Option<i32>,
    pub(in crate::game) armor: Option<i32>,
    pub(in crate::game) xp: Option<i32>,
    pub(in crate::game) level: Option<i32>,
}
impl Recorded {
    /// The food, air, armor, xp and level criteria whose value differs from the last
    /// recorded one, recorded as they are returned (`ServerPlayer.doTick`).
    pub fn changed_scores(
        &mut self,
        food: i32,
        air: i32,
        armor: i32,
        xp: i32,
        level: i32,
    ) -> Vec<(&'static str, i32)> {
        let mut out = Vec::new();
        for (criteria, last, value) in [
            (sb::CRITERIA_FOOD, &mut self.food, food),
            (sb::CRITERIA_AIR, &mut self.air, air),
            (sb::CRITERIA_ARMOR, &mut self.armor, armor),
            (sb::CRITERIA_XP, &mut self.xp, xp),
            (sb::CRITERIA_LEVEL, &mut self.level, level),
        ] {
            if *last != Some(value) {
                *last = Some(value);
                out.push((criteria, value));
            }
        }
        out
    }
}
#[derive(Default)]
pub struct ScoreState {
    pub board: Scoreboard,
    /// `/data ... storage`.
    pub storage: sunrise_sim::command_storage::CommandStorage,
    pub stopwatches: sunrise_sim::stopwatches::Stopwatches,
    pub loaded: bool,
    pub recorded: Recorded,
}

/// `TamableAnimal.getTeam`: a pet's own team, else, when it is tamed, its root
/// owner's, else none. An untamed pet has no owner to take one from.
pub fn pet_team<'a>(board: &'a Scoreboard, own: &str, owner: Option<&str>) -> Option<&'a sb::Team> {
    board
        .players_team(own)
        .or_else(|| board.players_team(owner?))
}
/// A tamed pet's owner (`TamableAnimal.getOwnerReference` while `isTame`).
pub fn tame_owner(m: &sunrise_sim::mobs::Mob) -> Option<[i32; 4]> {
    m.extra.tame.then_some(m.extra.owner).flatten()
}
/// `TamableAnimal.getRootOwner`: the owner, or that owner's own root owner
/// while it is a tamed pet itself. `pet_owner` is the owner of the tamed pet
/// with a UUID, if it has one.
pub fn root_owner(
    mut owner: [i32; 4],
    pet_owner: impl Fn([i32; 4]) -> Option<[i32; 4]>,
) -> [i32; 4] {
    // shortcut: vanilla has no cycle check, this bound stops a hand-made loop.
    for _ in 0..64 {
        match pet_owner(owner) {
            Some(next) => owner = next,
            None => break,
        }
    }
    owner
}

pub trait CommonGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// `Entity.getScoreboardName`: another player's profile name
    /// (`Player.getScoreboardName`), any other entity's UUID.
    fn scoreboard_name(&self, id: u32, uuid: [i32; 4]) -> String {
        crate::game::side_ops::player::SideOps::side_remote_player_name(self.side_state(), id)
            .map_or_else(|| uuid_text(uuid), str::to_owned)
    }

    /// `Entity.getTeam`: the team holding this entity's scoreboard name (its UUID).
    fn entity_team<'__loan>(&'__loan self, id: u32) -> Option<&'__loan sb::Team>
    where
        '__sg: '__loan,
    {
        if self.score.board.teams.is_empty() {
            return None;
        }
        let uuid = *self.persist.uuids.get(&id)?;
        let own = self.scoreboard_name(id, uuid);
        self.score.board.players_team(&own)
    }

    /// [`mob_team`](Self::mob_team) of a mob at hand, without looking it up.
    fn mob_team_of<'__loan>(&'__loan self, m: &sunrise_sim::mobs::Mob) -> Option<&'__loan sb::Team>
    where
        '__sg: '__loan,
    {
        if self.score.board.teams.is_empty() {
            return None;
        }
        self.team_with_owner(m.id, || tame_owner(m))
    }

    /// A mob's own team, else its tamed owner's (`owner` is asked only then).
    fn team_with_owner<'__loan>(
        &'__loan self,
        id: u32,
        owner: impl FnOnce() -> Option<[i32; 4]>,
    ) -> Option<&'__loan sb::Team>
    where
        '__sg: '__loan,
    {
        let uuid = *self.persist.uuids.get(&id)?;
        let own = self.scoreboard_name(id, uuid);
        if let Some(team) = self.score.board.players_team(&own) {
            return Some(team);
        }
        let owner = owner().and_then(|o| self.root_owner_name(o));
        pet_team(&self.score.board, &own, owner.as_deref())
    }

    /// `TamableAnimal.getRootOwner().getScoreboardName`: a pet's owner, through
    /// owners that are tamed pets themselves.
    fn root_owner_name(&self, owner: [i32; 4]) -> Option<String> {
        let pet_owner = |uuid: [i32; 4]| {
            let Target::Mob(pet) = self.target_by_uuid(ints_uuid(uuid))? else {
                return None;
            };
            self.mobs
                .iter()
                .find(|m| m.id == pet && m.extra.tame)?
                .extra
                .owner
        };
        let root = root_owner(owner, pet_owner);
        Some(match self.target_by_uuid(ints_uuid(root))? {
            Target::Player => self.acting_player_name().to_owned(),
            Target::Guest(conn) => self.command_guest(conn)?.name.clone(),
            Target::Mob(id) => self.scoreboard_name(id, root),
            _ => uuid_text(root),
        })
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> CommonGameExt1<'__sg> for G {}

use crate::game::commands_common::CommandsCommonGameExt2 as _;
