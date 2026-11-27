# Publishing your mod

Use [the Sunrise mod registry](https://playsunrise.net/mods) to distribute a content or native mod. Both ship as
`.smod` packages. A native release carries its source and its `Cargo.lock`, never compiled libraries: each player's
computer builds a native mod for the exact game it runs.

## Registering your repository

Register a repository once, before its first release. Its default branch must already hold, for each package:

- `sunrise-mod.toml`, which `cargo sunrise-mod check` accepts;
- a complete [`page.toml` and screenshots](#your-mods-page);
- the release workflow caller. `cargo sunrise-mod new` writes it for a new project. For an existing repository or
  a package in a subfolder, run `cargo sunrise-mod release setup` from the mod's folder on a clean tree, then
  commit and push the result.

The repository must be public. Sign in at [Submit a mod](https://playsunrise.net/mods/submit) with a GitHub account
that administers it, and enter its URL. A repository with packages in subfolders lists one package folder per line,
with `.` for a package at the root. The form checks those files on the default branch, then opens a registration
pull request on [sunrise-org/sunrise-mods](https://github.com/sunrise-org/sunrise-mods) that gives each package a
permanent project ID. A reviewer approves it after checking that you administer the repository and that the
project is not claiming someone else's. This approves no code: each release is reviewed on its own. Two separate
projects may use the same mod ID, so a port can sit beside the mod it replaces.

You can also register by opening a
[Register a repository](https://github.com/sunrise-org/sunrise-mods/issues/new?template=register.yml) issue on
sunrise-org/sunrise-mods, from a GitHub account that administers the repository. Without a browser, for example
from an agent, open it with the [GitHub CLI](https://cli.github.com/). `gh issue create --template` cannot fill an
issue form's fields, so write the body with the form's headings:

```sh
gh issue create -R sunrise-org/sunrise-mods \
  --title "Register <owner>/<repository>" \
  --body "### Repository

https://github.com/<owner>/<repository>

### Package folders

_No response_"
```

Leave `_No response_` under **Package folders** for a mod at the repository's root; otherwise list one folder per
line, each holding a `sunrise-mod.toml`. Registration reads the default branch only. A maintainer then checks that
you administer the repository, opens the registration pull request and links it on the issue, so this route waits
for a person where the form answers at once. Registering lists nothing: your project appears on Discover once its
first release is reviewed and merged.

## Releasing

`release` is in the [newest tool from main](../start/install-tool.md#newest-tool-from-main). A Nightly's tool that
lacks it answers `"release" is not a cargo sunrise-mod command`.

Commit your mod's source, its page, and `Cargo.lock` for a native mod. The lock must match the pinned SDK:
`cargo sunrise-mod pin` keeps it so, and after moving `[native] sunrise` by hand, `cargo sunrise-mod pin <build>`
with the build already pinned regenerates it. From the mod's folder on a clean branch
that is up to date with its upstream:

```sh
cargo sunrise-mod release patch --check
cargo sunrise-mod release patch
```

Use `minor`, `major`, or a higher SemVer instead of `patch` to choose another version. `--check` runs the
release checks for the proposed version without changing the checkout. Without it, `release` refreshes the
workflow, commits the version bump signed off (`Signed-off-by`, from your Git `user.name` and `user.email`), and pushes the branch, then the tag, to `origin` in two pushes: GitHub starts no workflow for a tag pushed together with the commit that first adds its workflow file. The tag is `v<version>`
at the repository root or `<id>/v<version>` for a subfolder package. GitHub Actions builds and attests both the
`.smod` and its page bundle, with no signing key for you to manage.

`release` reads your repository's ID from GitHub's API. Set `GH_TOKEN` or `GITHUB_TOKEN` when you release
many packages in an hour: with a token GitHub allows far more than the 60 requests an hour it allows without one.
The token is sent to `api.github.com` alone.

If your default branch accepts changes only through pull requests, add `--pull-request`:

```sh
cargo sunrise-mod release patch --pull-request
```

The release commit then goes on a new branch, `release/<tag>`, which is pushed before the tag, and your
checkout is left on that branch. The tag starts the release as usual. Open the pull request it prints, from
`release/<tag>` into the branch you released from, and merge it before your next release, so that branch carries
the new version.

For each later release, run `release` again. Each run reads the registry's current shared workflow version and
updates your caller in the release commit when it has moved, so you never edit the workflow by hand.

## After the tag

1. The tag runs [the shared release workflow](https://github.com/sunrise-org/sunrise-mods/blob/main/.github/workflows/release.yml)
   in your repository. It checks the mod with `cargo sunrise-mod check --release` and builds the `.smod` and the
   page bundle. A separate job, which never unpacks or runs what the build produced, signs both files with a GitHub
   artifact attestation that names your repository and the shared workflow, then publishes them on a GitHub
   release with their attestation bundles. A native `.smod` holds source and `Cargo.lock`, never a compiled library.
2. The registry bot watches registered repositories. For a new release it opens a pull request on
   sunrise-org/sunrise-mods that adds the release and updates the project's page. A release whose package is of
   another kind than the project (a content mod that gained `[native]` code, or the reverse) changes the project's
   kind in the same pull request and removes the project's earlier releases.
3. The bot's checks run on that pull request. They are static checks plus a compile, and never load the mod in the
   game:
   - the files' format, sizes and SHA-256 hashes, and both attestations against the registered repository, the
     shared workflow and the release tag;
   - a scan of the source that never builds or runs it. A dependency from outside crates.io (a path outside the
     package included), a stale `Cargo.lock`, or a crate listed as malicious fails the release. Other findings,
     such as network or process use, `unsafe`, build scripts and changed dependencies, are flagged for the reviewer;
   - a build of a native mod against its pinned SDK on Linux, and a compile check on Windows and macOS.
4. A reviewer reads the release's code with the scan's flags and its diff from your last approved release. The
   review looks only for malware, dangerous patterns and suspicious or obfuscated code. Bugs and suggestions
   arrive as issues on your repository, never as a reason to hold the release.
5. Once the pull request merges, the registry works out which game builds the release fits and publishes it in its
   signed index. A release is offered only for the builds it fits. Players' computers build a native mod from its
   source for the exact game they run.

To check a released package and its attestation yourself, use
[`check <file.smod>`](../reference/sdk/05-commands.md#check-a-released-package).

Once several of your mods are listed, you can show them as one card in Discover: see
[Group your mods into a collection](collections.md).

## Your mod's page

For a tagged registry release, edit `page.toml` beside `sunrise-mod.toml`:

- `summary`: a short description of at least 25 characters.
- `description`: a long description of at least 125 characters. You can use Markdown. Without it the page
  uses the manifest's `description`.
- `categories`: an optional list of up to eight distinct IDs from the registry's `registry/categories.toml`.
  Each category must apply to your package kind (`mod` or `native-mod`).
- `[[screenshots]]`: at least one entry, with a `path` such as `page/recipes.png` and an optional `caption`.
  Put the images under `page/`. Use readable PNG (`.png`) or JPEG (`.jpg` or `.jpeg`) files, at most 5 MiB each.

Text minimums count Unicode scalar values after trimming surrounding whitespace. `description`, `name`,
`authors`, `homepage` and `issues` default to the values in `sunrise-mod.toml`; set them in `page.toml` to
show different ones on the page. The name and each listed author must not be
blank. Links must be absolute HTTP(S) URLs without credentials. The page uses the mod's own `icon.png`, which a release needs, as it needs a `license` in
`sunrise-mod.toml`. Screenshot paths must be distinct, must not be `page/icon.png`, and must be names Windows
can create, and the page bundle must fit the registry's 64 MiB for one release file.

Run `cargo sunrise-mod check` to see what you still need before release. Run
[`cargo sunrise-mod check --release <tag>`](../reference/sdk/05-commands.md) to require a complete page and
check the tag against the manifest's version. `build` writes `target/sunrise-mod/<id>.page.tar` separately from the
`.smod` whenever the project has a `page.toml`.
