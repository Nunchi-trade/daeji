# Sync policy

This crate is a **verbatim vendor** of `examples/chat` from
[commonwarexyz/monorepo](https://github.com/commonwarexyz/monorepo).

- **Vendored at:** tag `v2026.4.0`
- **Upstream commit:** `3c4e02ceede03126f524216605a1195e1cee7d0e`
- **Upstream path:** `examples/chat/`

## Rules

1. **Source files (`src/*.rs`, `README.md`, `.gitignore`) are byte-identical
   to upstream.** No Daeji-specific changes are permitted in this crate.
   Symphony / lobby / room / contribution-score work lives in
   `crates/network/daeji-chat/`, not here.

2. **`Cargo.toml` is the only file with permitted edits**, and the edits are
   limited to:
   - Renaming the package to `commonware-chat-upstream` (the unmodified name
     `commonware-chat` clashes with the published crate on crates.io).
   - Resolving `*.workspace = true` to the concrete versions defined in
     upstream's workspace `Cargo.toml`.
   - Replacing `[lints] workspace = true` with the upstream workspace lints
     (`unused-must-use = "deny"`, `rust-2018-idioms = "deny"`) so daeji's
     stricter workspace lints do not reject upstream code.

3. **To re-sync to a newer upstream tag:**
   ```
   git clone --depth 1 --branch <new-tag> \
     https://github.com/commonwarexyz/monorepo /tmp/cw
   cp /tmp/cw/examples/chat/src/{main,handler,logger}.rs \
     crates/network/commonware-chat-upstream/src/
   cp /tmp/cw/examples/chat/{README.md,.gitignore} \
     crates/network/commonware-chat-upstream/
   # Then update version pins in Cargo.toml to match the new upstream
   # workspace, and update the tag/commit references in this file.
   ```

4. **No CI drift check yet.** Future improvement: a CI job that diffs this
   crate's source files against upstream at the pinned tag and fails if they
   diverge.
