# Rename Checklist

Current name: **wundler**  
Status: Provisional — pending potential acquisition of `cloudpack` name or another decision.

When the rename happens, this file is the single source of truth. Work through it top to bottom.

---

## 0. Before You Start

Pick the new name. Call it `$NEW` below. It goes in three forms:
- `$NEW` — the new lowercase name (e.g. `cloudpack`)
- `$NEW_UPPER` — PascalCase for prose (e.g. `Cloudpack`)
- `$NEW_SCREAMING` — SCREAMING_SNAKE for any env vars (e.g. `CLOUDPACK`)

---

## 1. Repository & Directory

| What | Current | Action |
|---|---|---|
| Repo root directory | `wundler/` | `mv wundler/ $NEW/` |
| GitHub repo name | `microsoft/wundler` | GitHub Settings → Rename repository |
| Git remote URL | `git@github.com:microsoft/wundler.git` | `git remote set-url origin git@github.com:microsoft/$NEW.git` |

---

## 2. Document Files to Rename

5 files. All in `docs/`:

```bash
git mv docs/wundler-architect-brief.md     docs/$NEW-architect-brief.md
git mv docs/wundler-architect-brief.docx   docs/$NEW-architect-brief.docx
git mv docs/wundler-executive-summary.md   docs/$NEW-executive-summary.md
git mv docs/wundler-executive-summary.docx docs/$NEW-executive-summary.docx
git mv docs/superpowers/specs/2026-05-13-wundler-design.md \
       docs/superpowers/specs/2026-05-13-$NEW-design.md
```

---

## 3. Plan Files to Rename

The 5 implementation plans use `wundler-*` crate names and `wundler.toml` throughout. The filenames themselves don't include "wundler", but the content is saturated (~223–350 occurrences each). Bulk-replace content after renaming, or regenerate the plans after the crate names are finalized.

| File | Occurrences |
|---|---|
| `docs/superpowers/plans/2026-05-13-phase-1-summarizer.md` | 223 |
| `docs/superpowers/plans/2026-05-13-abs.md` | 266 |
| `docs/superpowers/plans/2026-05-13-graph-analyzer.md` | 280 |
| `docs/superpowers/plans/2026-05-13-level-0-pipeline.md` | 350 |
| `docs/superpowers/plans/2026-05-13-pgo-store.md` | 234 |

Bulk replace all plan content:
```bash
for f in docs/superpowers/plans/*.md; do
  sed -i '' "s/wundler/$NEW/g; s/Wundler/$NEW_UPPER/g" "$f"
done
```

**Verify after:** Check that `$NEW.toml` (was `wundler.toml`) and `#[command(name = "$NEW")]` read correctly.

---

## 4. Rust Workspace — Crate Names

These crates don't exist yet (only in plans), but when the workspace is initialized all of these need the new name.

### Workspace `Cargo.toml` members

```toml
# Find in: Cargo.toml (workspace root, created in Plan 1 Task 1)
members = [
    "crates/wundler-core",       # → crates/$NEW-core
    "crates/wundler-cli",        # → crates/$NEW-cli
    "crates/wundler-graph",      # → crates/$NEW-graph
    "crates/wundler-transform",  # → crates/$NEW-transform
    "crates/wundler-pipeline",   # → crates/$NEW-pipeline
    "crates/wundler-abs",        # → crates/$NEW-abs
    "crates/wundler-pgo",        # → crates/$NEW-pgo
    "crates/wundler-sw",         # → crates/$NEW-sw  (TypeScript crate)
]
```

### Per-crate `Cargo.toml` `name =` fields

| Crate dir | `name =` to change |
|---|---|
| `crates/wundler-core/Cargo.toml` | `name = "wundler-core"` → `name = "$NEW-core"` |
| `crates/wundler-cli/Cargo.toml` | `name = "wundler-cli"` → `name = "$NEW-cli"` |
| `crates/wundler-graph/Cargo.toml` | `name = "wundler-graph"` → ... |
| `crates/wundler-transform/Cargo.toml` | |
| `crates/wundler-pipeline/Cargo.toml` | |
| `crates/wundler-abs/Cargo.toml` | |
| `crates/wundler-pgo/Cargo.toml` | |
| `crates/wundler-sw/Cargo.toml` | |

### Per-crate `Cargo.toml` dependency declarations

Every `wundler-*` crate depends on one or more of the others. Each dependency line like:
```toml
wundler-core = { path = "../wundler-core" }
```
becomes:
```toml
$NEW-core = { path = "../$NEW-core" }
```

Bulk fix after crate dirs are renamed:
```bash
find crates/ -name "Cargo.toml" -exec \
  sed -i '' "s/wundler-/$NEW-/g" {} \;
```

### Crate directory renames

```bash
for d in crates/wundler-*; do
  mv "$d" "crates/$NEW-${d#crates/wundler-}"
done
```

---

## 5. Rust Source Code

### `use` declarations and `extern crate`

```rust
// In every .rs file that imports from wundler-* crates:
use wundler_core::types::*;
use wundler_graph::types::ChunkManifest;
// etc.
```

Note: Cargo converts hyphens to underscores in crate names for `use` paths.  
`wundler-core` → `wundler_core` in source → `$NEW_UNDERSCORE` after rename.

Bulk fix:
```bash
find crates/ -name "*.rs" -exec \
  sed -i '' "s/wundler_core/${NEW//-/_}_core/g; s/wundler_graph/${NEW//-/_}_graph/g" {} \;
# Repeat for each crate name variant
```

### CLI binary name

Three locations in the plan declare the binary name:

```rust
// crates/wundler-cli/src/main.rs
#[command(name = "wundler", ...)]
```

```toml
# crates/wundler-cli/Cargo.toml
[[bin]]
name = "wundler"
```

Both → `name = "$NEW"`.

### Config file name (`wundler.toml`)

Referenced 18+ times in Plan 3 (level-0-pipeline). Change to `$NEW.toml` everywhere:
```bash
find crates/ -name "*.rs" -exec \
  sed -i '' 's/wundler\.toml/$NEW.toml/g' {} \;
```

Also update the default arg in the CLI:
```rust
#[arg(long, default_value = "wundler.toml")]
// → default_value = "$NEW.toml"
```

---

## 6. Documentation Content

### `docs/wundler-architect-brief.md` (after rename)

All occurrences of `Wundler` and `wundler` in prose. ~30 occurrences.

```bash
sed -i '' "s/Wundler/$NEW_UPPER/g; s/wundler/$NEW/g" docs/$NEW-architect-brief.md
```

### `docs/wundler-executive-summary.md` (after rename)

~10 occurrences. Same command.

### `docs/superpowers/specs/2026-05-13-$NEW-design.md`

~15 occurrences in the spec prose (the bulk are now replaced by the plan bulk-replace above).

---

## 7. External / Ecosystem (when ready to publish)

| Surface | Current | Action needed |
|---|---|---|
| crates.io | Not yet published | Register `$NEW-core`, `$NEW-graph`, etc. |
| npm | Not yet published | Register `@microsoft/$NEW` or `$NEW` |
| GitHub Actions CI | Not yet created | Update `cargo publish` workflow crate names |
| GitHub repo description/topics | — | Update after repo rename |
| docs site (if created) | — | Domain / GitHub Pages path update |

---

## 8. One-liner Smoke Check After Rename

After all the above:

```bash
# No stale wundler references should remain in source (allow this file and git history)
grep -r "wundler" crates/ --include="*.rs" --include="*.toml"
grep -r "wundler" docs/ --include="*.md" | grep -v "RENAME.md"
```

Both should return empty.

---

## 9. Stale References This File Won't Catch

- **Git history**: commits will still say "wundler". Use `git filter-repo` if history rewrite is needed (usually not worth it).
- **This file itself**: `RENAME.md` uses "wundler" throughout. Update or delete after the rename is complete.
- **Linker-map image** (`docs/assets/linker-map.png`): the gpt-image-2 generated image has "WUNDLER" text baked in. Regenerate or accept as legacy.
- **Any external issues/PRs/discussions** already created: manually update.
