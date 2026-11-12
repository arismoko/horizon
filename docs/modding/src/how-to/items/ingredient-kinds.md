# A mod's own ingredient kinds

A recipe's ingredient that names a type in a namespace outside `minecraft:` reaches
`Hooks::ingredient(context, event)` at load time. Fill `event.items` with the item ids the
ingredient matches (item ids only: components are not checked; a match-time hook follows
separately). Duplicates are removed keeping first-seen order. The hook gets the `DataLoadContext` of the load that parses the recipes and hears each occurrence of the kind once per load, nested ones and ones under any slot a recipe reads an ingredient from included, before the load's tables parse the recipes: the client's own recipe load has `origin()` `ClientPreload`, the server's `ServerDataLoad`. A server loads its recipes once however many tables and copies read them (its recipe tables, the stations and the copy synchronised to guests, on a dedicated server too), and a `/reload` asks again once; a client that only joins a remote server is never told `ServerDataLoad`. The answer must be the same every time. An unclaimed kind leaves the list empty and the recipe fails, as an unknown kind does.

`data` is an API `Compound`, not a JSON object. Match its fields as typed `Tag`
variants (`Int`, `Long`, `List`, `Compound`, and so on); the game converts the
recipe metadata once at load with vanilla JsonOps-to-NbtOps rules. Numeric JSON
arrays remain lists. Nested null refuses the ingredient and fails the recipe
like an unclaimed kind; matching retains the same compound without reconversion.
For an end-to-end mod test, use [horizon-machine-sample](https://github.com/arismoko/horizon-machine-sample)
and implement `Hooks::ingredient` / `Hooks::ingredient_test` with the typed
compound contract; no JSON signature is needed in a mod.
