# Register game rule categories

In `Hooks::register`, call `registrar.game_rule_category("machines")` to declare a category in your mod's namespace. A full ID in your own namespace is also accepted. Invalid IDs, another mod's namespace and duplicate declarations refuse mod loading, even if the returned error is ignored.

The category's translation key is `gamerule.category.<namespace>.<path>`. A category appears in the host's in-world game rules screen when it contains a matching rule. Remote clients do not receive categories in this version.

Declare a boolean rule with `registrar.game_rule_bool("enabled", "mymod:machines", true)` and retain its `BoolGameRuleKey`. Read it with `level.game_rule_bool(host, key)`. Category references resolve after all mods register; an unknown category refuses loading at finalization. Rule values are saved with the world; values belonging to a missing mod remain in the save. The world-creation rules screen does not list mod rules yet, so a new world starts with their declared defaults. Typed writes and integer rules are planned separately.
