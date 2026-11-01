# Native dependency proof

Build these bundles through the launcher on the pinned Horizon revision. Use fresh instance and server data.

1. Install `base_mod` and `user_mod`, without `extra_mod`. Start a world on the server thread and then a dedicated server. In each log, `deps-proof base` must precede `deps-proof user`; the latter must show `counter=1 extra=off` and the appropriate `dedicated` value.
2. Install all three and repeat on both server paths. The user line must show `counter=1 extra=on`, after the base line.
3. Install `user_mod` without `base_mod`; the launcher build must fail and name both `user_mod` and `base_mod`.
4. Build and run horizon-machine-sample's mixin bundle separately; its four cobblestone must become four stone.
