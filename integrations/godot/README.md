# Lantern swing in Godot

Open `project.godot` in Godot 4 and press **F5**. The Lantern Keeper swings the lamp once per second while both boots stay planted on the horizontal guide. To edit the drawings in Phyro, open `examples/lantern-watch.vanim` from the repository root. The **Lantern swing** clip contains eight separate vector drawings. **Signal** holds one drawing and animates the flame with position, scale, rotation and opacity keys.

`keeper.gd` reads `art/lantern-swing/sheet.json` and makes an `AtlasTexture` for every rectangle in the accompanying PNG. It sets the `SpriteFrames` speed, loop flag and frame durations from the metadata. With centering disabled, each frame's sprite offset is the negative of its pivot. For this 128 × 128 animation the point (56, 114) sits between the boots in every pose.

Run the headless check from the repository root with Godot 4.6.2 or a compatible 4.x executable on your path.

```sh
godot --headless --path integrations/godot --import
godot --headless --path integrations/godot --fixed-fps 120 --script res://verify.gd
```

The check compares each of the eight 128 × 128 atlas regions with its individual PNG, checks that all drawings are distinct, then watches the running scene for a full loop. It checks frame order, timing, sprite origin and boot pixels. The demo uses the sheet for playback. The individual PNGs are reference frames for the check.

To regenerate the native project and both export forms from `examples/lantern-keeper.svg`, supply a new output directory.

```sh
cargo run --locked -p project-io --example generate_lantern_watch -- keeper-output
```

The generator writes `keeper-output/lantern-watch.vanim`, reopens it, and exports the **Lantern swing** clip to `keeper-output/lantern-swing`. The demo's `art/lantern-swing` directory uses that bundle directly.

For the packed atlas importer, animated character scene and event test, see [Packed atlases in Godot 4](PACKED.md).

## Verify the rigged walk

The Lantern Walk generator accepts a new native project path and a new sprite directory. From the repository root, run the following commands in PowerShell. The generator reopens the saved project before exporting the grid and the eight reference frames.

```powershell
cargo run --locked -p project-io --example generate_lantern_walk -- target/lantern-walk.vanim integrations/godot/art/lantern-walk
Copy-Item integrations/godot/art/lantern-walk/sheet.json integrations/godot/art/lantern-walk/animation.phyro
godot --headless --path integrations/godot --import
godot --headless --fixed-fps 120 --path integrations/godot --script res://verify_walk.gd
```

The check plays the imported atlas through a full loop and compares its regions with the reference frames. It also verifies the pivot and the near and far foot-contact events.

## Verify animated selection exports

The [Sleeve Reach fixture](../../examples/lantern-sleeve-reach.vanim) animates a sleeve across the left edge of the artboard. Frames 3 and 4 are blank. At frame 5 the sleeve returns on a new drawing with a new object ID and layer ID. Its original ID now belongs to a button, and an accessory on another layer shares the sleeve’s name.

Build the CLI, then run the Python 3 check with Godot 4 on your path. In PowerShell, use:

```powershell
cargo build --release --locked -p phyro-cli
python integrations/godot/check_selection_animation.py --cli target/release/phyro-cli.exe --godot godot
```

On Linux, use `python3` and `target/release/phyro-cli`. The check creates a fresh Godot project under `target/` and imports the exported sheets. Open that project and press **F5** to play the exported sleeve. The check plays frames 1–6 of Sleeve Reach, its empty two-frame hold, and the full weighted sleeve animation from Lantern Walk. Each sheet cell and individual frame PNG is compared with an independent static export positioned relative to the original artboard. Playback checks the pivot, frame order, FPS and event ticks through a complete loop.
