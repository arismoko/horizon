//! A player's attack strength for native server mods, through a `PlayerRef`:
//! `Player.getAttackStrengthScale(partial)`, the charge `Player.attack` reads
//! at its start (`attack_entity` fires before that, and before the swing
//! resets the ticker). Reads return None for a stale handle or a player who
//! isn't online here; a resting guest reads its saved body.
use crate::game::ServerGame;
use crate::game::server_side::lan_actions_server::Struck;
use mod_api::__private::game::Fields;
use mod_api::{ActionRefused, AttackRefused, EntityRef, PlayerRef};
use sunrise_gameplay_common::game::native_server::{Acting, PlayerRead};
use sunrise_sim::attack_strength::{BASE_ATTACK_SPEED, delay};

impl ServerGame<'_> {
    /// `Player.getAttackStrengthScale(partial)` for `player`.
    /// Returns `None` for a non-finite partial tick; finite partials are not restricted.
    pub(in crate::game) fn player_attack_strength_scale(
        &self,
        player: PlayerRef,
        partial: f32,
    ) -> Option<f32> {
        if !partial.is_finite() {
            return None;
        }
        let read = self.player_handle_read(player)?;
        Some(match read {
            PlayerRead::Acting { .. } => return self.attack_strength_scale_at(partial),
            PlayerRead::Guest(guest) => {
                let (ticks, speed) = self.guest_attack_charge(guest)?;
                ((ticks as f32 + partial) / delay(speed)).clamp(0.0, 1.0)
            }
        })
    }

    /// `Player.getCurrentItemAttackStrengthDelay()` for `player` on the server (#5691).
    pub(in crate::game) fn player_current_item_attack_strength_delay(
        &self,
        player: PlayerRef,
    ) -> Option<f32> {
        Some(delay(match self.player_handle_read(player)? {
            PlayerRead::Acting { .. } => self.main_hand_attack_speed(),
            PlayerRead::Guest(guest) => self.guest_attack_charge(guest)?.1,
        }))
    }

    /// `LivingEntity.attackStrengthTicker` of a server's player: the one in place, or a guest's
    /// between its turns.
    pub(in crate::game) fn player_attack_strength_ticker(&self, player: PlayerRef) -> Option<i32> {
        if !self.common.is_logical_server() {
            return None;
        }
        Some(match self.player_handle_read(player)? {
            PlayerRead::Acting { .. } => self.server.attack_ticks,
            PlayerRead::Guest(guest) => self.guest_attack_charge(guest)?.0,
        })
    }

    /// Sets a server's player's `LivingEntity.attackStrengthTicker`, dead or alive, as given.
    pub(in crate::game) fn player_set_attack_strength_ticker(
        &mut self,
        player: PlayerRef,
        ticks: i32,
    ) -> bool {
        self.native_as_resident_player(player, |game| game.server.attack_ticks = ticks)
    }

    /// `Player.attack(target)` for a connected or fake player, from its start and without the
    /// packet's checks or swing ([`ServerGame::guest_player_attack`]). True once it ran past
    /// `cannotAttack`.
    pub(in crate::game) fn player_attack(
        &mut self,
        player: PlayerRef,
        target: EntityRef,
    ) -> Result<bool, AttackRefused> {
        let epoch = self.common.native_lifecycle.epoch();
        let me = player.entity();
        if !self.common.is_logical_server()
            || epoch != Some(me.epoch())
            || epoch != Some(target.epoch())
        {
            return Err(AttackRefused::Stale);
        }
        let found = self.server.lan.as_ref().and_then(|host| {
            let gi = host
                .hub
                .guests
                .identity_index(player.fields().uuid, me.id())?;
            let guest = host.hub.guests.get(gi)?;
            (guest.placed && guest.left_reason.is_empty()).then_some((gi, guest.conn))
        });
        let Some((gi, conn)) = found else {
            // Not in the hub's list as itself: the player whose turn runs now, or gone.
            return Err(match self.player_handle_read(player) {
                Some(_) => AttackRefused::Busy,
                None => AttackRefused::Stale,
            });
        };
        if !self.server.lan_guest_swaps.is_empty()
            || !(self.common.transactions.is_empty() || self.in_level_tick_boundary_alone())
            || self.server.fake_action.0.acting()
        {
            return Err(AttackRefused::Busy);
        }
        let (Some(from), Some(to)) = (self.entity_level(me), self.entity_level(target)) else {
            return Err(AttackRefused::Stale);
        };
        if from != to || !self.guest_in_place(gi) {
            return Err(AttackRefused::OtherLevel);
        }
        if target.id() == me.id() {
            return Err(AttackRefused::Unsupported);
        }
        let id = i32::try_from(target.id()).map_err(|_| AttackRefused::Stale)?;
        // As a fake player's `perform_action`, beside a level tick listener's transaction.
        let struck = self.beside_listener_journal(|game| {
            let _acting = Acting::start_unswung(&game.server.fake_action.0);
            game.guest_player_attack(conn, gi, id, false)
        });
        match struck {
            Ok(struck) => Ok(struck != Struck::Stopped),
            // No body Sunrise strikes along the player's aim.
            Err(ActionRefused::InvalidTarget) => Err(AttackRefused::Unsupported),
            Err(_) => Err(AttackRefused::Stale),
        }
    }

    /// A guest between turns: its attack ticker and its attack speed with the main-hand item of
    /// its last tick.
    fn guest_attack_charge(&self, guest: &sunrise_sim::lan::guest::Guest) -> Option<(i32, f64)> {
        let (ticks, item) = self
            .server
            .lan
            .as_ref()?
            .play
            .get(&guest.conn)
            .map_or((0, None), |play| (play.attack_ticks(), play.attack_item()));
        Some((ticks, guest_attack_speed(self, guest, item)))
    }
}

/// `Game.main_hand_attack_speed` for a guest between turns: its saved body's
/// attack speed base, the main-hand item of its last tick, and its effects'
/// modifiers.
fn guest_attack_speed(
    game: &crate::game::ServerGame<'_>,
    guest: &sunrise_sim::lan::guest::Guest,
    item: Option<&str>,
) -> f64 {
    let base = guest
        .motion
        .as_ref()
        .and_then(|player| {
            let key = player.attributes.key("minecraft:attack_speed")?;
            let handle = player.attributes.lookup(Some(&key))?;
            player
                .attributes
                .instance(&handle)
                .ok()
                .map(|value| value.base_value())
        })
        .unwrap_or(BASE_ATTACK_SPEED);
    let value = base
        + item
            .and_then(|id| game.items().get(id))
            .map_or(0.0, |info| f64::from(info.attack_speed));
    game.sanitize_attack_speed(
        guest
            .effects
            .modifiers()
            .into_iter()
            .filter(|modifier| modifier.0 == "minecraft:attack_speed")
            .fold(value, |value, modifier| value * (1.0 + modifier.2)),
    )
}

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
use sunrise_gameplay_common::game::tables::GameItems as _;

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::ServerGame;
    use mod_api::EntityRef;
    use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
    use sunrise_items::items::ItemStack;

    /// A zombie `id` standing at `at`, and its handle.
    pub(in crate::game) fn zombie(game: &mut ServerGame<'_>, id: u32, at: [f64; 3]) -> EntityRef {
        let kind = game
            .common
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let [x, y, z] = at;
        game.common.mobs.push(sunrise_sim::mobs::Mob::new(
            id,
            kind,
            sunrise_core::geometry::Vec3::new(x, y, z),
            0.0,
        ));
        EntityRef::new(game.common.native_lifecycle.epoch().unwrap(), id)
    }

    fn mob<'a>(game: &'a ServerGame<'_>, id: u32) -> &'a sunrise_sim::mobs::Mob {
        game.common.mobs.iter().find(|m| m.id == id).unwrap()
    }

    /// A connected player's `attack` on a zombie, holding a Knockback II, Fire Aspect diamond
    /// sword while falling, at full charge: `Player.attack`'s critical hit (7 x 1.5 through the
    /// zombie's 2 armour), Fire Aspect's 4 seconds of fire, the push along the player's facing
    /// (+z at yaw 0), one point of durability, and the ticker reset, with no swing, which is
    /// the packet's.
    #[test]
    #[ignore = "daily"]
    fn a_mods_attack_runs_player_attack_with_its_crit_enchantments_and_durability_unswung() {
        use crate::game::native_entity_refs::tests::with_game;
        use crate::game::native_server::fake_action_tests::fake;
        use sunrise_gameplay_common::game::test_fixtures::run_fresh_cases;
        if run_fresh_cases(
            "game::server_side::native_player_attack::tests::a_mods_attack_runs_player_attack_with_its_crit_enchantments_and_durability_unswung",
            "SUNRISE_PLAYER_ATTACK_CASE",
            &["attack"],
        ) {
            return;
        }
        with_game(|game| {
            let (_, _, conn) = fake(game, None, 0);
            let player = game.player(1).unwrap();
            let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
            sword.enchant("minecraft:knockback", 2);
            sword.enchant("minecraft:fire_aspect", 1);
            let host = game.server.lan.as_mut().unwrap();
            host.hub.update_guest(conn, |g| {
                g.inventory[g.selected] = Some(sword);
                g.on_ground = false;
            });
            game.as_guest(conn, |g| {
                g.put_equipment_modifiers();
                g.common.player.as_mut().unwrap().fall_distance = 1.0;
            })
            .unwrap();
            let target = zombie(game, 5000, [0.5, 64.0, 2.0]);
            assert!(player.set_attack_strength_ticker(game, 100));
            assert_eq!(player.attack(game, target), Ok(true));
            let damage = 7.0_f32 * 1.5;
            let armor = (2.0_f32 - damage / 2.0).max(2.0 * 0.2);
            let zombie = mob(game, 5000);
            assert_eq!(zombie.health, 20.0 - damage * (1.0 - armor / 25.0));
            assert_eq!(zombie.fire_ticks, 80);
            assert!(zombie.velocity.z > 0.0 && zombie.velocity.x == 0.0);
            let guest = game.server.lan.as_ref().unwrap().hub.guest(conn).unwrap();
            assert_eq!(guest.inventory[guest.selected].as_ref().unwrap().damage, 1);
            assert_eq!(player.attack_strength_ticker(game), Some(0));
            assert_eq!(
                game.as_guest(conn, |g| g.common.swing.current.is_none()),
                Some(true)
            );
        });
    }

    /// One gesture striking two zombies: the ticker read once, set back to that value before
    /// the second `attack`, gives both the same partial charge, `7 * (0.2 + c * c * 0.8)` with
    /// `c = (5 + 0.5) / delay` for a diamond sword; each attack resets it. Any value is kept as
    /// given, a negative one too.
    #[test]
    #[ignore = "daily"]
    fn the_ticker_set_back_between_two_attacks_gives_both_the_same_charge() {
        use crate::game::native_entity_refs::tests::with_game;
        use crate::game::native_server::fake_action_tests::fake;
        use sunrise_gameplay_common::game::test_fixtures::run_fresh_cases;
        if run_fresh_cases(
            "game::server_side::native_player_attack::tests::the_ticker_set_back_between_two_attacks_gives_both_the_same_charge",
            "SUNRISE_PLAYER_TICKER_CASE",
            &["ticker"],
        ) {
            return;
        }
        with_game(|game| {
            let (_, _, conn) = fake(game, Some("minecraft:diamond_sword"), 0);
            let player = game.player(1).unwrap();
            game.as_guest(conn, |g| g.put_equipment_modifiers())
                .unwrap();
            assert!(player.set_attack_strength_ticker(game, -7));
            assert_eq!(player.attack_strength_ticker(game), Some(-7));
            assert!(player.set_attack_strength_ticker(game, 5));
            let swing = player.attack_strength_ticker(game).unwrap();
            assert_eq!(swing, 5);
            let first = zombie(game, 5000, [2.0, 64.0, 0.5]);
            let second = zombie(game, 5001, [-1.5, 64.0, 0.5]);
            assert_eq!(player.attack(game, first), Ok(true));
            assert_eq!(player.attack_strength_ticker(game), Some(0));
            assert!(player.set_attack_strength_ticker(game, swing));
            assert_eq!(player.attack(game, second), Ok(true));
            // Diamond sword: 4 + (-2.4f32) attack speed; Player.attack samples the ticker at 0.5.
            let delay = (1.0 / (4.0 + f64::from(-2.4f32)) * 20.0) as f32;
            let charge = (5.0 + 0.5) / delay;
            let damage = 7.0 * (0.2 + charge * charge * 0.8);
            let armor = (2.0_f32 - damage / 2.0).max(2.0 * 0.2);
            let hurt = 20.0 - damage * (1.0 - armor / 25.0);
            assert_eq!(
                (mob(game, 5000).health, mob(game, 5001).health),
                (hurt, hurt)
            );
        });
    }
}
