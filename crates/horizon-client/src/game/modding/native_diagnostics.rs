//! What a server mod's diagnostics read (spark, PacketEvents' counters): the tick times, the
//! time each mob type takes, and each player's packets. Nothing is kept unless a mod asks.
use super::{Game, native_server::PlayerRead};
use horizon_client::lan::packet_counts;
use mod_api::{MobTickCost, PacketCount, PlayerRef};

impl Game<'_> {
    /// `MinecraftServer.getTickTimesNanos`.
    pub(super) fn tick_times(&self) -> Vec<u64> {
        self.clock.tick_times()
    }
    /// The mod's own mob tick sampling (`tick_profile::set_mod_sampling`).
    pub(super) fn mob_cost_sampling(&mut self, on: bool) -> bool {
        if !self.is_logical_server() {
            return false;
        }
        super::tick_profile::set_mod_sampling(on);
        true
    }
    /// What the mod's sampling summed since the last call, costliest first.
    pub(super) fn take_mob_costs(&mut self) -> Vec<MobTickCost> {
        if !self.is_logical_server() {
            return Vec::new();
        }
        super::tick_profile::take_mod()
            .into_iter()
            .map(|c| MobTickCost::new(c.dimension.to_owned(), c.kind, c.nanos, c.mob_ticks))
            .collect()
    }
    /// The server's packet counting switch (`packet_counts::set_counting`).
    pub(super) fn count_packets(&mut self, on: bool) -> bool {
        if !self.is_logical_server() {
            return false;
        }
        packet_counts::set_counting(on);
        true
    }
    /// `PlayerList.getViewDistance`.
    pub(super) fn server_view_distance(&self) -> Option<i32> {
        Some(self.lan.as_ref()?.hub.config.view_distance)
    }
    /// `PlayerList.getSimulationDistance`.
    pub(super) fn server_simulation_distance(&self) -> Option<i32> {
        Some(self.lan.as_ref()?.hub.config.simulation_distance)
    }
    /// `PlayerList.setViewDistance` on a dedicated server; the tickets read the settings.
    pub(super) fn server_set_view_distance(&mut self, distance: i32) -> bool {
        if !(2..=32).contains(&distance) || self.options.dedicated.is_none() || self.lan.is_none()
        {
            return false;
        }
        self.set_view_distance(distance);
        self.settings.render_distance = distance;
        true
    }
    /// `PlayerList.setSimulationDistance` on a dedicated server.
    pub(super) fn server_set_simulation_distance(&mut self, distance: i32) -> bool {
        if !(2..=32).contains(&distance) || self.options.dedicated.is_none() || self.lan.is_none()
        {
            return false;
        }
        self.set_simulation_distance(distance);
        self.settings.simulation_distance = distance;
        true
    }
    /// A connected player's packet counts since the last take.
    pub(super) fn player_take_packet_counts(
        &mut self,
        player: PlayerRef,
    ) -> Option<Vec<PacketCount>> {
        if !self.is_logical_server() {
            return None;
        }
        let conn = match self.player_handle_read(player)? {
            PlayerRead::Acting { .. } => self.lan_acting?,
            PlayerRead::Guest(g) => g.conn,
        };
        let guest = self.lan.as_ref()?.hub.guest(conn)?;
        if guest.is_fake() {
            return None;
        }
        Some(
            guest
                .sender
                .packet_counts
                .take()
                .into_iter()
                .map(|(outbound, id, c)| PacketCount::new(outbound, id, c.packets, c.bytes))
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_guest_turn_tests::with_guests;
    use horizon_client::lan::packet_counts;

    /// A guest's sent packets are counted while counting is on, taken once, and the tick
    /// times are the clock's.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_players_packets_are_counted_while_asked() {
        with_guests(|game| {
            let b = game.player(13).unwrap();
            assert_eq!(game.tick_times().len(), 100);
            assert!(game.count_packets(true));
            let sender = game.lan.as_ref().unwrap().hub.guest(13).unwrap().sender.clone();
            sender.send(vec![0x2b, 1, 2]);
            sender.send(vec![0x2b]);
            let counts = game.player_take_packet_counts(b).unwrap();
            assert!(game.count_packets(false));
            let ours = counts
                .iter()
                .find(|c| c.outbound && c.id == 0x2b)
                .expect("counted");
            assert_eq!((ours.packets, ours.bytes), (2, 4));
            assert!(game.player_take_packet_counts(b).unwrap().is_empty());
            sender.send(vec![0x2b]);
            assert!(game.player_take_packet_counts(b).unwrap().is_empty());
            assert!(!packet_counts::counting());
            assert!(game.mob_cost_sampling(true));
            assert!(game.mob_cost_sampling(false));
            assert!(game.take_mob_costs().is_empty());
        });
    }

    /// The distances are the hub's, set live on a dedicated server only, within 2 to 32.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_mod_reads_and_sets_the_servers_distances() {
        with_guests(|game| {
            assert_eq!(game.server_view_distance(), Some(2));
            assert_eq!(game.server_simulation_distance(), Some(2));
            assert!(!game.server_set_view_distance(6), "an integrated server");
            game.options.dedicated = Some(Vec::new());
            assert!(game.server_set_view_distance(6));
            assert!(game.server_set_simulation_distance(5));
            assert!(!game.server_set_view_distance(1));
            assert!(!game.server_set_simulation_distance(33));
            assert_eq!(game.server_view_distance(), Some(6));
            assert_eq!(game.server_simulation_distance(), Some(5));
            assert_eq!(game.settings.simulation_distance, 5);
        });
    }
}
