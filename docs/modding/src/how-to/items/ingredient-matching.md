# A mod's own ingredient kinds, matching at craft time

The load-time list only names items. Matching a concrete stack — its components, usually —
uses `Hooks::ingredient_test(context, event)`: the ingredient's `test(host, stack)` reaches it with the loop `context` of the side `host` runs on, the
kind and NBT compound the load-time hook saw, the stack being matched and the verdict, already `true`
(the list named the item). Set `accept` to `false` to refuse the stack (`ICustomIngredient.test`).
The hook may be called many times a craft; the answer must be the same for the same stack.

`data` is an API `Compound`, not a JSON object. Match its fields as typed `Tag`
variants (`Int`, `Long`, `List`, `Compound`, and so on); the game converts the
recipe metadata once at load with vanilla JsonOps-to-NbtOps rules. Numeric JSON
arrays remain lists. Nested null refuses the ingredient and fails the recipe
like an unclaimed kind; matching retains the same compound without reconversion.
For an end-to-end mod test, use [horizon-machine-sample](https://github.com/arismoko/horizon-machine-sample)
and implement `Hooks::ingredient` / `Hooks::ingredient_test` with the typed
compound contract; no JSON signature is needed in a mod.
