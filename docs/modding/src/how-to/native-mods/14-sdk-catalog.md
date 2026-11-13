# Machine-readable SDK catalog

Use `mod-catalog.json` to inspect the hooks and injection points for the game build
you target. The catalog ships in the prebuilt SDK and as a standalone asset on the
[Nightly release](https://github.com/arismoko/horizon/releases/tag/nightly).
Download the [catalog asset](https://github.com/arismoko/horizon/releases/download/nightly/mod-catalog.json)
and verify it against that release's signed `SHA256SUMS` and build-specific signature.
For an older pinned build, use its dated Nightly release rather than the latest catalog.
The catalog is generated from the release's exact published commit and follows the
release's retention policy.

## Document fields

| Field | Contents |
| --- | --- |
| `schema_version` | `1`. Reject schema versions your tool does not support. |
| `api_version` | The mod manifest API version, independent of the catalog format version. |
| `hooks` | Hook records, sorted by `id`. |
| `points` | Injection point records, sorted by `name`, then `target`. |

The JSON contains no timestamps or absolute paths.

## Hook records

| Field | Contents |
| --- | --- |
| `id` | Hook name. |
| `side` | `server`, `client` or `both`. |
| `scope` | Part scope with `kind` and `key`, or `null`. Kinds are `block`, `blockentity`, `item`, `menu` and `fluid`. |
| `contract` | `listen`, `cancel`, `collect` or `replace`. |
| `arguments` | Ordered argument objects with `name` and `type`. |
| `output` | Output object with `name` and `type`, or `null`. |

Types are Rust API event field types, including their borrow lifetimes.

## Injection point records

| Field | Contents |
| --- | --- |
| `name` | Point name. |
| `target` | Function path used by an injection's `at`. |
| `side` | Point side; `none` for functions outside `Game`. |
| `host` | Listener host kind: `host`, the side's writing host, or `none`. |
| `record` | API listener record name in `mod_points`. |
| `args` | Ordered pairs of parameter name and type. |
| `returns` | Return control: `cancel`, `result` or `none`. |
| `return_ty` | Function return type, with `()` for no return value. |
| `at_return` | Whether API listeners can run at the return. |
| `api_refusal` | Reason API listeners are refused at registration, or `null` when an API record is available. |
