//! `ServerScoreboard`: what a player's client knows of the server's
//! scoreboard. Every team goes to every player; an objective goes once it
//! sits in a display slot (`startTrackingObjective`: its objective, its
//! slots, then its scores) and leaves when it sits in none
//! (`stopTrackingObjective`). A joining player gets it all
//! (`PlayerList.updateEntireScoreboard`).
//!
//! A server's commands change the board in place; each tick a player is
//! sent the packets that turn what it was last sent into what it should
//! have now, which are the ones `ServerScoreboard`'s callbacks send.
use super::guest::Guest;
use crate::scoreboard::{NumberFormat, RenderType, Scoreboard, component_tag, display_slots};
use crate::text::COLORS;
use mod_api::{Sidebar, Text, TextFormError};
use std::collections::{BTreeMap, BTreeSet};
use sunrise_core::nbt::{self, Tag};
use sunrise_protocol::packets::play::{
    self, Clientbound as C, ResetScore, SetDisplayObjective, SetObjective, SetPlayerTeam, SetScore,
    TeamParameters,
};

type Format = Option<play::NumberFormat>;

// Vanilla DisplaySlot ids: SIDEBAR, then TEAM_BLACK starts the color slots.
const SIDEBAR_SLOT: i32 = 1;
const FIRST_TEAM_SLOT: i32 = 3;

fn team_color_index(color: &str) -> Option<i32> {
    COLORS
        .iter()
        .position(|(_, n, _)| *n == color)
        .map(|i| i as i32)
}

/// The world scoreboard last sent to a client, excluding any personal sidebar.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct View {
    /// The tracked objectives: display name, render type (1 hearts), format.
    objectives: BTreeMap<String, (Tag, i32, Format)>,
    /// `DisplaySlot` id to objective.
    display: BTreeMap<i32, String>,
    /// The tracked objectives' scores, by holder and objective.
    scores: BTreeMap<(String, String), (i32, Option<Tag>, Format)>,
    /// Every team: its parameters and players, in the team's order.
    teams: BTreeMap<String, (TeamParameters, Vec<String>)>,
}

fn format(f: Option<&NumberFormat>) -> Format {
    f.map(|f| match f {
        NumberFormat::Blank => play::NumberFormat::Blank,
        NumberFormat::Styled(s) => play::NumberFormat::Styled(crate::scoreboard::json_to_tag(s)),
        NumberFormat::Fixed(c) => play::NumberFormat::Fixed(component_tag(c)),
    })
}

impl View {
    /// What the board's state gives a client.
    pub fn of(board: &Scoreboard) -> Self {
        let slots = display_slots();
        let display: BTreeMap<i32, String> = board
            .display
            .iter()
            .filter_map(|(slot, o)| Some((slots.iter().position(|s| s == slot)? as i32, o.clone())))
            .collect();
        let tracked: BTreeSet<&String> = display.values().collect();
        let objectives = board
            .objectives
            .iter()
            .filter(|o| tracked.contains(&o.name))
            .map(|o| {
                let render = i32::from(o.render_type == RenderType::Hearts);
                (
                    o.name.clone(),
                    (
                        component_tag(&o.display_name),
                        render,
                        format(o.number_format.as_ref()),
                    ),
                )
            })
            .collect();
        let scores = board
            .scores
            .iter()
            .flat_map(|(holder, scores)| {
                scores
                    .iter()
                    .filter(|(o, _)| tracked.contains(o))
                    .map(move |(o, s)| {
                        (
                            (holder.clone(), o.clone()),
                            (
                                s.value,
                                s.display.as_ref().map(component_tag),
                                format(s.number_format.as_ref()),
                            ),
                        )
                    })
            })
            .collect();
        let teams = board
            .teams
            .iter()
            .map(|t| {
                let params = TeamParameters {
                    display_name: component_tag(&t.display_name),
                    prefix: component_tag(&t.prefix),
                    suffix: component_tag(&t.suffix),
                    name_tag_visibility: t.nametag_visibility as i32,
                    collision_rule: t.collision_rule as i32,
                    color: t.color.as_deref().and_then(team_color_index),
                    options: i8::from(t.allow_friendly_fire)
                        | (i8::from(t.see_friendly_invisibles) << 1),
                };
                (
                    t.name.clone(),
                    (params, t.players.iter().cloned().collect()),
                )
            })
            .collect();
        Self {
            objectives,
            display,
            scores,
            teams,
        }
    }

    /// `ServerScoreboard.getStartTrackingPackets` for a joining player: every
    /// team, then each displayed objective in slot order, once, with its add,
    /// every slot showing it and its scores before the next objective's add.
    pub fn initial(&self) -> Vec<C> {
        let mut out: Vec<C> = self
            .teams
            .iter()
            .map(|(name, (params, players))| {
                C::SetPlayerTeam(SetPlayerTeam {
                    name: name.clone(),
                    method: 0,
                    parameters: Some(params.clone()),
                    players: players.clone(),
                })
            })
            .collect();
        let mut seen = BTreeSet::new();
        for objective in self.display.values() {
            if !seen.insert(objective) {
                continue;
            }
            let Some(display) = self.objectives.get(objective) else {
                continue;
            };
            out.push(C::SetObjective(SetObjective {
                name: objective.clone(),
                method: 0,
                display: Some(display.clone()),
            }));
            for (slot, _) in self.display.iter().filter(|(_, o)| *o == objective) {
                out.push(C::SetDisplayObjective(SetDisplayObjective {
                    slot: *slot,
                    objective: objective.clone(),
                }));
            }
            for ((holder, _), (value, display, number_format)) in
                self.scores.iter().filter(|((_, o), _)| o == objective)
            {
                out.push(C::SetScore(SetScore {
                    owner: holder.clone(),
                    objective: objective.clone(),
                    score: *value,
                    display: display.clone(),
                    number_format: number_format.clone(),
                }));
            }
        }
        out
    }

    /// The packets that take a client from `self` to `next`.
    pub fn changes(&self, next: &View) -> Vec<C> {
        let mut out = Vec::new();
        let team = |name: &str, method, parameters, players| {
            C::SetPlayerTeam(SetPlayerTeam {
                name: name.to_owned(),
                method,
                parameters,
                players,
            })
        };
        for name in self.teams.keys().filter(|n| !next.teams.contains_key(*n)) {
            out.push(team(name, 1, None, Vec::new()));
        }
        // Every leave before any join: `addPlayerToTeam` sends the old
        // team's leave first, and a client's `removePlayerFromTeam` throws
        // when the player already sits in another team.
        for (name, (_, had)) in &self.teams {
            let Some((_, players)) = next.teams.get(name) else {
                continue;
            };
            let left: Vec<_> = had
                .iter()
                .filter(|p| !players.contains(p))
                .cloned()
                .collect();
            if !left.is_empty() {
                out.push(team(name, 4, None, left));
            }
        }
        for (name, (params, players)) in &next.teams {
            let Some((was, had)) = self.teams.get(name) else {
                out.push(team(name, 0, Some(params.clone()), players.clone()));
                continue;
            };
            if was != params {
                out.push(team(name, 2, Some(params.clone()), Vec::new()));
            }
            let joined: Vec<_> = players
                .iter()
                .filter(|p| !had.contains(p))
                .cloned()
                .collect();
            if !joined.is_empty() {
                out.push(team(name, 3, None, joined));
            }
        }
        for (name, display) in &next.objectives {
            let method = match self.objectives.get(name) {
                None => 0,
                Some(was) if was != display => 2,
                Some(_) => continue,
            };
            out.push(C::SetObjective(SetObjective {
                name: name.clone(),
                method,
                display: Some(display.clone()),
            }));
        }
        let slots: BTreeSet<i32> = self
            .display
            .keys()
            .chain(next.display.keys())
            .copied()
            .collect();
        for slot in slots {
            let objective = next.display.get(&slot);
            if self.display.get(&slot) != objective {
                out.push(C::SetDisplayObjective(SetDisplayObjective {
                    slot,
                    objective: objective.cloned().unwrap_or_default(),
                }));
            }
        }
        for ((holder, objective), (value, display, number_format)) in &next.scores {
            let key = (holder.clone(), objective.clone());
            if self.scores.get(&key) != Some(&(*value, display.clone(), number_format.clone())) {
                out.push(C::SetScore(SetScore {
                    owner: holder.clone(),
                    objective: objective.clone(),
                    score: *value,
                    display: display.clone(),
                    number_format: number_format.clone(),
                }));
            }
        }
        for (holder, objective) in self.scores.keys() {
            let key = (holder.clone(), objective.clone());
            if !next.scores.contains_key(&key) && next.objectives.contains_key(objective) {
                out.push(C::ResetScore(ResetScore {
                    owner: holder.clone(),
                    objective: Some(objective.clone()),
                }));
            }
        }
        for name in self
            .objectives
            .keys()
            .filter(|n| !next.objectives.contains_key(*n))
        {
            out.push(C::SetObjective(SetObjective {
                name: name.clone(),
                method: 1,
                display: None,
            }));
        }
        out
    }
}

/// A complete personal sidebar admitted by the trusted scoreboard text codecs.
pub struct SidebarContents {
    title: Tag,
    lines: Vec<(Tag, play::NumberFormat)>,
}

impl SidebarContents {
    /// Prepare the title and only the lines `Hud.displayScoreboardSidebar` can show.
    /// Do this before changing either the connection's state or its packets.
    pub fn prepare(
        sidebar: Sidebar,
        has_item: &dyn Fn(&str) -> bool,
    ) -> Result<Self, TextFormError> {
        fn prepare_text(
            text: &Text,
            has_item: &dyn Fn(&str) -> bool,
        ) -> Result<Tag, TextFormError> {
            let component = crate::api_text::to_serialized_engine(text)?;
            let tag = component_tag(&component.component_view());
            // Trusted component streams use NbtAccounter.unlimitedHeap, including its depth bound.
            let limits = nbt::Limits::unlimited_heap();
            if !nbt::within_write_depth(&tag, limits.max_depth) {
                return Err(TextFormError::TooDeep);
            }
            let bytes = nbt::write_network(&tag, limits).map_err(|_| TextFormError::Malformed)?;
            let (decoded, _) =
                nbt::read_network(&bytes, limits).map_err(|_| TextFormError::Malformed)?;
            sunrise_world::component_value::source::text_from_tag(
                &decoded,
                &|id| sunrise_protocol::item::custom::id(id).is_some(),
                has_item,
            )
            .map_err(|_| TextFormError::Malformed)?;
            Ok(tag)
        }

        let title = prepare_text(&sidebar.title, has_item)?;
        let lines = sidebar
            .lines
            .into_iter()
            .take(15)
            .map(|line| {
                let text = prepare_text(&line.text, has_item)?;
                let format = match line.value {
                    Some(value) => play::NumberFormat::Fixed(prepare_text(&value, has_item)?),
                    None => play::NumberFormat::Blank,
                };
                Ok((text, format))
            })
            .collect::<Result<_, TextFormError>>()?;
        Ok(Self { title, lines })
    }
}

/// Connection-local state. The objective is never inserted into `Scoreboard`.
pub(super) struct PersonalSidebar {
    name: String,
    contents: SidebarContents,
    ordinary_slot: bool,
    team_slot: Option<i32>,
}

impl PersonalSidebar {
    fn free_name(board: &Scoreboard, sent: &View) -> String {
        // Brigadier's unquoted objective argument excludes ':'. Native writes and saves can
        // still name an objective arbitrarily, including one previously sent to this client.
        (0usize..)
            .map(|suffix| format!("sunrise:sidebar{suffix}"))
            .find(|name| board.objective(name).is_none() && !sent.objectives.contains_key(name))
            .expect("a private objective name remains free")
    }

    fn remove_packet(&self) -> C {
        C::SetObjective(SetObjective {
            name: self.name.clone(),
            method: 1,
            display: None,
        })
    }

    fn append_show(&self, out: &mut Vec<C>) {
        out.push(C::SetObjective(SetObjective {
            name: self.name.clone(),
            method: 0,
            display: Some((self.contents.title.clone(), 0, None)),
        }));
        for slot in self
            .ordinary_slot
            .then_some(SIDEBAR_SLOT)
            .into_iter()
            .chain(self.team_slot)
        {
            out.push(C::SetDisplayObjective(SetDisplayObjective {
                slot,
                objective: self.name.clone(),
            }));
        }
        let lines = &self.contents.lines;
        for (i, (text, format)) in lines.iter().enumerate() {
            out.push(C::SetScore(SetScore {
                owner: format!("sunrise:line{i}"),
                objective: self.name.clone(),
                score: (lines.len() - i) as i32,
                display: Some(text.clone()),
                number_format: Some(format.clone()),
            }));
        }
    }
}

impl Guest {
    /// Replace or clear this connection's private sidebar. A first set can come from the
    /// login hook before the periodic scoreboard fan-out has initialized the client.
    pub fn set_personal_sidebar(&mut self, board: &Scoreboard, contents: Option<SidebarContents>) {
        if contents.is_some() && !self.scoreboard_synced {
            self.sync_scoreboard(&View::of(board), board);
        }
        let mut out = Vec::new();
        let previous = self.personal_sidebar.take();
        if let Some(previous) = &previous {
            out.push(previous.remove_packet());
        }
        if let Some(contents) = contents {
            let team_slot = board
                .players_team(&self.name)
                .and_then(|team| team.color.as_deref())
                .and_then(team_color_index)
                .map(|color| FIRST_TEAM_SLOT + color);
            if let Some(slot) = previous.and_then(|p| p.team_slot)
                && Some(slot) != team_slot
                && let Some(objective) = self.scoreboard.display.get(&slot)
            {
                out.push(C::SetDisplayObjective(SetDisplayObjective {
                    slot,
                    objective: objective.clone(),
                }));
            }
            let sidebar = Box::new(PersonalSidebar {
                name: PersonalSidebar::free_name(board, &self.scoreboard),
                contents,
                ordinary_slot: true,
                team_slot,
            });
            sidebar.append_show(&mut out);
            self.personal_sidebar = Some(sidebar);
        } else if let Some(previous) = previous {
            // Restore only already-published world objectives. The ordinary slot may have
            // yielded, and the player may have left the original team in the meantime.
            for (&slot, objective) in &self.scoreboard.display {
                if slot >= FIRST_TEAM_SLOT || (slot == SIDEBAR_SLOT && previous.ordinary_slot) {
                    out.push(C::SetDisplayObjective(SetDisplayObjective {
                        slot,
                        objective: objective.clone(),
                    }));
                }
            }
        }
        self.send_all(&out);
    }

    /// Publish the normal world view, observing actual display writes rather than unrelated
    /// score or team changes. If a world objective takes the private name, remove the private
    /// objective before the world adds its own and replay the overlay under a free name.
    pub fn sync_scoreboard(&mut self, view: &View, board: &Scoreboard) {
        if !self.scoreboard_synced {
            self.send_all(&view.initial());
            self.scoreboard = view.clone();
            self.scoreboard_synced = true;
            return;
        }
        if self.scoreboard == *view {
            return;
        }
        let mut out = self.scoreboard.changes(view);
        if let Some(sidebar) = &mut self.personal_sidebar {
            for packet in &out {
                if let C::SetDisplayObjective(display) = packet {
                    if display.slot == SIDEBAR_SLOT {
                        sidebar.ordinary_slot = false;
                    }
                    if Some(display.slot) == sidebar.team_slot {
                        sidebar.team_slot = None;
                    }
                }
            }
            if view.objectives.contains_key(&sidebar.name) {
                out.insert(0, sidebar.remove_packet());
                sidebar.name = PersonalSidebar::free_name(board, &self.scoreboard);
                sidebar.append_show(&mut out);
            }
        }
        self.send_all(&out);
        self.scoreboard = view.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lan::{Joined, Out, PlayerSender};
    use mod_api::SidebarLine;
    use serde_json::json;
    use std::sync::{Arc, mpsc};
    use sunrise_protocol::{buf::Reader, packets::configuration::ClientInformation};

    fn connection(conn: u32, uuid: u128, name: &str) -> (Guest, mpsc::Receiver<Out>) {
        let (tx, rx) = mpsc::channel();
        let mut guest = Guest::new(
            Joined {
                conn,
                uuid,
                name: name.into(),
                properties: Vec::new(),
                info: ClientInformation::new(8),
                brand: String::new(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::default(),
                address: "127.0.0.1:1".parse().unwrap(),
                expected: false,
            },
            conn as i32,
            [0.0; 3],
            8,
        );
        guest.placed = true;
        (guest, rx)
    }

    fn received(rx: &mpsc::Receiver<Out>) -> Vec<C> {
        rx.try_iter()
            .flat_map(|out| match out {
                Out::Packet(body) => vec![body],
                Out::Packets(bodies) => bodies,
                Out::Disconnect(_) => panic!("unexpected disconnect"),
            })
            .map(|body| {
                let mut r = Reader::new(&body);
                C::decode(r.varint().unwrap(), r.rest()).unwrap()
            })
            .collect()
    }

    fn world_board() -> Scoreboard {
        let mut board = Scoreboard::default();
        board.add_objective(
            "world",
            "dummy",
            json!("World"),
            RenderType::Integer,
            false,
            None,
        );
        board.set_display_objective("sidebar", Some("world"));
        board
    }

    fn plain_sidebar(title: &str, line: &str) -> SidebarContents {
        SidebarContents::prepare(
            Sidebar {
                title: Text::literal(title),
                lines: vec![SidebarLine {
                    text: Text::literal(line),
                    value: None,
                }],
            },
            &|_| false,
        )
        .unwrap()
    }

    #[test]
    fn personal_sidebar_is_private_and_starts_after_the_join_snapshot() {
        let mut board = world_board();
        board.add_team("red").color = Some("red".into());
        board.add_to_team("Alpha", "red");
        let (mut alpha, alpha_rx) = connection(1, 11, "Alpha");
        let (mut beta, beta_rx) = connection(2, 12, "Beta");
        let mut lines: Vec<_> = (1..=15)
            .map(|i| SidebarLine {
                text: Text::literal(format!("Line {i}")),
                value: None,
            })
            .collect();
        lines[0].value = Some(Text::literal("3 pts"));
        // An overlong modified-UTF-8 string is not even checked past the HUD cut.
        lines.push(SidebarLine {
            text: Text::literal("\0".repeat(32_768)),
            value: None,
        });
        let contents = SidebarContents::prepare(
            Sidebar {
                title: Text::literal("Private"),
                lines,
            },
            &|_| false,
        )
        .unwrap();
        alpha.set_personal_sidebar(&board, Some(contents));

        let packets = received(&alpha_rx);
        let (world, personal) = packets.split_at(3);
        assert_eq!(
            methods(world),
            [
                "team red 0 [\"Alpha\"]",
                "objective world 0",
                "display 1 world"
            ]
        );
        let [
            C::SetObjective(objective),
            C::SetDisplayObjective(ordinary),
            C::SetDisplayObjective(red),
            scores @ ..,
        ] = personal
        else {
            panic!("expected add, ordinary and red display slots, then scores");
        };
        assert_eq!(objective.method, 0);
        assert_eq!(
            objective.display,
            Some((Tag::String("Private".into()), 0, None))
        );
        assert_eq!((ordinary.slot, red.slot), (1, 15));
        assert_eq!(ordinary.objective, objective.name);
        assert_eq!(red.objective, objective.name);
        let mut holders = BTreeSet::new();
        assert_eq!(scores.len(), 15);
        for (i, score) in scores.iter().enumerate() {
            let C::SetScore(score) = score else {
                panic!("expected a score")
            };
            assert!(!score.owner.starts_with('#') && holders.insert(&score.owner));
            assert_eq!(score.objective, objective.name);
            assert_eq!(score.score, 15 - i as i32);
            assert_eq!(
                score.display,
                Some(Tag::String(format!("Line {}", i + 1).as_str().into()))
            );
            let expected = if i == 0 {
                play::NumberFormat::Fixed(Tag::String("3 pts".into()))
            } else {
                play::NumberFormat::Blank
            };
            assert_eq!(score.number_format, Some(expected));
        }
        assert!(board.objective(&objective.name).is_none());
        assert!(received(&beta_rx).is_empty());
        let view = View::of(&board);
        alpha.sync_scoreboard(&view, &board);
        beta.sync_scoreboard(&view, &board);
        assert!(received(&alpha_rx).is_empty());
        assert_eq!(methods(&received(&beta_rx)), methods(world));
    }

    #[test]
    fn replacing_removes_old_lines_and_clearing_restores_world_team_slots() {
        let mut board = world_board();
        for color in ["red", "blue"] {
            board.add_team(color).color = Some(color.into());
            board.add_objective(
                color,
                "dummy",
                json!(color),
                RenderType::Integer,
                false,
                None,
            );
            board.set_display_objective(&format!("sidebar.team.{color}"), Some(color));
        }
        board.add_to_team("Alpha", "red");
        let (mut alpha, rx) = connection(1, 11, "Alpha");
        let old = SidebarContents::prepare(
            Sidebar {
                title: Text::literal("Old"),
                lines: vec![
                    SidebarLine {
                        text: Text::literal("old line"),
                        value: None
                    };
                    2
                ],
            },
            &|_| false,
        )
        .unwrap();
        alpha.set_personal_sidebar(&board, Some(old));
        let first = received(&rx);
        let old_name = first
            .iter()
            .find_map(|p| match p {
                C::SetObjective(o) if o.display == Some((Tag::String("Old".into()), 0, None)) => {
                    Some(o.name.as_str())
                }
                _ => None,
            })
            .unwrap();

        board.add_to_team("Alpha", "blue");
        alpha.sync_scoreboard(&View::of(&board), &board);
        received(&rx);
        alpha.set_personal_sidebar(&board, Some(plain_sidebar("New", "only line")));
        let replaced = received(&rx);
        let [
            C::SetObjective(removed),
            C::SetDisplayObjective(restored),
            C::SetObjective(added),
            C::SetDisplayObjective(ordinary),
            C::SetDisplayObjective(blue),
            C::SetScore(line),
        ] = replaced.as_slice()
        else {
            panic!("expected removal and restoration before replacement with one score");
        };
        assert_eq!((removed.method, removed.name.as_str()), (1, old_name));
        assert_eq!((restored.slot, restored.objective.as_str()), (15, "red"));
        assert_eq!(added.display, Some((Tag::String("New".into()), 0, None)));
        assert_eq!((ordinary.slot, blue.slot), (1, 12));
        assert_eq!(line.display, Some(Tag::String("only line".into())));
        assert_eq!(line.objective, added.name);
        assert_eq!(line.score, 1);

        // The overlay still holds blue even though the player has moved back to red.
        board.add_to_team("Alpha", "red");
        alpha.sync_scoreboard(&View::of(&board), &board);
        received(&rx);
        alpha.set_personal_sidebar(&board, None);
        assert_eq!(
            methods(&received(&rx)),
            [
                format!("objective {} 1", added.name),
                "display 1 world".into(),
                "display 12 blue".into(),
                "display 15 red".into()
            ]
        );
        alpha.set_personal_sidebar(&board, None);
        assert!(received(&rx).is_empty());

        let (mut rejoined, rejoined_rx) = connection(3, 11, "Alpha");
        rejoined.sync_scoreboard(&View::of(&board), &board);
        let initial = methods(&received(&rejoined_rx));
        assert!(initial.contains(&"display 1 world".into()));
        assert!(
            !initial
                .iter()
                .any(|message| message.contains(added.name.as_str()))
        );
        rejoined.set_personal_sidebar(&board, None);
        assert!(received(&rejoined_rx).is_empty());
    }

    #[test]
    fn only_a_world_write_to_the_ordinary_slot_takes_it_back() {
        for (slot, assignment, owns_ordinary) in [
            (None, None, true),
            (Some("below_name"), Some("other"), true),
            (Some("sidebar"), Some("other"), false),
            (Some("sidebar"), None, false),
        ] {
            let mut board = world_board();
            board.add_objective(
                "other",
                "dummy",
                json!("Other"),
                RenderType::Integer,
                false,
                None,
            );
            let (mut alpha, rx) = connection(1, 11, "Alpha");
            alpha.set_personal_sidebar(&board, Some(plain_sidebar("Private", "line")));
            received(&rx);
            if let Some(slot) = slot {
                board.set_display_objective(slot, assignment);
            } else {
                board.score_mut("Alpha", "world").value = 20;
            }
            alpha.sync_scoreboard(&View::of(&board), &board);
            let world_diff = received(&rx);
            assert_eq!(
                world_diff
                    .iter()
                    .any(|p| matches!(p, C::SetDisplayObjective(d) if d.slot == 1)),
                !owns_ordinary
            );
            alpha.set_personal_sidebar(&board, None);
            let clear = received(&rx);
            assert!(matches!(clear.first(), Some(C::SetObjective(o)) if o.method == 1));
            if owns_ordinary {
                assert!(
                    matches!(&clear[1..], [C::SetDisplayObjective(d)] if d.slot == 1 && d.objective == "world")
                );
            } else {
                assert_eq!(clear.len(), 1);
            }
        }
    }

    #[test]
    fn publishing_a_colliding_world_objective_rekeys_only_the_private_one() {
        let mut board = world_board();
        board.add_team("red").color = Some("red".into());
        board.add_to_team("Alpha", "red");
        let (mut alpha, alpha_rx) = connection(1, 11, "Alpha");
        let (mut beta, beta_rx) = connection(2, 12, "Beta");
        let view = View::of(&board);
        alpha.sync_scoreboard(&view, &board);
        beta.sync_scoreboard(&view, &board);
        received(&alpha_rx);
        received(&beta_rx);
        alpha.set_personal_sidebar(&board, Some(plain_sidebar("Private", "line")));
        let first = received(&alpha_rx);
        let C::SetObjective(private) = &first[0] else {
            panic!("expected a private objective")
        };
        board.add_objective(
            &private.name,
            "dummy",
            json!("Shared"),
            RenderType::Integer,
            false,
            None,
        );
        // An objective the world hasn't published cannot alias on either client yet.
        alpha.sync_scoreboard(&View::of(&board), &board);
        assert!(received(&alpha_rx).is_empty());

        board.set_display_objective("sidebar", Some(&private.name));
        let next = View::of(&board);
        alpha.sync_scoreboard(&next, &board);
        beta.sync_scoreboard(&next, &board);
        let alpha_diff = received(&alpha_rx);
        let beta_diff = received(&beta_rx);
        let C::SetObjective(removed) = &alpha_diff[0] else {
            panic!("expected private removal before world add")
        };
        assert_eq!(removed.name, private.name);
        assert_eq!(removed.method, 1);
        let world_len = beta_diff.len();
        assert_eq!(methods(&alpha_diff[1..=world_len]), methods(&beta_diff));
        assert!(
            matches!(&beta_diff[0], C::SetObjective(o) if o.name == private.name && o.display == Some((Tag::String("Shared".into()), 0, None)))
        );
        let [
            C::SetObjective(rekeyed),
            C::SetDisplayObjective(red),
            C::SetScore(line),
        ] = &alpha_diff[1 + world_len..]
        else {
            panic!("expected replay only in the color slot still owned by the private sidebar");
        };
        assert_ne!(rekeyed.name, private.name);
        assert_eq!(rekeyed.display, private.display);
        assert_eq!(red.slot, 15);
        assert_eq!(red.objective, rekeyed.name);
        assert_eq!(line.objective, rekeyed.name);
        assert_eq!(line.display, Some(Tag::String("line".into())));
        alpha.set_personal_sidebar(&board, None);
        assert_eq!(
            methods(&received(&alpha_rx)),
            [format!("objective {} 1", rekeyed.name)]
        );
    }

    #[test]
    fn sidebar_hover_items_must_be_known_to_the_receiving_item_codec() {
        let mut title = Text::literal("Hover");
        title.style.hover = Some(mod_api::TextHover::Item {
            id: "test:token".into(),
            count: 1,
            components: None,
        });
        let sidebar = Sidebar {
            title,
            lines: Vec::new(),
        };
        assert!(matches!(
            SidebarContents::prepare(sidebar.clone(), &|_| false),
            Err(TextFormError::Malformed)
        ));
        assert!(SidebarContents::prepare(sidebar, &|item| item == "test:token").is_ok());
    }

    #[test]
    fn sidebar_prepares_every_visible_text_with_the_trusted_nbt_policy() {
        for field in 0..3 {
            let mut sidebar = Sidebar {
                title: Text::literal("Title"),
                lines: vec![SidebarLine {
                    text: Text::literal("Line"),
                    value: Some(Text::literal("Value")),
                }],
            };
            let overlong = Text::literal("\0".repeat(32_768));
            match field {
                0 => sidebar.title = overlong,
                1 => sidebar.lines[0].text = overlong,
                _ => sidebar.lines[0].value = Some(overlong),
            }
            assert!(matches!(
                SidebarContents::prepare(sidebar, &|_| false),
                Err(TextFormError::Malformed)
            ));
        }
        // Every nested child adds an `extra` list and a component compound. The API node
        // depth fits, but the binary tag's container depth does not.
        let deep = (0..260).fold(Text::literal("leaf"), |child, _| {
            Text::literal("").append(child)
        });
        assert!(deep.validate().is_ok());
        assert!(matches!(
            SidebarContents::prepare(
                Sidebar {
                    title: deep,
                    lines: vec![]
                },
                &|_| false
            ),
            Err(TextFormError::TooDeep)
        ));

        // Twenty individual legal strings cross the bounded NBT heap quota, not the per-string limit.
        let chunk = "x".repeat(60_000);
        let wide = (0..19).fold(Text::literal(chunk.as_str()), |root, _| {
            root.append(Text::literal(chunk.as_str()))
        });
        let contents = SidebarContents::prepare(
            Sidebar {
                title: wide.clone(),
                lines: vec![SidebarLine {
                    text: Text::literal("Line"),
                    value: Some(wide),
                }],
            },
            &|_| false,
        )
        .unwrap();
        let (mut alpha, rx) = connection(1, 11, "Alpha");
        alpha.set_personal_sidebar(&Scoreboard::default(), Some(contents));
        let packets = received(&rx);
        let [
            C::SetObjective(title),
            C::SetDisplayObjective(_),
            C::SetScore(row),
        ] = packets.as_slice()
        else {
            panic!("expected a scoreboard add and score that survived the trusted packet codecs");
        };
        let (title, _, _) = title.display.as_ref().unwrap();
        let Some(play::NumberFormat::Fixed(value)) = &row.number_format else {
            panic!("expected fixed text")
        };
        for text in [title, value] {
            let bytes = nbt::write_network(text, nbt::Limits::unlimited_heap()).unwrap();
            // Vanilla's network NbtAccounter (2 MiB, two bytes per string unit) refuses it.
            assert!(Tag::from_network_bytes(&bytes, None).is_err());
            assert!(matches!(text.get("extra"), Some(Tag::List(children)) if children.len() == 19));
        }
    }

    fn methods(packets: &[C]) -> Vec<String> {
        packets
            .iter()
            .map(|p| match p {
                C::SetObjective(o) => format!("objective {} {}", o.name, o.method),
                C::SetDisplayObjective(d) => format!("display {} {}", d.slot, d.objective),
                C::SetScore(s) => format!("score {} {} {}", s.owner, s.objective, s.score),
                C::ResetScore(r) => format!("reset {} {:?}", r.owner, r.objective),
                C::SetPlayerTeam(t) => format!("team {} {} {:?}", t.name, t.method, t.players),
                _ => "other".into(),
            })
            .collect()
    }

    #[test]
    fn a_shown_objective_goes_with_its_scores_and_leaves_when_hidden() {
        let mut board = Scoreboard::default();
        board.add_objective(
            "audit",
            "dummy",
            json!("Audit"),
            RenderType::Integer,
            false,
            None,
        );
        board.score_mut("Alpha", "audit").value = 7;
        let empty = View::default();
        // Not displayed: not tracked, nothing sent.
        let hidden = View::of(&board);
        assert!(empty.changes(&hidden).is_empty());
        board.set_display_objective("sidebar", Some("audit"));
        let shown = View::of(&board);
        assert_eq!(
            methods(&hidden.changes(&shown)),
            [
                "objective audit 0",
                "display 1 audit",
                "score Alpha audit 7"
            ]
        );
        board.score_mut("Alpha", "audit").value = 12;
        board.score_mut("Beta", "audit").value = 3;
        board.reset_one("Beta", "audit");
        let changed = View::of(&board);
        assert_eq!(methods(&shown.changes(&changed)), ["score Alpha audit 12"]);
        board.reset_one("Alpha", "audit");
        let reset = View::of(&board);
        assert_eq!(
            methods(&changed.changes(&reset)),
            ["reset Alpha Some(\"audit\")"]
        );
        board.set_display_objective("sidebar", None);
        assert_eq!(
            methods(&reset.changes(&View::of(&board))),
            ["display 1 ", "objective audit 1"]
        );
    }

    #[test]
    fn teams_go_to_everyone_with_their_players() {
        let mut board = Scoreboard::default();
        board.add_team("red");
        board.add_to_team("Alpha", "red");
        let first = View::of(&board);
        assert_eq!(
            methods(&View::default().changes(&first)),
            ["team red 0 [\"Alpha\"]"]
        );
        board.add_to_team("Beta", "red");
        board.remove_from_team("Alpha");
        let second = View::of(&board);
        assert_eq!(
            methods(&first.changes(&second)),
            ["team red 4 [\"Alpha\"]", "team red 3 [\"Beta\"]"]
        );
        board.remove_team("red");
        assert_eq!(
            methods(&second.changes(&View::of(&board))),
            ["team red 1 []"]
        );
    }

    #[test]
    fn a_player_moving_teams_leaves_the_old_one_first() {
        let mut board = Scoreboard::default();
        board.add_team("red");
        board.add_team("blue");
        board.add_to_team("Alpha", "red");
        let first = View::of(&board);
        // `blue` sorts before `red`; the leave still goes first.
        board.add_to_team("Alpha", "blue");
        board.add_team("amber");
        board.add_to_team("Beta", "amber");
        assert_eq!(
            methods(&first.changes(&View::of(&board))),
            [
                "team red 4 [\"Alpha\"]",
                "team amber 0 [\"Beta\"]",
                "team blue 3 [\"Alpha\"]"
            ]
        );
    }

    #[test]
    fn a_joining_player_gets_each_displayed_objective_whole_in_slot_order() {
        let mut board = Scoreboard::default();
        for name in ["zebra", "alpha", "hidden"] {
            board.add_objective(name, "dummy", json!(name), RenderType::Integer, false, None);
            board.score_mut("Alpha", name).value = 1;
        }
        board.score_mut("Beta", "zebra").value = 2;
        board.set_display_objective("list", Some("zebra"));
        board.set_display_objective("sidebar", Some("alpha"));
        board.set_display_objective("below_name", Some("zebra"));
        board.add_team("red");
        board.add_to_team("Alpha", "red");
        let mut view = View::of(&board);
        view.display.insert(3, "missing".into());
        let mut trace = methods(&view.initial());
        assert_eq!(trace.len(), 9);
        // Score order within an objective is unspecified; the group stays between
        // zebra's display packets and alpha's add.
        trace[4..6].sort_unstable();
        assert_eq!(
            trace,
            [
                "team red 0 [\"Alpha\"]",
                "objective zebra 0",
                "display 0 zebra",
                "display 2 zebra",
                "score Alpha zebra 1",
                "score Beta zebra 2",
                "objective alpha 0",
                "display 1 alpha",
                "score Alpha alpha 1",
            ]
        );
    }
}
