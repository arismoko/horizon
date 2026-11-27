# Group your mods into a collection

A collection groups several of your published mods in the registry under one name and icon, so players find and
install them together. Each member stays its own project, with its own page, releases and updates. A collection has no
file or version of its own.

## Who can make one

Every member must be published by the same GitHub account: the account that owns the repository each project is
registered to. Only that publisher curates the collection. The registry refuses a pull request that adds, changes or
removes a collection unless its author is listed in `owners` of every member's `project.toml`, both before and after
the change, as `owners` stands on the registry's `main` branch. You can't add someone else's mod to your collection.

## Write the collection

Keep the collection in a folder of one of your mod repositories, for example `collection/`, and commit it there like
the rest of your mods. In that folder, run:

```sh
cargo sunrise-mod collection
```

The first run writes `collection.toml` with a `[[member]]` for each project from this repository with a
non-revoked release in the signed registry index, then stops. Fill in `name` and `summary`, add any published
members from your other repositories, and put the icon beside it. If the scaffold contains `member = []`,
replace that line with the `[[member]]` entries rather than keeping both:

- `collection.toml`:

  ```toml
  id = "Qm4tZ81c"
  name = "Wayfinding"
  summary = "The map and its markers."
  description = """
  Optional longer text shown when a player opens the collection.
  """

  [[member]]
  project = "Ab3dE9xQ"  # waymarks

  [[member]]
  project = "Zz91kLmN"  # minimap
  default = false
  ```

  `name` and `summary` must not be blank, and `description` is optional. Each `[[member]]` names a project with a
  non-revoked release in the signed registry index. Use the permanent project ID from the folder name in
  `registry/projects/<project ID>/project.toml` on
  [sunrise-org/sunrise-mods](https://github.com/sunrise-org/sunrise-mods/tree/main/registry/projects), not the
  in-game mod ID. Members appear in the order you list them. `default` says whether a member starts ticked,
  and is `true` when you leave it out. You need at least two different members, and at least one of them selected by
  default. Any other key is refused.
- `icon.png`: a square PNG of at most 512×512 pixels and 256 KiB, the same rules as a mod's icon.

## Propose it

Run `cargo sunrise-mod collection` again. When the file has no `id` yet, the command gives the collection a new,
permanent one and writes it into `collection.toml`, keeping your comments: commit the file. It then checks the
collection with the registry's own rules and stops with every problem it finds. `--check` stops after the checks.

The pull request comes from your fork of sunrise-org/sunrise-mods: fork it once, under the account that owns your
repository's `origin`. The command commits the two files to `registry/collections/<id>/` on top of the registry's
`main`, signed off, pushes them to the branch `collection-<id>` of that fork, and prints the link that opens the pull
request. It also names who may open it: the accounts listed in `owners` of every member. When your repository belongs
to an organisation without a fork, it says so and where the two files go, so one of those accounts can open the pull
request from their own fork. The registry checks the folder again and that the pull request's author owns every member;
a maintainer reviews and merges it.

To change a collection, edit the files and run the command again: it replaces the branch on your fork. To remove one,
delete its folder in a pull request.

The registry publishes a collection only once at least 2 members, including a default, have a release.
