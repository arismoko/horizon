//! The game test instance registry's entries: `GameTestInstance`'s codec.
use crate::test_environments::{self, Environment};
use sunrise_core::identifier::Identifier;
use sunrise_game_data::gamerules::GameRules;
use sunrise_game_data::text::Text;
use sunrise_world::worldgen::structure::Rotation;

/// The `GameTestInstance` a registry entry describes: `TestData` and what
/// `describe()` prints.
#[derive(Clone)]
pub struct TestDef {
    pub structure: String,
    pub rotation: Rotation,
    pub padding: i32,
    pub required: bool,
    pub sky_access: bool,
    /// `BlockBasedTestInstance` (else a `FunctionGameTestInstance`).
    pub block_based: bool,
    /// The function a function test runs.
    pub function: Option<String>,
    pub callback: Option<std::sync::Arc<dyn mod_api::GameTestFunction>>,
    /// The test environment's registered id (`batch().getRegisteredName()`),
    /// or `"[unregistered]"` for an inline environment.
    pub environment: String,
    /// Inline `TestEnvironmentDefinition` when `environment` was a JSON object.
    pub inline_environment: Option<Environment>,
    pub max_ticks: i32,
    pub setup_ticks: i32,
    /// `TestData.maxAttempts` and `requiredSuccesses`.
    pub max_attempts: i32,
    pub required_successes: i32,
    /// The dimension the test runs in (`TestData.dimension`).
    pub dimension: String,
    pub description: Text,
}

/// `GameTestInstance`'s codec, as the game reads a `test_instance` file.
pub fn parse_test_def(json: &serde_json::Value) -> Option<TestDef> {
    parse_test_def_with_env(json, &|_| None, &GameRules::default())
}

/// `GameTestInstance`'s codec, resolving inline `environment` objects against `load_env`.
pub fn parse_test_def_with_env(
    json: &serde_json::Value,
    load_env: &dyn Fn(&str) -> Option<serde_json::Value>,
    rules: &GameRules,
) -> Option<TestDef> {
    let fields = json.as_object()?;
    let kind = Identifier::parse(fields.get("type")?.as_str()?).ok()?;
    if kind.namespace() != "minecraft" {
        return None;
    }
    let block_based = match kind.path() {
        "block_based" => true,
        "function" => false,
        _ => return None,
    };
    let function = if block_based {
        None
    } else {
        Some(
            Identifier::parse(fields.get("function")?.as_str()?)
                .ok()?
                .to_string(),
        )
    };
    let structure = Identifier::parse(fields.get("structure")?.as_str()?)
        .ok()?
        .to_string();
    let env_value = fields.get("environment")?;
    let (environment, inline_environment) = if let Some(id) = env_value.as_str() {
        (Identifier::parse(id).ok()?.to_string(), None)
    } else if env_value.is_object() {
        let parsed = test_environments::parse(env_value, load_env, 0, rules)?;
        ("[unregistered]".to_owned(), Some(parsed))
    } else {
        return None;
    };
    let dimension = match fields.get("dimension") {
        None => "minecraft:overworld".to_owned(),
        Some(v) => Identifier::parse(v.as_str()?).ok()?.to_string(),
    };
    let int_range = |key: &str, min: i32, max: i32, default: Option<i32>| -> Option<i32> {
        match fields.get(key) {
            None => default,
            Some(v) => {
                let n = v.as_i64().and_then(|i| i32::try_from(i).ok())?;
                (min..=max).contains(&n).then_some(n)
            }
        }
    };
    let opt_bool = |key: &str, default: bool| -> Option<bool> {
        match fields.get(key) {
            None => Some(default),
            Some(v) => v.as_bool(),
        }
    };
    let max_ticks = int_range("max_ticks", 1, i32::MAX, None)?;
    let setup_ticks = int_range("setup_ticks", 0, i32::MAX, Some(0))?;
    let required = opt_bool("required", true)?;
    let _manual_only = opt_bool("manual_only", false)?;
    let sky_access = opt_bool("sky_access", false)?;
    let max_attempts = int_range("max_attempts", 1, i32::MAX, Some(1))?;
    let required_successes = int_range("required_successes", 1, i32::MAX, Some(1))?;
    let padding = int_range("padding", 0, 128, Some(0))?;
    let rotation = match fields.get("rotation") {
        None => Rotation::None,
        Some(v) => Rotation::from_name(v.as_str()?)?,
    };
    // GameTestInstance.describe: the type, the function (function tests
    // only), then the structure, batch and dimension.
    let mut description = Text::translate(
        "test_instance.description.type",
        &[Text::translate(
            if block_based {
                "test_instance.type.block_based"
            } else {
                "test_instance.type.function"
            },
            &[],
        )
        .with_color(0x5555FF)],
    )
    .append(Text::literal("\n"));
    if let Some(function) = &function {
        description = description.append(row("test_instance.description.function", function));
    }
    description = description
        .append(row("test_instance.description.structure", &structure))
        .append(row("test_instance.description.batch", &environment))
        .append(row("test_instance.description.dimension", &dimension));
    Some(TestDef {
        structure,
        rotation,
        padding,
        required,
        sky_access,
        block_based,
        function,
        callback: None,
        environment,
        inline_environment,
        dimension,
        max_ticks,
        setup_ticks,
        max_attempts,
        required_successes,
        description,
    })
}
/// `GameTestInstance.descriptionRow`: the row's value in blue, then a line break.
fn row(key: &str, value: &str) -> Text {
    Text::translate(key, &[Text::literal(value).with_color(0x5555FF)]).append(Text::literal("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_print_the_value_in_blue_then_a_line_break() {
        let text = row("test_instance.description.structure", "minecraft:empty");
        assert!(text.plain().ends_with('\n'));
    }
}
