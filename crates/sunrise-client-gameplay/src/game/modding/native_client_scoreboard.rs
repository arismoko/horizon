//! Scoped reads of the scoreboard populated by client play packets.

use crate::game::ClientGame;
use core::ops::ControlFlow;
use mod_api::{
    ClientObjective, ClientScore, ClientScoreboard, ClientSessionReadError, ClientTeam,
    CollisionRule, DisplaySlot, NumberFormat, ObjectiveRenderType, TeamColor, TeamVisibility, Text,
    TextFormError,
};
use sunrise_game_data::{
    scoreboard::{self as sb, Comp, comp_text},
    text::Text as EngineText,
};
use sunrise_gameplay_common::game::{api_values::TryIntoApi, scoreboard_common::ScoreState};

fn api_text(text: &EngineText) -> Result<Text, TextFormError> {
    text.try_into_api()
}

fn api_comp(comp: &Comp) -> Result<Text, TextFormError> {
    api_text(&comp_text(comp))
}

fn api_render_type(render: sb::RenderType) -> ObjectiveRenderType {
    match render {
        sb::RenderType::Integer => ObjectiveRenderType::Integer,
        sb::RenderType::Hearts => ObjectiveRenderType::Hearts,
    }
}

fn api_number_format(format: &sb::NumberFormat) -> Result<NumberFormat, TextFormError> {
    Ok(match format {
        sb::NumberFormat::Blank => NumberFormat::Blank,
        sb::NumberFormat::Fixed(comp) => NumberFormat::Fixed(api_text(&comp_text(comp))?),
        sb::NumberFormat::Styled(style) => {
            let mut json = style.clone();
            if let Comp::Object(fields) = &mut json {
                fields.insert("text".into(), Comp::String(String::new()));
            }
            NumberFormat::Styled(std::mem::take(&mut api_text(&comp_text(&json))?.style))
        }
    })
}

fn objective_slots<'a>(
    board: &'a sb::Scoreboard,
    name: &'a str,
) -> impl Iterator<Item = DisplaySlot> + 'a {
    DisplaySlot::all().filter(move |slot| {
        board
            .display
            .get(slot.name())
            .is_some_and(|objective| objective.as_str() == name)
    })
}

fn api_visibility(visibility: sb::Visibility) -> TeamVisibility {
    match visibility {
        sb::Visibility::Always => TeamVisibility::Always,
        sb::Visibility::Never => TeamVisibility::Never,
        sb::Visibility::HideForOtherTeams => TeamVisibility::HideForOtherTeams,
        sb::Visibility::HideForOwnTeam => TeamVisibility::HideForOwnTeam,
    }
}

fn api_collision_rule(rule: sb::CollisionRule) -> CollisionRule {
    match rule {
        sb::CollisionRule::Always => CollisionRule::Always,
        sb::CollisionRule::Never => CollisionRule::Never,
        sb::CollisionRule::PushOtherTeams => CollisionRule::PushOtherTeams,
        sb::CollisionRule::PushOwnTeam => CollisionRule::PushOwnTeam,
    }
}

fn api_color(team: &sb::Team) -> Option<TeamColor> {
    team.color.as_deref().and_then(TeamColor::from_name)
}

impl ClientGame<'_> {
    /// No board is exposed until this window has a logged-in client world.
    pub(in crate::game) fn client_scoreboard(
        &self,
    ) -> Result<Option<&dyn ClientScoreboard>, ClientSessionReadError> {
        Ok(self.native_session().map(|_| self as &dyn ClientScoreboard))
    }
}

// Lend the state already resident on ClientGame; only the nested views need stack wrappers.
impl ClientScoreboard for ClientGame<'_> {
    fn visit_objectives(
        &self,
        visit: &mut dyn FnMut(&dyn ClientObjective) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        ScoreboardView(&self.common.score).visit_objectives(visit)
    }

    fn visit_teams(
        &self,
        visit: &mut dyn FnMut(&dyn ClientTeam) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        ScoreboardView(&self.common.score).visit_teams(visit)
    }

    fn with_slot(
        &self,
        slot: DisplaySlot,
        visit: &mut dyn FnMut(&dyn ClientObjective),
    ) -> Option<()> {
        ScoreboardView(&self.common.score).with_slot(slot, visit)
    }
}

struct ScoreboardView<'a>(&'a ScoreState);

impl ClientScoreboard for ScoreboardView<'_> {
    fn visit_objectives(
        &self,
        visit: &mut dyn FnMut(&dyn ClientObjective) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        self.0.board.objectives.iter().try_for_each(|objective| {
            visit(&ObjectiveView {
                board: &self.0.board,
                objective,
            })
        })
    }

    fn visit_teams(
        &self,
        visit: &mut dyn FnMut(&dyn ClientTeam) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        self.0
            .board
            .teams
            .iter()
            .try_for_each(|team| visit(&TeamView(team)))
    }

    fn with_slot(
        &self,
        slot: DisplaySlot,
        visit: &mut dyn FnMut(&dyn ClientObjective),
    ) -> Option<()> {
        let objective = self.0.board.display_objective(slot.name())?;
        visit(&ObjectiveView {
            board: &self.0.board,
            objective,
        });
        Some(())
    }
}

struct ObjectiveView<'a> {
    board: &'a sb::Scoreboard,
    objective: &'a sb::Objective,
}

impl ClientObjective for ObjectiveView<'_> {
    fn name(&self) -> &str {
        &self.objective.name
    }

    fn render_type(&self) -> ObjectiveRenderType {
        api_render_type(self.objective.render_type)
    }

    fn visit_slots(
        &self,
        visit: &mut dyn FnMut(DisplaySlot) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        objective_slots(self.board, self.name()).try_for_each(visit)
    }

    fn visit_scores(
        &self,
        visit: &mut dyn FnMut(&dyn ClientScore) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        self.board
            .score_rows(self.name())
            .try_for_each(|(holder, score)| visit(&ScoreView { holder, score }))
    }

    fn copy_display_name(&self) -> Result<Text, TextFormError> {
        api_comp(&self.objective.display_name)
    }

    fn copy_number_format(&self) -> Result<Option<NumberFormat>, TextFormError> {
        self.objective
            .number_format
            .as_ref()
            .map(api_number_format)
            .transpose()
    }
}

struct ScoreView<'a> {
    holder: &'a str,
    score: &'a sb::Score,
}

impl ClientScore for ScoreView<'_> {
    fn holder(&self) -> &str {
        self.holder
    }

    fn value(&self) -> i32 {
        self.score.value
    }

    fn copy_display(&self) -> Result<Option<Text>, TextFormError> {
        self.score.display.as_ref().map(api_comp).transpose()
    }

    fn copy_number_format(&self) -> Result<Option<NumberFormat>, TextFormError> {
        self.score
            .number_format
            .as_ref()
            .map(api_number_format)
            .transpose()
    }
}

struct TeamView<'a>(&'a sb::Team);

impl ClientTeam for TeamView<'_> {
    fn name(&self) -> &str {
        &self.0.name
    }

    fn color(&self) -> Option<TeamColor> {
        api_color(self.0)
    }

    fn friendly_fire(&self) -> bool {
        self.0.allow_friendly_fire
    }

    fn see_friendly_invisibles(&self) -> bool {
        self.0.see_friendly_invisibles
    }

    fn name_tag_visibility(&self) -> TeamVisibility {
        api_visibility(self.0.nametag_visibility)
    }

    fn collision_rule(&self) -> CollisionRule {
        api_collision_rule(self.0.collision_rule)
    }

    fn visit_members(&self, visit: &mut dyn FnMut(&str) -> ControlFlow<()>) -> ControlFlow<()> {
        self.0
            .players
            .iter()
            .map(String::as_str)
            .try_for_each(visit)
    }

    fn copy_display_name(&self) -> Result<Text, TextFormError> {
        api_comp(&self.0.display_name)
    }

    fn copy_prefix(&self) -> Result<Text, TextFormError> {
        api_comp(&self.0.prefix)
    }

    fn copy_suffix(&self) -> Result<Text, TextFormError> {
        api_comp(&self.0.suffix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::ops::ControlFlow::{Break, Continue};
    use sunrise_core::nbt::Tag;

    #[test]
    fn client_scoreboard_lends_received_rows_and_copies_text_independently() {
        let mut scores = ScoreState::default();
        let board = &mut scores.board;
        assert!(board.add_objective(
            "points",
            "dummy",
            sb::literal("Points"),
            sb::RenderType::Hearts,
            false,
            Some(sb::NumberFormat::Blank),
        ));
        assert!(board.add_objective(
            "queued",
            "dummy",
            sb::literal("Queued"),
            sb::RenderType::Integer,
            false,
            None,
        ));
        board.set_display_objective("sidebar", Some("points"));
        board.set_display_objective("list", Some("points"));
        let row = board.score_mut("#hidden", "points");
        row.value = -3;
        row.display = Some(sb::literal("Counter"));
        row.number_format = Some(sb::NumberFormat::Fixed(sb::literal("minus three")));
        let row = board.score_mut("Sam", "points");
        row.value = 17;
        // The hover item retains its custom data, but the packet's NBT-to-JSON
        // projection turns the non-finite number into a nested null.
        row.display = Some(sb::tag_to_json(&Tag::compound([
            ("text", Tag::String("Hover for details".into())),
            (
                "hover_event",
                Tag::compound([
                    ("action", Tag::String("show_item".into())),
                    ("id", Tag::String("minecraft:stone".into())),
                    (
                        "components",
                        Tag::compound([(
                            "minecraft:custom_data",
                            Tag::compound([("example", Tag::Double(f64::NAN.to_bits()))]),
                        )]),
                    ),
                ]),
            ),
        ])));
        let team = board.add_team("red");
        team.display_name = sb::literal("Reds");
        team.color = Some("red".into());
        team.allow_friendly_fire = false;
        team.see_friendly_invisibles = true;
        team.nametag_visibility = sb::Visibility::HideForOtherTeams;
        team.collision_rule = sb::CollisionRule::PushOwnTeam;
        team.prefix = sb::literal("[R] ");
        team.suffix = sb::literal(" !");
        assert!(board.add_to_team("guest1", "red"));
        assert!(board.add_to_team("offline", "red"));

        let board: &dyn ClientScoreboard = &ScoreboardView(&scores);
        let mut names = Vec::new();
        assert_eq!(
            board.visit_objectives(&mut |objective| {
                names.push(objective.name().to_owned());
                let mut slots = Vec::new();
                assert_eq!(
                    objective.visit_slots(&mut |slot| {
                        slots.push(slot);
                        Continue(())
                    }),
                    Continue(())
                );
                if objective.name() == "queued" {
                    assert!(slots.is_empty());
                    assert_eq!(objective.render_type(), ObjectiveRenderType::Integer);
                    assert_eq!(objective.copy_number_format(), Ok(None));
                    return Continue(());
                }
                assert_eq!(slots, [DisplaySlot::List, DisplaySlot::Sidebar]);
                assert_eq!(objective.render_type(), ObjectiveRenderType::Hearts);
                assert_eq!(objective.copy_display_name(), Ok(Text::literal("Points")));
                assert_eq!(
                    objective.copy_number_format(),
                    Ok(Some(NumberFormat::Blank))
                );
                let mut rows = Vec::new();
                assert_eq!(
                    objective.visit_scores(&mut |row| {
                        rows.push((row.holder().to_owned(), row.value()));
                        if row.holder() == "Sam" {
                            assert_eq!(row.copy_display(), Err(TextFormError::NestedNull));
                            assert_eq!(row.copy_number_format(), Ok(None));
                        } else {
                            assert_eq!(row.copy_display(), Ok(Some(Text::literal("Counter"))));
                            assert_eq!(
                                row.copy_number_format(),
                                Ok(Some(NumberFormat::Fixed(Text::literal("minus three"))))
                            );
                        }
                        Continue(())
                    }),
                    Continue(())
                );
                // Fastutil traversal uses descending buckets: Sam (31), then #hidden (28).
                assert_eq!(rows, [("Sam".to_owned(), 17), ("#hidden".to_owned(), -3)]);
                Continue(())
            }),
            Continue(())
        );
        // The queued objective is already on the board before a slot is set, and the
        // higher bucket (28) visits before points (16).
        assert_eq!(names, ["queued", "points"]);
        for slot in [DisplaySlot::List, DisplaySlot::Sidebar] {
            assert_eq!(
                board.with_slot(slot, &mut |objective| assert_eq!(
                    objective.name(),
                    "points"
                )),
                Some(())
            );
        }
        assert_eq!(
            board.with_slot(DisplaySlot::BelowName, &mut |_| panic!("empty slot")),
            None
        );

        let mut team_names = Vec::new();
        assert_eq!(
            board.visit_teams(&mut |team| {
                team_names.push(team.name().to_owned());
                assert_eq!(team.color(), Some(TeamColor::Red));
                assert!(!team.friendly_fire());
                assert!(team.see_friendly_invisibles());
                assert_eq!(
                    team.name_tag_visibility(),
                    TeamVisibility::HideForOtherTeams
                );
                assert_eq!(team.collision_rule(), CollisionRule::PushOwnTeam);
                assert_eq!(team.copy_display_name(), Ok(Text::literal("Reds")));
                assert_eq!(team.copy_prefix(), Ok(Text::literal("[R] ")));
                assert_eq!(team.copy_suffix(), Ok(Text::literal(" !")));
                let mut members = Vec::new();
                assert_eq!(
                    team.visit_members(&mut |name| {
                        members.push(name.to_owned());
                        Continue(())
                    }),
                    Continue(())
                );
                // Java HashSet traversal uses ascending buckets: offline (1), then guest1 (10).
                assert_eq!(members, ["offline", "guest1"]);
                Break(())
            }),
            Break(())
        );
        assert_eq!(team_names, ["red"]);
    }
}
