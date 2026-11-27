//! The shared per-state resolver: builds a table from a state-only `BlockPart` method, one entry
//! per block state or per (state, fluid) pair, so the code that reads it (path finding, the
//! meshing workers) dispatches to no mod. A table is built when parts install (world load,
//! `/reload`) or when its owner refreshes it, never per tick or per frame, and only where a part
//! subscribes: with none, there is no table and every reader keeps the block's own answer.
