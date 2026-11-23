# CoverageCraft: instructions for agents

CoverageCraft plans Wi-Fi coverage from CADCraft DXF plans and SolidCraft OBJ/STL models. It is a
beta that aims to join the Craft suite, so it follows the Craft app standards
(`standards/app-agents.md` in storytold/craft-contrib). Read that file first; the rules below are
how this repository applies it.

## This repository

- `crates/coveragecraft-engine`: everything that isn't drawing. Plans (`dxf.rs`, `mesh.rs`,
  `plan.rs`), materials (`materials.rs`), the signal model (`model.rs`), channels and link rate
  (`radio.rs`), the maps (`views.rs`), spot suggestions (`placement.rs`), measured points and
  calibration (`survey.rs`), devices (`devices.rs`), the commands (`engine.rs`), PNG export
  (`picture.rs`), the HTML report (`report.rs`), viewpoints (`viewpoint.rs`), floors (`floors.rs`; the
  cross-floor path is `model::Building`), SolidCraft designs
  through `solidcraft-cli` (`sc.rs`) and DWG through acadrust (`dwg.rs`). No egui here.
- `apps/coveragecraft`: the egui window. It only calls `Engine::execute` and reads the engine's
  state; a feature that exists only in the window is a bug. `header.rs` is the Craft header
  (craft-window's title bar, craft-menu's menus, toolbar, status bar, Home): every menu item is a
  command id run by `App::menu_command`; `captions.rs` draws the window buttons and resize edges
  where the app's bar is the title bar (Windows, Linux).
- `apps/coveragecraft-cli`: the same commands without a window, plus scripts.
- Tests: unit tests beside the code (`*_tests.rs` or `mod tests`); integration tests in
  `crates/coveragecraft-engine/tests/it/main.rs`, one binary. Fixtures in `tests/fixtures/`, each
  with a row in `ATTRIBUTION.md`.

## Rules

- **No personal or private information, ever.** Nothing about the maintainer or anyone else goes
  into this repository, its history, issues, pull requests, screenshots or examples: no names,
  email addresses, home paths, addresses, and no one's own floor plans or models (not even their
  object names or measurements), and no real survey data (measured points come from someone's
  home or office). Test and demonstrate only on made-up plans and made-up measurements, or
  openly licensed ones (like the CADCraft example in `tests/fixtures/`). Commits use this repository's neutral
  identity (`git config user.name` / `user.email`), not a personal one. Check before each commit:
  `git grep -n -i -E '/Users/|/home/|@'` and look at every image you add.
- **Everything is a command.** A new ability is a command in `engine.rs` (id, one-line params doc
  in `COMMANDS`), then a control in the window that calls it. Programmatic calls never open
  dialogs.
- **Never crash.** No `unwrap`, `expect`, `panic!`, `todo!` or `unsafe` outside tests (the
  workspace lints deny them), and no indexing that can go out of range (use `get`). Files and parameters are hostile:
  return an `EngineError`. Add hostile cases to `hostile_commands_are_refused`.
- **Exact geometry decides.** Wall crossings come from the plan's outlines, not from the
  heatmap grid. The grid is only for display and the floor figures.
- **Clean room.** Reviewing another Wi-Fi tool's public feature list is fine; copying its code,
  text, UI layout, icons, map names or data (material tables, rate tables) is not, and its name
  stays out of code, labels, commits and docs. Numbers come from public standards (802.11) or
  our own measurements.
  Material losses are our own presets in `materials.rs`: typical values, not yet tied to a
  published source; when changing one, record where the new number comes from. Device data
  comes only from public certification records (FCC, ISED).
- **Craft libraries** (`craft-dxf`, `craft-mesh`) are git dependencies pinned at one `rev` in
  the workspace `Cargo.toml`. Bump them together.
- **Text.** Every user-visible string goes through `i18n::tr` with its English text as the key.
  Don't name trademarks in labels: say "signal", "coverage", "router", "access point".
- **Colours** come from `theme.rs` tokens.
- **Names.** New feature or product names get a check recorded in `docs/feature-names.md`;
  unclear means no.

## Before each commit

```bash
cargo fmt --all
```

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

```bash
cargo test --workspace
```

On a shared machine, add `-j 2` and run under `nice`.
