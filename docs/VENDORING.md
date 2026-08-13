# Vendored dependencies: how and why

*Plain-language guide for everyone touching this repo — Casey, Claude Code
sessions, and Mez (the cloud Cowork session). Adopted August 13, 2026;
decision 20 in `ARCHITECTURE.md`.*

## The problem this solves

Mez's cloud sandbox cannot reach crates.io, so it needs dependency sources on
disk to build and test (`cargo vendor` output). But committing `vendor/` to
`master` turned out to be expensive in a way that deleting it later cannot
fix: **git history carries every committed blob forever**, and merges carry
history. After the iroh bump, `vendor/` weighed ~559 MB — and any
ancestry-preserving merge toward staging or main would have dragged all of it
along, even if a later commit deleted the folder. Tree weight and history
weight are different things.

## The scheme

- **`master` never contains `vendor/` or `.cargo/`** — both are gitignored.
  Normal builds (Casey, Claude Code, anyone online) just use crates.io;
  nothing special to do.
- **The `vendor-snapshot` branch is an artifact shelf, not history.** It is an
  orphan branch containing only `vendor/` and `.cargo/config.toml`, matching
  the current `Cargo.lock`. It is force-pushed whenever dependencies change,
  merged into nothing, and ancestor of nothing. Its weight stays its own.
- **The cloud session lays the snapshot down as untracked files**:

  ```
  git fetch origin vendor-snapshot
  git restore --source=origin/vendor-snapshot -- vendor/ .cargo/
  cargo test --workspace --offline   # hermetic, as before
  ```

## When dependencies change (Claude Code)

After a dep bump builds green, refresh the shelf:

```
# from the branch where Cargo.lock is current:
cargo vendor vendor
git checkout --orphan vendor-snapshot   # or: git checkout vendor-snapshot
git rm -rfq --cached .
git add -f vendor .cargo/config.toml Cargo.lock
git commit -m "vendor snapshot for Cargo.lock @ <short-sha of source branch>"
git push -f origin vendor-snapshot
git checkout <your working branch>
```

Force-pushing here is fine and expected: the branch is a single-writer
artifact with no downstream ancestry. (If `.cargo/config.toml` doesn't exist
because you're on the de-vendored master, create it first:)

```
[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "vendor"
```

## Graduation: the history boundary

The ~590 MB of vendor blobs already in this fork's history stay its private
cost — they must never ride to staging or main. Publication
(workshop → staging) is therefore a **content boundary, not an ancestry
boundary**: the spike graduates as a fresh, vendor-free snapshot branch (its
final tree as one or a few curated commits), pushed as `spike-<name>` per
TIERS.md. Within staging, and from staging to main, the plain-merge-commit
rule applies with full force. See TIERS.md ("Cross-tier merges preserve
ancestry") for the rule and its carve-out.
