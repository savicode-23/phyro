# Working on Phyro

Phyro is a Rust 2021 workspace for a desktop vector animation editor and its headless CLI. The desktop package is named `phyro` and lives in `crates/editor-app`. It is the default Cargo member. Use `--workspace` for checks that cover the whole application.

## GitHub pushes

- Get explicit permission from the user before pushing any commit, branch, or tag to GitHub. This applies to all agents and to equivalent operations through the GitHub API, integrations, or automation. A request to edit files or create a local commit does not authorize a push.

## Start a session

- Check `git status --short --branch` before editing. Use the roadmap to pick up current work and the architecture map to find the implementation and tests. [Roadmap](ROADMAP.md) · [Architecture](ARCHITECTURE.md)
- Use [SPEC.md](SPEC.md) for product intent. Its original milestone sequence is historical. For current user workflows, consult the [editor guide](docs/editor-guide.md) and [CLI guide](docs/command-line.md).
- Use stable Rust and the committed `Cargo.lock`. The README lists native build prerequisites, including the X11 runtime library needed on Ubuntu. [Build from source](README.md#build-from-source)

## Conventions

- Put reusable geometry, animation evaluation and edit rules in `document-core`. Keep egui gestures, focus, selection and undo in the editor. Route project files, artwork interchange and exports through `project-io` so the CLI and editor agree.
- Preserve authored base values, stable IDs and layer paint order. Sample the playhead through the model. Use the editor’s existing edit transaction so a completed gesture can be undone once. Test the held frames on either side of a drawing edit.
- For persisted fields, check defaults for older files, archive version selection, validation, and a save/reopen round trip. Exercise edits through both front ends when they expose the same operation. The CLI’s `describe` output is its machine-readable contract.
- Keep input and playback responsive. Reuse shared drawings, limit timeline work to visible rows and ticks, and check a background result against the project or view it was requested for. Measure the affected gesture on a representative scene.
- Place regressions beside the responsible module or in its crate’s `tests/`. Use the existing headless editor interaction tests for gestures and the project I/O fixtures for saved files and render comparisons. Keep generated builds under `target/`.
- Record implementation decisions in the architecture map. Keep the roadmap’s shipped summary short and put operating instructions in the user guides. For changes to bundled guide images, update the [package list](scripts/package_release.py).

## Run and validate

Run the editor or inspect the CLI contract from the repository root.

```sh
cargo run --locked -p phyro -- examples/lantern-walk.vanim
cargo run --locked -p phyro-cli -- describe
```

Desktop CI runs these workspace checks on stable Rust. Ubuntu also exercises native open, clipboard paste, save and export under X11. The workflow lists the exact packages and smoke command. [Desktop checks](.github/workflows/ci.yml)

```sh
cargo +stable fmt --all --check
cargo +stable clippy --locked --workspace --all-targets
cargo +stable test --locked --workspace
cargo +stable build --locked --workspace
```

For performance work, pick a named release probe. For example, the following measures scrolling and stepping a 1,000-frame timeline. Other probes cover snapping, first drag, large-selection undo, onion skins and library placements. They report CPU-side timings. Check input-to-display latency in a native window. [Responsiveness tests](crates/editor-app/src/editor/tests/responsiveness.rs)

```sh
cargo test --locked --release -p phyro named_layer_timeline_scroll_and_step -- --ignored --nocapture --test-threads=1
```

For packaging or release changes, follow the [release instructions](RELEASING.md).
