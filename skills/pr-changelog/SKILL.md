---
name: pr-changelog
description: >-
  The changelog and PR rules for the terp-rs crate repo. Use when opening or
  updating a terp-rs pull request, when writing release notes or a changelog
  entry, when a changelog check fails, or when deciding whether a change is
  operator-visible. Covers fragment format and categories, the PR title
  convention, the commit-message scopes, and the checks a PR must pass. Not for
  terp-core chain or Go changes, which use terp-core's own
  `changelog/unreleased/` fragments.
---

# PR and changelog (terp-rs)

`terp-rs` is the SDK and contract-crate monorepo submodule. It is released
separately from terp-core, and it has **its own** changelog convention because
its consumers are contract authors and integrators, not chain operators.

## One fragment per PR

After the draft PR number exists, every PR owns exactly one file at
`changelog/unreleased/<PR-number>.md`. The filename is the PR number. Internal
work still gets the file, marked none.

```sh
mkdir -p changelog/unreleased
scripts/ci/check-changelog.sh "$(gh pr view --json number -q .number)"   # scaffold
scripts/ci/check-changelog.sh                                          # validate
```

`check-changelog.sh` is the gate: it fails a PR with no fragment, a
malformed fragment, an unknown category, or a category with no entries. Use
`--scaffold` to write the skeleton and `--help` for the rest.

## Operator-visible, or not

The question is **what does someone integrating terp-rs see?** Not "does it
change code".

| Needs a real fragment | `<!-- changelog: none -->` |
|---|---|
| New or removed crate in the workspace | Tests, fixtures, goldens |
| New/removed/renamed public Rust item | Doc and comment edits |
| Changed message, event, or error name | Internal refactors with no API change |
| Changed default, feature flag, or feature-gated behaviour | CI, scripts, justfile moves |
| New or removed `just` recipe | Formatting, dependency bumps with no API change |
| Changed proto surface (vendored or generated) | Lockfile-only changes |
| New migration requirement | Renamed CI job or workflow name |
| Bug fix to a verified path | New `#[ignore]`d test |

Borderline: a **renamed** public item is `Changed` plus `Removed` if the old
name disappears. A new *optional* feature flag is `Added` if it changes
behaviour for someone who enables it, `none` if nothing is enabled by default.

## Fragment format

```markdown
## Fixed

- Fixed the private-dex constant-product quote overflowing on `u128` amount
  pairs ([#123](https://github.com/terpnetwork/terp-rs/pull/123)).
```

Categories, in the order they should appear: `Added`, `Changed`, `Deprecated`,
`Removed`, `Fixed`, `Security`. Use at most the ones that apply; omit the rest.

Rules:

- Start every entry with a verb in the past tense: `Added`, `Fixed`, `Removed`.
- Describe the observable effect, not the patch. "Fixed a panic in the oracle
  bound check", not "refactored oracle.rs".
- One line per change. If it needs a paragraph, it needs a doc page instead.
- Link the PR. `([#123](.../pull/123))`.
- Prefer `Fixed` when unsure.

### `Security`

Only for a disclosed, fixed vulnerability. Do not describe an undisclosed or
unfixed vulnerability under `Security` — that is a disclosure, not a changelog
entry. Coordinate with the maintainers first.

### Internal only

```markdown
<!-- changelog: none -->

This PR only splits the justfile into per-concern files; no recipe changed
behaviour and no public item moved.
```

The reason must be specific. "Internal only" on its own fails review.

## PR title

```
<type>(<scope>): <imperative summary under 72 chars>
```

`type` is the same vocabulary as the changelog categories: `feat`, `fix`,
`docs`, `refactor`, `test`, `ci`, `build`, `chore`, `perf`, `revert`.

`scope` is the crate or surface, matching the module path: `terp-vsck`,
`private-dex`, `terp-seams`, `hash-market`, `onboard`, `proto`, `ci`, `just`.
Multiple scopes means the PR is too broad — split it.

## Commit messages

One feature per commit, and the commit history is what the changelog is
assembled from. That is the whole reason to keep commits narrow: a reviewer
should be able to read `git log v0.0.2..HEAD` and write the release note
without opening the diff.

```
feat(terp-seams): consolidate five seam fixture crates into one

feat(private-dex): repoint seam SSOT at terp-seams::dex
fix(crosslink): fail closed on incomplete misbehaviour evidence
refactor(ci): rename scripts/act to scripts/ci
docs(private-dex): publish the six Vocs pages
```

- Body explains **why**, not what the diff already shows.
- Note a behaviour change in the body even when the title says `refactor`, so
  the changelog fragment has something to draw from.
- `fix` commits say what was wrong before. `feat` commits say what is now
  possible.
- Land `refactor` and `feat` as separate commits. A refactor that changes
  behaviour is a `feat` or a `fix`.

## Before you open the PR

Gate order, cheapest first:

```sh
just fmt-check          # formatting
just check              # cargo check --workspace --tests
just test-rs            # tier 0, no Docker
just ci-core            # tier 1, what CI's merge gate runs
just ci-extended        # if contracts/ or crates/ changed
just check-protos       # if proto/ changed
```

Then check the diff for the things CI will not catch:

- Vendored protos edited in place rather than synced from terp-core.
- A new package not added to a pack in `scripts/ci/dense-packs.sh`, so CI
  never runs its tests.
- A `#[ignore]` reason that does not name its env gate.
- A test file behind a cfg gate that makes it compile to nothing.
- A `just` recipe that docs reference but nothing defines.

## Do not

- Edit an assembled release note in an ordinary PR. Release notes are written
  when the cut is made.
- Put a fragment under `docs/changelog/`. `docs/` here is site content.
- Ship a `none` fragment on a PR that touches a public item. If you are unsure
  whether something is public, say so in the PR body rather than claiming
  `none`.
- Bundle unrelated features into one PR to save a review. The commit split is
  what makes the changelog honest.
- Add a `Security` entry for an unfixed or undisclosed issue.
