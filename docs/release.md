# Releasing koklaude

Automated, conventional-commit driven. You never tag or write a changelog by
hand — you merge a PR. Two workflows do the rest.

## The flow

1. **Land work on `main`** with conventional-commit messages (`feat:`, `fix:`,
   `perf:`, …). The commit *type* is what git-cliff uses to compute the next
   semver bump, so write them honestly.
2. **`release-pr.yml`** runs on every push to `main`. git-cliff works out the
   next version from the commits since the last tag. If a release is due, it
   opens (or updates) a PR `chore: release vX.Y.Z` on branch `release/next`,
   labeled `autorelease`, that bumps `CHANGELOG.md` and the workspace version in
   `Cargo.toml`. If nothing release-worthy landed, it does nothing.
3. **Review and merge that PR.** Merging is the release trigger — nothing ships
   until you do.
4. **`tag-release.yml`** fires on the merge (PR closed + merged + `autorelease`
   label):
   - job **`release`** (Linux): tags the merge commit `vX.Y.Z`, pushes it, and
     publishes a GitHub Release whose body is the `--latest` changelog section.
   - job **`binaries`** (macOS, Apple Silicon): builds `cargo build --release`
     and attaches `koklaude-vX.Y.Z-aarch64-apple-darwin.tar.gz` to that release.

That's it. A merged `autorelease` PR ⇒ a tag, a GitHub Release, and a macOS
binary.

## Notes / gotchas

- **`-` in the version ⇒ prerelease.** A tag like `v0.2.0-rc.1` is published as
  a GitHub *pre-release* automatically (the workflow checks for a `-`).
- **Only `aarch64-apple-darwin` today.** The runner (`macos-14`) is Apple
  Silicon and builds natively. x86_64 / Linux / Windows are out of scope while
  playback is macOS-`afplay`-only (see `docs/plan.md` → Later).
- **`Cargo.lock` is intentionally not bumped** in the release PR. Regenerating
  it (`cargo generate-lockfile`) rebuilds the whole dep graph and busts CI
  caches for zero benefit here; the lockfile's local-package version is
  cosmetic and CI never builds `--locked`.
- **Both jobs live in one workflow run** on purpose: a tag pushed with the
  default `GITHUB_TOKEN` does *not* trigger a separate `on: push: tags`
  workflow, so the binary build is wired as a `needs:` job instead.
- **crates.io publish** (`cargo install koklaude` from the registry) is a
  post-1.0 item, not part of this flow yet.

## If something goes wrong

- *PR opened but version looks wrong* → check the commit types since the last
  tag; git-cliff only bumps minor on `feat`, patch on `fix`, major on `!`/
  `BREAKING CHANGE`.
- *Release made but no binary attached* → check the `binaries` job log; a macOS
  build failure leaves the release without its asset. Re-run that job.
- *Need to redo a release* → delete the tag and the GitHub Release, then re-run
  `tag-release.yml`, or push a corrected version PR.
