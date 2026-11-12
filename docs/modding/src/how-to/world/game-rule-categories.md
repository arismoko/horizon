# Register game rule categories

In `Hooks::register`, call `registrar.game_rule_category("machines")` to declare a category in your mod's namespace. The call returns `Result<(), GameRuleError>`. A full ID in a manifest-owned namespace is also accepted.

Declare a boolean rule with `registrar.game_rule_bool("enabled", "mymod:machines", true)`. Handle its `Result<BoolGameRuleKey, GameRuleError>` and retain the key from `Ok(key)`. Read the value with `level.game_rule_bool(host, key)`, which returns `Option<bool>`.

| Rule | Behavior |
| --- | --- |
| Invalid IDs, unowned namespaces or duplicate declarations | Registration returns `GameRuleError::InvalidId`, `ForeignNamespace` or `Duplicate`; loading is refused even if you ignore the error. |
| Category references | Resolved after all mods register; an unknown category refuses loading. |
| Translation | The category key is `gamerule.category.<namespace>.<path>`. |
| In-world game rules screen | The host displays a category when it contains a matching rule. Remote clients do not receive mod categories. |
| World creation | Mod rules use their declared defaults; the world-creation rules screen does not list them. |
| Persistence | Values are saved with the world, including values belonging to a missing mod. |
