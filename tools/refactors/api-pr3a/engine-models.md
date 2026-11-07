# Engine model restoration

The seed scripts in this directory run once on the API PR 3a base. They do not
modify item algorithms. `restore_engine_models.py` restores engine Ingredient
matching, station results and menu records; `restore_recipe_snapshots.py` restores
recipe snapshot records and emits exhaustive field mappings for RecipeRule.

Declared mechanical edits:

- Replace the client-lib Ingredient re-export with an engine-owned record and its
  parse, resolve and stack-matching methods. Share WalkedIngredient's parsing walk
  with the API's fluid ingredient parser; Shape is plain data shared by both.
- Remove API Ingredient resolve, forget_answers and test_in: those methods own
  engine load or matching behaviour. Remove the private IngredientTest trait.
- Replace client-lib station/menu record re-exports only where they carry stacks
  or expose engine slot mutation. Keep plain side and button-context data shared.
- Copy the recipe snapshot records and rule variants into client-lib; rename the
  top-level recipe snapshot to RecipeSnapshot to distinguish it from the engine's
  executable Recipe enum. Rewrite the client-lib snapshot constructors and tests
  to use these records. Engine snapshot enums are exhaustive.
- Emit one game-side match over RecipeRule, mapping every nested value through
  TryIntoApi. Other record mappings use the same conversion boundary.

Hand-written boundary edits follow the seeds: StackHooks callback installation,
ApiSlot adaptation, station hook adapters and tooltip snapshot admission/drawing.
The game remains the only implementation of engine/API stack conversion.
Subscription guards run before snapshot encoding. API writes are validated by
TryIntoEngine against the owning engine Items registry; rejected station writes
cancel the event.

The attribute/enchantment hook tests move to the game boundary, since the
client library no longer owns the conversion. Pure modifier/enchantment helper
tests stay in the client library. The brewing menu test checks its observable
engine take queue; a separate game boundary test checks the dispatched API player
identity and stack snapshot. The vanilla queue cases are retained.

Compiler-guided checks and their outcomes are recorded in the PR verification
report rather than treated as transformation equivalence proof: the API stack
representation intentionally changes, while engine behaviour remains unchanged.
