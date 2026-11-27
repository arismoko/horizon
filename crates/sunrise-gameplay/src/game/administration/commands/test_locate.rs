//! `TestCommand`'s `locate`, `pos` and `create`.
use super::{Arg, Context, R, err};
use crate::game::test_instances::TestDef;
use sunrise_game_data::text::{self, ClickEvent, Style, Text};
use sunrise_level::{
    containers::BlockEntity,
    test_instance_block::{Status, TestInstanceBlock},
};
use sunrise_world::worldgen::structure::Rotation;

pub(in crate::game) type Pos = (i32, i32, i32);

/// `Rotation.rotate(Direction.NORTH)` as (dx, dz) and the `toYRot` of the
/// opposite direction.
pub(in crate::game) fn north_turned(rotation: Rotation) -> ((i32, i32), i32) {
    match rotation {
        Rotation::None => ((0, -1), 0),
        Rotation::Clockwise90 => ((1, 0), 90),
        Rotation::Clockwise180 => ((0, 1), 180),
        Rotation::Counterclockwise90 => ((-1, 0), 270),
    }
}

/// `String.format("/execute in %s run tp @s %d %d %d %d 0", ..)` of `locate`.
pub(in crate::game) fn locate_command(dimension: &str, at: Pos, y_rot: i32) -> String {
    format!(
        "/execute in {dimension} run tp @s {} {} {} {y_rot} 0",
        at.0, at.1, at.2
    )
}

/// `Mth.floor(Mth.sqrt(dx * dx + dz * dz))` over ints.
pub(in crate::game) fn horizontal_distance(dx: i32, dz: i32) -> i32 {
    ((dx.wrapping_mul(dx).wrapping_add(dz.wrapping_mul(dz))) as f32)
        .sqrt()
        .floor() as i32
}

/// The text of a clickable bracketed position.
pub(in crate::game) fn link(inner: Text, color: u32, command: String) -> Text {
    Text::literal("[")
        .append(inner)
        .append("]")
        .with_style(&Style {
            click: Some(ClickEvent::SuggestCommand(command)),
            hover: Some(std::sync::Arc::new(Text::translate(
                "chat.coordinates.tooltip",
                &[],
            ))),
            ..Style::color(color)
        })
}

pub(in crate::game) fn path_of(dimension: &str) -> &str {
    dimension.split_once(':').map_or(dimension, |(_, p)| p)
}

impl crate::game::ServerGame<'_> {
    /// `outputPlayerCoordinates`.
    pub(in crate::game::server_side) fn output_player_coordinates(
        &mut self,
        test_dimensions: &[String],
    ) {
        let Some(player) = self.common.player.as_ref() else {
            return;
        };
        let dimension = self.common.dim.id().to_owned();
        // isPlayerInCurrentTestDimension
        if test_dimensions.len() <= 1
            && test_dimensions.contains(&dimension)
            && test_dimensions.iter().any(|d| d == "minecraft:overworld")
        {
            return;
        }
        let at = [
            player.position.x.floor() as i32,
            player.position.y.floor() as i32,
            player.position.z.floor() as i32,
        ];
        let command = format!(
            "/execute in {dimension} run tp @s {} {} {} {} {}",
            at[0],
            at[1],
            at[2],
            self.yaw.floor() as i32,
            self.pitch.floor() as i32
        );
        let coordinates = link(
            Text::translate(
                "test.player.coordinates",
                &[
                    Text::literal(at[0].to_string()),
                    Text::literal(at[1].to_string()),
                    Text::literal(at[2].to_string()),
                    Text::literal(path_of(&dimension).to_owned()),
                ],
            ),
            text::GOLD,
            command,
        );
        self.feedback_unbroadcast(coordinates);
    }

    /// `TestCommand.createNewStructure`.
    pub(in crate::game) fn test_create(&mut self, ctx: &Context) -> R<i32> {
        let int = |name: &str, default: i32| match ctx.get(name) {
            Some(Arg::Int(v)) => *v,
            _ => default,
        };
        let Some(Arg::Resource(id)) = ctx.get("id") else {
            return Ok(0);
        };
        let key = if id.contains(':') {
            id.clone()
        } else {
            format!("minecraft:{id}")
        };
        // `create <id> <width>` is a cube.
        let width = int("width", 5);
        let size = if ctx.get("height").is_some() {
            [width, int("height", 5), int("depth", 5)]
        } else {
            [width; 3]
        };
        if size.iter().any(|&v| v > 48) {
            return Err(err("commands.test.error.too_large", &[&48]));
        }
        let at = self.test_position_around();
        self.create_empty_test(&key, at, size);
        let padding = self.test_holder_padding(at);
        let low =
            TestInstanceBlock::structure_pos_of([at.0 + padding, at.1 + padding, at.2 + padding]);
        let bedrock = self
            .common
            .pack
            .assets
            .registry
            .default_state("minecraft:bedrock");
        if bedrock.is_some() {
            for x in low[0]..low[0] + size[0] {
                for z in low[2]..low[2] + size[2] {
                    self.set_test_block_state(
                        (x, low[1], z),
                        "minecraft:bedrock",
                        sunrise_world::level::flags::UPDATE_ALL,
                    );
                }
            }
        }
        let name = self.test_name(at);
        self.feedback(Text::translate("commands.test.create.success", &[name]));
        Ok(1)
    }
}

impl crate::game::ServerGame<'_> {
    /// `StructureUtils.createNewEmptyTest`.
    pub(in crate::game::server_side) fn create_empty_test(
        &mut self,
        key: &str,
        at: Pos,
        size: [i32; 3],
    ) {
        let padding = self.test_def(key).map_or(0, |d: TestDef| d.padding);
        let corner =
            TestInstanceBlock::structure_pos_of([at.0 + padding, at.1 + padding, at.2 + padding]);
        // getStructureBoundingBox(getStructurePos(testPos), size, NONE)
        let area = sunrise_core::bounding_box::BoundingBox::new(
            corner,
            [0, 1, 2].map(|i| corner[i] + size[i] - 1),
        );
        self.clear_test_space(&area);
        self.set_test_block_state(
            at,
            "minecraft:test_instance_block",
            sunrise_world::level::flags::UPDATE_ALL,
        );
        if self.test_instance_at(at).is_none() {
            self.common.world.block_entities.insert(
                at,
                BlockEntity::TestInstanceBlock(Box::<TestInstanceBlock>::default()),
            );
        }
        if let Some(block) = self.test_instance_mut(at) {
            block.data.test = Some(key.to_owned());
            block.data.size = size;
            block.data.rotation = Rotation::None;
            block.data.ignore_entities = false;
            block.data.status = Status::Cleared;
            block.data.error_message = None;
        }
        self.mark_block_entity_updated(at);
    }
}

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::*;

    #[test]
    fn locate_teleports_three_blocks_in_front_of_the_rotated_north_face() {
        // Rotation.rotate(Direction.NORTH) and the toYRot of its opposite.
        assert_eq!(north_turned(Rotation::None), ((0, -1), 0));
        assert_eq!(north_turned(Rotation::Clockwise90), ((1, 0), 90));
        assert_eq!(north_turned(Rotation::Clockwise180), ((0, 1), 180));
        assert_eq!(north_turned(Rotation::Counterclockwise90), ((-1, 0), 270));
        assert_eq!(
            locate_command("minecraft:the_nether", (1, 64, -3), 90),
            "/execute in minecraft:the_nether run tp @s 1 64 -3 90 0"
        );
    }

    #[test]
    fn the_distance_is_the_floored_horizontal_one() {
        assert_eq!(horizontal_distance(3, 4), 5);
        assert_eq!(horizontal_distance(-1, 1), 1);
        assert_eq!(path_of("minecraft:overworld"), "overworld");
    }
}
