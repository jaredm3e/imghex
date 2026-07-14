# CLAUDE.md

Contribution workflow and conventions for this repo live in **[AGENTS.md](AGENTS.md)** —
read it before making changes. The essentials:

- **One issue → one git worktree → one branch → one PR** (worktrees required when
  multiple agents are active).
- **Be green locally before opening a PR:** `cargo fmt --all --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
  (CI only tests `imghex-core`; run the full workspace yourself.)
- **Post reviews as PR comments** (`gh pr comment`); the maintainer merges — you
  can't self-approve.
- **TDD**, and keep **`imghex-core` a pure decoder** (no GUI/I/O); GUI and editing
  live in `imghex-gui`.

See `README.md` for the architecture overview.
