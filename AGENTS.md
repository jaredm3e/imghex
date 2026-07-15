# Contributing to imghex (agent workflow)

This repo is developed by multiple Claude Code agents working **in parallel**,
plus one maintainer who reviews and merges. These conventions keep parallel work
from colliding and keep every change landing green. Follow them exactly.

If anything here disagrees with what you were told in a prompt, the prompt wins —
but tell the maintainer so this file gets fixed.

## The unit of work: one issue → one worktree → one branch → one PR

Pick up a single open issue and do all of its work in an **isolated git
worktree** so you never share a checkout with another agent:

```sh
# From the main checkout, create a sibling worktree on a fresh branch.
git worktree add ../imghex-<issue-number> -b <branch-name> origin/main
cd ../imghex-<issue-number>
```

- **Worktrees are required, not optional**, whenever more than one agent may be
  active — which is the normal case here. Two agents editing the same working
  tree will clobber each other. A worktree gives you your own files while sharing
  the one `.git`.
- **Stay inside your worktree.** Only edit files under your own worktree path;
  never touch the primary checkout, and leave `main` clean. A stray edit there
  corrupts the tree other agents share (watch for symlinks pointing back to it).
- **Branch naming:** short and descriptive, e.g. `jpeg-dqt-fields`,
  `hex-edit-overwrite`. One branch per issue.
- When the branch is merged, clean up: `git worktree remove ../imghex-<issue>`.

## Before you open a PR: be green locally

CI (`.github/workflows/ci.yml`) runs these on every PR and **fails the build on
any warning**. Run all three yourself first:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings   # -D warnings: warnings fail
cargo test --workspace
```

Note: CI only runs `cargo test -p imghex-core`, so **you** are responsible for
running the full `--workspace` test suite (the `imghex-gui` crate is not gated by
CI). Don't rely on CI to catch a gui-side break.

## Pull requests

- **One PR per issue**, targeting `main`. Link the issue (`Closes #N`).
- Write the description to be **self-contained**: another agent (or the
  maintainer) should understand the change without the originating conversation.
  State what changed, why, and how you tested it.
- End the PR body with:

  ```
  🤖 Generated with [Claude Code](https://claude.com/claude-code)
  ```

- End commit messages with:

  ```
  Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
  ```

## Reviews

- Post review feedback as a **PR comment** (`gh pr comment <n>`). GitHub blocks
  approving your own PR, and agents share the `jaredm3e` identity, so a formal
  "Approve" isn't available — a thorough comment is the deliverable.
- **Never merge your own PR.** Who merges is set by the merge policy below.
- When you review, verify the claims: run the tests, `fmt`, and `clippy` against
  the branch rather than trusting the description.

## Merging

A PR merges only after it is green and has passed review with any **blocking**
findings fixed. Who performs the merge depends on what the change touches:

- **Pure-decoder changes (`imghex-core` only)** — parser/field/summary work
  covered by the automated tests and code review. Once green and review-clean,
  the coordinating reviewer may merge these directly; there's no runtime behavior
  a human needs to eyeball.
- **GUI / interactive changes (`imghex-gui` runtime behavior)** — anything whose
  correctness shows up only when the app runs (editing, keyboard/mouse handling,
  dialogs, the rendered preview and bit-plane views). egui can't be driven in CI,
  so these are **held for the maintainer to manually test before merge**, even
  when green and review-clean. The PR description must carry a short **manual
  test checklist** (edit a byte → …, Save → reopen → …) so the maintainer can run
  through it quickly.
- When several GUI PRs are ready at once, they are **queued together** and handed
  to the maintainer as a batch with each PR's checklist, rather than one at a
  time, so review sessions stay efficient.

## Coding conventions

- **TDD.** Add tests alongside the change. Integration tests live in
  `imghex-core/tests/<format>_tests.rs`; mirror the existing pattern (build a
  fixture, `parse_auto`, assert on regions/fields/summary).
- **Keep `imghex-core` a pure decoder** — no GUI dependencies, no I/O. Parsers
  are `fn parse(&[u8]) -> Result<ParsedImage, _>`. GUI/editing/state lives in
  `imghex-gui`.
- **Match the surrounding code — mirror the nearest sibling.** When extending an
  existing area, copy the closest existing pattern (e.g. a new JPEG segment
  decoder mirrors `decode_dqt`); match naming, comment density, and idioms. Read
  the neighboring code before writing.
- Adding a new format is one trait impl in `imghex-core/src/formats/` plus one
  line in `format::registry()`. See `README.md` for the architecture tour.

## Backlog

Open issues describe the current work. Each is written to be picked up cold —
read the whole issue, including its "Display requirement" / phasing notes, before
starting.
