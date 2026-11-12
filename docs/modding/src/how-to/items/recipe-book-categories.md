# Recipe book category declarations (first slice of #5501)

The `recipe_book_categories` hook declares recipe book categories. Trait mods import
`RecipeBookCategory` from `mod_api` and append declarations in
`Hooks::recipe_book_categories`:

```rust,ignore
event.categories.push(RecipeBookCategory {
    id: "mymod:crushing".into(),
    recipe_type: "mymod:crusher".into(),
    icon: "mymod:crusher".into(),
});
```

Mixin bundles use `native_recipe_book_categories(categories)` at the same hook
point. `host.recipe_book_category("mymod:crusher")` (the mixin helper is
`recipe_book_category`) returns the validated declaration, including its
icon, or `None`. It does not grant recipes. Declarations are requested lazily at
first lookup, cached for that recipe table, and invalidated on a new session,
server recipe-content replacement or server `/reload`. No lookup means no hook
call or per-frame work. A client validates against its own custom recipes unless
`native_datapack_sync` has supplied the server's type.

Every field must be a fully namespaced resource id. Category ids and recipe types
must be outside `minecraft:`; the type must occur in the current custom recipe
table. The icon must name a registered, non-air item. Invalid ids, unknown types,
missing/unknown icons, and duplicate category ids or recipe types are logged and
skipped, never panicked on. The first valid declaration wins each id and type;
an invalid declaration does not reserve either. This slice supports one category
per type, not per-recipe category selectors.

**Not yet book UI support:** custom recipes still have no book displays or
unlock/display-id synchronization. This slice deliberately leaves vanilla tabs,
search, layouts and unlock behavior unchanged. Adding custom tabs, displaying and
searching recipes there, and unlocking them like vanilla remain follow-up work
for #5501; a declaration alone will not make a recipe visible.

For proposed book identities and saved open/filter flags, see the
[custom recipe-book settings design](https://github.com/arismoko/horizon/blob/main/docs/design/mod-api/recipe-book-types.md).
