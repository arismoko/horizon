# Loading chunk attachments

`chunk_data_load` receives `&dyn ChunkDiskHost` and a `ChunkDataLoadEvent` after
a full or proto disk record has decoded, before its attachment store is
published. A missing record, fresh generation and proto-to-full promotion do
not fire this hook. The callback uses `event.data.get(&KEY)` and
`event.data.set(&KEY, value)` to read and edit the typed store of this load.
For example, reading disk value 8 and setting 9 publishes 9 on success.

The worker host supplies only the job identity and immutable resource reads.
It has no live backend and is not a `ServerHost`. Values cross the native/wasm
boundary through the generated Wire adapter; no typed Rust pointer crosses it.
Every typed value needs both a persistence codec and the shared `Wire` trait.
Unknown bounded attachment entries are preserved opaquely.

Codec errors, native unwinds and guest dispatch failures refuse the entire
store. The generator must distinguish this refusal from an absent chunk:
publishing freshly generated defaults would lose the saved data. The refused
record stays on disk untouched and the failure is logged with the chunk and the
mod. For the rest of that session the chunk is neither loaded nor generated,
so the player sees a gap in the terrain there, and later saves of that chunk
are refused. Fix the mod and rejoin the world to load it again.

The load runs on the thread that decodes the record, never with access to live
world state. See the [design](../concepts/chunk-attachment-disk.md) for the
budgets (65536 bytes per value, depth 64, 1048576 bytes per chunk) and the
failure policy.
