# Reading recipe rules

`recipes_of_type` returns the loaded recipes after pack overrides. Shaped,
shapeless and cooking recipes expose their resolved ingredients and fixed
results. Stonecutting, smithing and special crafting recipes expose a typed
`RecipeRule` instead, including resolved ingredient roles. A smithing or
special rule's result depends on its inputs: its template is not a fixed
stack. Use `LevelRef::recipe_for` for the actual output and remainders of a
crafting grid. Smithing transform's template and addition ingredients can be
absent, requiring empty slots. Trim patterns are either registry references or
inline definitions. Result templates retain their count and component patch;
stonecutting exposes the complete fixed output. A mod-defined recipe type
retains its original `json`. Look up recipes again after a data reload.
