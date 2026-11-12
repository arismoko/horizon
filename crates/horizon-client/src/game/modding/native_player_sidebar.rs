//! `player_set_sidebar`: a sidebar only one player's client holds.
use super::{Game, api_values::TryIntoEngine, native_server::NativeSidebar};
use mod_api::{PlayerRef, Sidebar, TextFormError};

impl Game<'_> {
    /// Shows `sidebar` to that player, or takes it away (None). Ok(false) when they
    /// aren't on; a text the boundary refuses changes nothing.
    pub(super) fn player_set_sidebar(
        &mut self,
        player: PlayerRef,
        sidebar: Option<Sidebar>,
    ) -> Result<bool, TextFormError> {
        let sidebar = match sidebar {
            Some(sidebar) => {
                let title = sidebar.title.try_into_engine(&())?;
                let mut lines = Vec::with_capacity(sidebar.lines.len());
                for line in &sidebar.lines {
                    let text = line.text.try_into_engine(&())?;
                    let value = line.value.try_into_engine(&())?;
                    lines.push((text, value));
                }
                Some(NativeSidebar { title, lines })
            }
            None => None,
        };
        let online = self.native_acting_player() == Some(player)
            || self.online_player_record(player).is_some();
        Ok(online && self.native_sidebar(player.uuid(), sidebar.as_ref()))
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_guest_turn_tests::with_guests;
    use horizon_client::lan::PlayerSender;
    use horizon_protocol::packets::play::{self, Clientbound as C};
    use mod_api::{Sidebar, SidebarLine, Text};
    use std::sync::{Arc, atomic::AtomicUsize};

    /// The sidebar reaches only that player: its objective, the sidebar slot and one score a
    /// line with the right-hand text as a fixed number format; None removes it.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn player_set_sidebar_sends_the_objective_and_its_lines() {
        with_guests(|game| {
            let (tx, rx) = std::sync::mpsc::channel();
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |g| {
                    g.sender = PlayerSender::new(tx, Arc::new(AtomicUsize::new(0)));
                })
                .unwrap();
            let a = game.player(12).unwrap();
            let sidebar = Sidebar::new(
                Text::literal("Stats"),
                vec![
                    SidebarLine::new(Text::literal("Kills"), Some(Text::literal("3"))),
                    SidebarLine::new(Text::literal("Coins"), None),
                ],
            );
            assert_eq!(game.player_set_sidebar(a, Some(sidebar)), Ok(true));
            let sent: Vec<C> = rx
                .try_iter()
                .filter_map(|out| match out {
                    horizon_client::lan::Out::Packet(body) => {
                        let mut r = horizon_protocol::buf::Reader::new(&body);
                        let id = r.varint().ok()?;
                        C::decode(id, &body[r.position()..]).ok()
                    }
                    _ => None,
                })
                .collect();
            let scores: Vec<_> = sent
                .iter()
                .filter_map(|p| match p {
                    C::SetScore(s) => Some((s.score, s.number_format.clone())),
                    _ => None,
                })
                .collect();
            assert_eq!(scores.len(), 2);
            assert_eq!(scores[0].0, 2, "top line scores highest");
            assert!(matches!(scores[0].1, Some(play::NumberFormat::Fixed(_))));
            assert!(matches!(scores[1].1, Some(play::NumberFormat::Blank)));
            assert!(
                sent.iter()
                    .any(|p| matches!(p, C::SetDisplayObjective(d) if d.slot == 1))
            );
            assert_eq!(game.player_set_sidebar(a, None), Ok(true));
            let gone = mod_api::__private::game::player_ref(a.entity(), 999);
            assert_eq!(game.player_set_sidebar(gone, None), Ok(false));
        });
    }
}
