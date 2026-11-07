# Owned-stack Part fixture edits

`migrate_part_fixtures.py` applies the mechanical field-read seed on the PR 3a
fixture base at `be6ebcd5e0`. The compiler-guided mutation edits then use the
ordinary owned-stack operations:

- `grow` and `shrink` replace direct count mutation and zero-count sentinels.
- `split` returns a taken value and optional remainder for extraction.
- Mock transfer backends take `ItemResource` plus an amount. A transfer request
  or remainder may exceed a resident stack's maximum; the fixtures retain their
  existing slot capacities and partial-acceptance behavior.
- Mock item metadata reads take `ItemKey`, not string item names.
- Component-patch assertions inspect the API-owned patch, not the engine codec.

The test stack factory resolves an explicit list of fixture item IDs through the
normal registry. API unit tests install fresh thread-local Parts before using
it, preserving registration and key resolution through the same machinery as
other API tests. The factory never creates keys from numeric slots.

The existing one-item-per-call handler test now offers 128 items to a slot whose
capacity is 64. It checks the backend's 127-item remainder in an aborted
transaction, then checks that the public handler reports one item moved and that
the committed resident slot contains exactly one item.

Focused verification: 25 item Part tests, five block entity Part tests, two API
container model tests, and the fishing, host dispatch and item-use dispatch tests
passed. No full workspace test run was used.
