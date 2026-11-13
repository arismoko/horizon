# Passages within a level

A passage is an opening in a level that leads somewhere else in the same level, as an Immersive Portals portal does. A mod's server side registers it with its level; the engine numbers it and tells every Horizon client that tracks the passage's chunk. Passages into another level are not covered yet.

```rust,ignore
let passage = mod_api::Passage {
    origin: mod_api::Vec3::new(0.5, 65.5, 8.5), // the opening's centre
    axis_w: mod_api::Vec3::new(1.0, 0.0, 0.0),
    axis_h: mod_api::Vec3::new(0.0, 1.0, 0.0),
    width: 2.0,
    height: 3.0,
    destination: mod_api::Vec3::new(100.5, 65.5, 8.5), // where the centre comes out
    turn: mod_api::PassageTurn::Clockwise90,
    ..mod_api::Passage::default()
};
match level.register_passage(host, passage) {
    Ok(id) => { /* keep `id` to remove it with `level.remove_passage(host, id)` */ }
    Err(refused) => log::warn!("portal refused: {refused:?}"),
}
```

- **The shape.** `origin` is the centre of a rectangle, `width` blocks along `axis_w` and `height` along `axis_h`. Both axes are unit vectors at a right angle. The far side is the same rectangle centred on `destination`, turned about the vertical axis by `turn`: a clockwise quarter turn takes south (+z) to west (-x), as vanilla's `Rotation` does. A passage leads one way, so a way back is a second passage.
- **Refusals.** Registration returns a `PassageRegistrationError` and registers nothing:
  - `Stale`: the level handle is from an earlier open of the world.
  - `Unavailable`: the call came from a client.
  - `NotFinite`: a coordinate or a size is NaN or infinite.
  - `BadAxes`: an axis is not a unit vector, or the axes are not at a right angle.
  - `BadSize { max: 32 }`: a side is not above 0 and at most 32 blocks.
  - `OutsideWorld`: a corner at either end is past the positions vanilla's server accepts from a client: 30,000,000 blocks across, 20,000,000 up or down.
  - `LevelFull { limit: 256 }` and `ChunkFull { limit: 8 }`.
  - `NoIdsLeft`: every id was given (`u64::MAX` of them), and an id is never given twice.
- **Caps.** A level holds 256 passages and a chunk 8. The chunk is the one that holds `origin` (`passage.chunk()`). A client is sent every passage of the chunks it tracks, so the level cap bounds what a client holds, whatever its view distance. The chunk cap bounds what goes out with one chunk, and how many passages a client looks through around a moving player.
- **Who hears of it.** At the end of the server's tick, a registration goes to every Horizon client that tracks the passage's chunk (`ChunkMap.isChunkTracked`). A client that gets the chunk later is sent the passage right after the chunk, at the chunk's `chunk_sent`. A client forgets a passage with its chunk, and a removal reaches the clients that still track the chunk. Vanilla clients are never sent passages.
- **Lifetime.** A passage holds until `remove_passage`, until its level closes, or until the world closes; a failed open drops what the open registered. Passages are not saved, and a datapack reload does not bring them back: a mod registers its own again in `level_load`. Numbers are never given again while the game runs. `remove_passage` returns false for an id from another level, an unknown id, a stale handle and a client.
- **Churn.** A passage registered and removed before the end of the tick is never sent. The changes the server keeps for its clients are bounded by the live passages and the removed ones it already told.
- **On the client.** A client keeps a passage only if it passes the same checks as a registration, its chunk is one the client holds, and it has fewer than 256. It drops the rest and logs why.
- **Cost.** Nothing while no passage was registered. From the first one on, until the world closes, the hub records the chunks it sends, as it does while a mod listens to `chunk_watch`. That is a `Watch` entry and a `Sent` entry for each chunk sent, an `Unwatch` entry for each chunk a player stops watching, and a map lookup for each sent chunk's passages.
