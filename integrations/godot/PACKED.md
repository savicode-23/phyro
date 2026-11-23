# Packed atlases in Godot 4

When upgrading an existing Godot project, disable the old **Loopline atlas** plugin before enabling **Phyro atlas**. Phyro imports existing `.loopline` files at their current paths. Existing player scripts can continue to read their pivots and events.

Copy `addons/phyro` into your Godot project and enable **Phyro atlas** in **Project Settings → Plugins**. In Phyro, use **Export clip…** or **Export animation set…** and choose a PNG sequence, one sheet per clip, or a shared sheet. Copy `animation.phyro` and all its PNGs into the same Godot project folder. All three layouts include the timing and texture regions for each animation. Godot imports the manifest as a `SpriteFrames` resource. Assign it to an `AnimatedSprite2D` node’s **Sprite Frames** property and select the clip by name. The neutral `sheet.json` contains the same metadata.

For the exported pivot and events, attach `addons/phyro/phyro_sprite.gd` to your node and call `play_clip(&"Lantern patrol")`. Connect its `animation_event(event_name, detail)` signal to react to a marker. `detail.frame` is the zero-based source tick. Events on the same tick arrive in their authored order, including markers on the first tick when playback starts or loops. The imported textures retain their original frame dimensions, so the node’s position stays at the exported pivot as its packed rectangle changes.

Open `packed.tscn` and press **F6** to run the included scene. The native `examples/lantern-patrol.vanim` uses eight cel poses across twelve ticks. The flame follows a curved guide with custom Bézier easing while its tip changes shape. Independent eyelid tracks continue to animate during held poses. Its saved 256 px preset produces three PNG pages and stores eight frames rotated. The addon restores their orientation once at import. At 12 fps the loop lasts one second.

The scene and check were verified in Godot 4.6.2. From the repository root, run the editor import first, then run the scene check with deterministic playback timing.

```sh
godot --headless --path integrations/godot --import
godot --headless --path integrations/godot --fixed-fps 120 --script res://verify_packed.gd
```

The check reconstructs each untrimmed frame from the imported atlas region and margin, compares it against the individual PNG exported from the native project, checks the actual texture pixels after Godot’s PNG import, and watches the playing node for one loop. It checks planted feet, the world-space pivot, frame durations and event delivery.

To regenerate the native project, atlas, manifest and reference PNGs in a new directory, run the example generator.

```sh
cargo run --locked -p project-io --example generate_lantern_patrol -- patrol-output
```

The addon accepts grid metadata version 1, single-clip packed metadata version 2, and animation set metadata version 3. A set imports as one `SpriteFrames` resource with each clip available by its local name. Each keeps its own frame rate, loop setting, pivots and events while sharing atlas pages. Call `play_clip(&"Idle")` or `play_clip(&"Walk")` to switch actions.

For a grid export that has `sheet.json` and `sheet.png`, such as an animated selection sheet, copy `sheet.json` to `animation.phyro` beside the PNG. The editor's clip and set export layouts already write the Godot manifest. The importer also reads older multipage atlases. It restores transparent margins and splits a held exposure when an event falls inside it, so that Godot emits the marker at its authored tick. The `reference` PNG sequence is only required for the headless example check.
