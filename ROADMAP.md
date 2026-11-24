# Phyro roadmap

## Next priority — editor responsiveness

1. Measure the pause between pressing a move, scale or rotation handle and seeing the first changed pose. Start with duplicated library clips, a large selection from an imported SVG and a snapping-enabled scene with thousands of off-artboard objects. Record press, first movement, release and undo separately. Aim for 60 fps during ordinary editing. [Transform probes](crates/editor-app/src/editor/tests/responsiveness/transforms.rs) · [Library placement probes](crates/editor-app/src/editor/tests/responsiveness/library_placement.rs)
2. Profile onion-skin stepping in 1,000-frame clips containing nested playback or animated bones. The editor already reuses known identical poses. Check both timing and ghost poses when artwork changes during a hold. [Held-pose detection](crates/editor-app/src/canvas/onion/held_ranges.rs) · [Long-animation probes](crates/editor-app/src/editor/tests/responsiveness/long_animation.rs)

## Upcoming work

- **Boolean curve fidelity.** Preserve more original Bézier spans at touching or ambiguous overlaps. Compare the number of editable nodes, silhouette and holes after repeated Union and Difference. The current Boolean engine falls back to polygon segments where it cannot recover a curve. [Boolean implementation](crates/document-core/src/boolean.rs)
- **Input and accessibility.** Check Pencil pressure and smoothing on a physical tablet, including corners and strokes reopened after a node edit. Audit keyboard focus, selected-state announcements and screen-reader navigation through Layers and the timeline. [Pencil](crates/editor-app/src/canvas/pencil.rs) · [Layer tree](crates/editor-app/src/inspector/layer_tree.rs) · [Timeline ruler](crates/editor-app/src/timeline/ruler.rs)
- **Distribution.** Evaluate a signed Windows installer with Start menu integration and a `.vanim` file association for public beta. Current packaging produces portable Windows and Linux archives. [Release workflow](.github/workflows/release.yml)
- **Longer-term character animation.** Explore joint limits, general mesh deformation and corrective poses beyond the current two-bone rigging tools. Later candidates include blending or reusing actions across clips and baking secondary motion into exported frames. [Rig implementation](crates/document-core/src/rig.rs) · [Product scope](SPEC.md)

## Implemented in the current tree

| Area | Working functionality |
| --- | --- |
| Drawing | Bézier and pressure Pencil tools, editable text, path and node editing, Booleans, gradients, blend modes, clipping, layers, groups, symbols and embedded raster references. |
| Animation | Animation sets, nested clips, independently timed drawing layers, property keys and easing, onion skins, guided shape morphs, two-bone IK, foot pins and weighted path deformation. The timeline supports frame box selection, disjoint ranges and bulk deletion with undo. |
| Workspace | Document tabs, rectangular and axonometric grids, folder asset library, configurable shortcuts and panel layout, recoverable drafts, recent issue history and copyable crash reports. |
| Assistant sharing | Opt-in MCP pairing for one tab, scoped inspection, frame previews and artist-reviewed batches. Live drawing previews can follow, pin and keep a completed step with one undo. |
| Delivery | PNG/SVG artboard, selection and batch export. Clips and sets export as PNG sequences, separate sheets or a shared sheet. The headless CLI supports project inspection, transactional edits and export. |

The [editor guide](docs/editor-guide.md) and [CLI guide](docs/command-line.md) describe how to use these features.

## Current limits

- The project has one shared artboard size, up to 4,096 pixels per side. Each clip can have its own timing, up to 4,096 frames at 240 fps. Project I/O limits animation JSON to 64 MiB, including embedded resources. These bounds matter when planning a large asset library in one file. [Model limits](crates/document-core/src/lib.rs) · [Archive limits](crates/project-io/src/project.rs)
- Guided morphs accept one outline per pose with matching open/closed topology. The two outlines can have different node counts. SVG import supports a subset of SVG, with nested stencils and offset radial gradient focal points among the unsupported cases. [Morph pairing](crates/document-core/src/morph.rs) · [SVG paint definitions](crates/project-io/src/svg/defs_read.rs)

For implementation trade-offs behind future changes, see [architecture and technical debt](ARCHITECTURE.md#technical-debt-to-keep-in-view).
