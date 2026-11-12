# Parts

A part is code a trait mod attaches to one block, block entity, item or mod menu type, its own or vanilla's
([the spec](https://github.com/arismoko/horizon/issues/41)). A mod attaches its parts once, at startup, in
`Hooks::register`. Each part trait implementation must have `#[mod_api::horizon_part]`; the attribute
subscribes it only to the methods it implements. A `BlockEntityPart` also needs a separately annotated
`BlockPart` implementation:

```rust
struct Gauge;
#[mod_api::horizon_part]
impl BlockPart for Gauge {
    fn has_analog_output(&self, _: &dyn LevelHost, event: &mut HasAnalogOutputEvent) {
        event.output = true;
    }
    fn analog_output(&self, _: &dyn LevelHost, event: &mut AnalogOutputEvent) {
        event.output = 15;
    }
}

#[mod_api::horizon_mod]
impl Hooks for MyMod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let stone = r.block_key("minecraft:stone");
        r.block_part(stone, Gauge);
    }
}
```
