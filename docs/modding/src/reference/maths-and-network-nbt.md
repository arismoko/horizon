# Maths and network NBT

## Portable maths

`mod_api::strict_math` provides the established platform-independent StrictMath
operations: `log`, `log10`, `cbrt`, `hypot`, `atan`, `asin`, `acos` and `atan2`.
They are pure functions with identical bits on native and wasm. It does not expose
`sin`, `cos`, `tan`, `exp`, `pow`, or the AArch64 FMA variants.

`mod_api::java_math` exposes the engine’s vanilla maths functions for native mods,
including `round` and `round_f32`. These are the paths used for vanilla `Math`
call sites; rounding can differ by machine, and universal HotSpot bit equality
is not promised. Calling them on wasm fails to compile and points to `strict_math`.

## The math profile

`host.math_profile()` is a `Host` read, the same on native and wasm and constant for the
process lifetime. Its non-exhaustive `MathProfile` describes the
engine's selection for `sin`, `cos`, `tan`, `exp`, `log`, `log10` and `pow`, without
returning machine/vendor/OS identification:

- `LibmRounding`: platform libm for all seven.
- `CorrectlyRounded`: only `sin`, `cos` and `log` use the correctly rounded path;
  `tan`, `exp`, `log10` and `pow` still use platform libm. This is the
  selection on Windows other than AArch64.
- `Aarch64Fma`: fdlibm with fused multiply-adds for `sin` and `cos`, fdlibm for
  `log` and `log10`, and the Rust `libm` crate for `tan`, `exp` and `pow`.
  Those last three are not advertised as StrictMath. This branch wins on all
  AArch64 targets, including Windows.
- `StrictMath`: fdlibm throughout. No target selects it: every other target uses
  `LibmRounding`.

Every profile retains the engine's Java `pow` special cases. This is information,
not guest math: `java_math` stays native-only. A guest must ship whatever libm,
correctly rounded or FMA operations it needs, or explicitly report that it cannot
reproduce the profile. The read does not extend `strict_math`'s supported operations
or promise universal HotSpot equality. Native reads allocate nothing; wasm uses the
ordinary generated four-byte enum response and import machinery.

## Network NBT

`Tag::to_network_bytes()` writes the engine's nameless network root, preserving
NaN bits and modified UTF-8 (including unpaired UTF-16 surrogates).
Reading follows the engine codec: negative floating zero normalizes to positive
zero, and mixed-list compound wrappers are unwrapped.
`Tag::from_network_bytes(&bytes, None)` returns `(tag, consumed_bytes)`;
bytes following the root remain unread. `Tag::End` encodes as `[0]` and represents
vanilla's null marker. Errors have the exported `mod_api::NbtError` type.

The default read quota is `mod_api::NETWORK_NBT_QUOTA` (2,097,152 bytes), matching
vanilla 26.3 `ByteBufCodecs.TAG` and `NbtAccounter.defaultQuota`. It counts heap
charges, not wire bytes: primitive tag overhead, array contents, UTF-16 strings
and compound keys, list references, and distinct compound entries. Pass
`Some(larger_quota)` for trusted data. Both methods allow 512 container levels;
the writer refuses strings over 65,535 modified UTF-8 bytes. Vanilla applies
heap quotas on reads, so writing does not impose a heap quota.

The implementation follows vanilla 26.3 `NbtIo.readAnyTag`/`writeAnyTag`,
`FriendlyByteBuf.readNbt`/`writeNbt`, `NbtAccounter`, and each `TagType.load`.
Codec calls take time proportional to the encoded data.
