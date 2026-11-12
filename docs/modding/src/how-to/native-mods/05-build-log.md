# Build log

The full log of a build is `native/build.log`, beside the build's own folders under the launcher's data
folder (`native_mods::log_path`). Each new attempt replaces it. Compiler output and the final result
stay in the log, including failures before compilation or while selecting or exporting the built game.

The launcher has a **Build log** action at the top right. It is there whenever a log exists, also right
after the launcher starts, and it always opens the latest build's log: the next build replaces it. The
panel shows a bounded preview of the end of the log and says when it truncated; **Copy log** copies the
whole file, **Open full log** opens it outside the launcher, **Show in folder** reveals it, and
**Refresh** reads it again (also after a build finishes). The panel is a snapshot read on open and
refresh, not a live stream: a running build's later output appears only on the next refresh. Nothing is
uploaded; the log stays on the player's computer.
