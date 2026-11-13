# Dependencies between mods

A mod can publish functions and statics from its root crate for another mod to call.
Declare dependencies in `horizon-mod.toml`, not `Cargo.toml`:

```toml
[dependencies]
base_mod = "^1"
extra_mod = { version = "^2", optional = true }
```

Requirements use Cargo semver syntax and are checked against each installed mod's `horizon-mod.toml`
`version`, not its Cargo package version. Content-only mods use these declarations to validate their
requirements and load their layers after dependencies. Code dependencies additionally require API packages
for the dependent and every installed dependency.
Missing required dependencies, incompatible versions and cycles stop the build with the mods named.
An absent optional dependency is permitted; an installed optional dependency must match its requirement.

The launcher adds a path dependency using the mod ID as the Cargo alias and the target's actual Cargo
package name. Optional dependencies also get a Cargo feature named exactly as their mod ID, enabled
only when that mod is installed. Guard calls with `if_installed!`, which builds the block only with
that feature and runs it only when the mod is installed, so the same source works when built to
`mod.wasm`, where the library is linked at load and an absent one's functions trap:

```rust,ignore
mod_api::if_installed!(host, extra_mod, {
    extra_mod::register_extra(host);
});
```

Do not define these dependencies or features yourself in `Cargo.toml`.
Public statics must use appropriate synchronization for shared and reentrant hooks; private state in
another mod's hook object is not exposed.

Hook listeners run in descending priority, then dependency installation order, with dependency-order ties kept in bundle order. Mixin injection order remains
horizon-mixins' priority, mod ID, then injection ID for source splices. Function-slot listeners
instead use descending priority, dependency/registration order, then injection ID within a mod;
call-site listeners retain mod-ID tie ordering.
