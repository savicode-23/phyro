# Phyro from the command line

Phyro downloads include a headless CLI for inspecting, editing and exporting `.vanim` projects. Extract the archive and open a terminal in the folder containing the editor and CLI. From a source checkout, run `cargo build --locked -p phyro -p phyro-cli` and use the executables under `target/debug`. The sample projects are under `examples/` in the checkout.

| Task | Windows PowerShell | Linux shell |
| --- | --- | --- |
| Show help | `.\phyro-cli.exe --help` | `./phyro-cli --help` |
| List commands, edit operations and arguments as JSON | `.\phyro-cli.exe describe` | `./phyro-cli describe` |

The commands below use `./phyro-cli`, which works in both shells from the extracted folder. From the repository root, use `./target/debug/phyro-cli` and prefix each sample project and script path with `examples/`. Use a fresh output filename or directory for each run.

## Connect an assistant to the open editor

Add a local stdio server to your MCP client. Set the command to the absolute path of the installed CLI and pass the single argument `mcp`. For clients that use an `mcpServers` configuration, a Linux example is shown below. On Windows, use a path such as `C:/Tools/Phyro/phyro-cli.exe`.

```json
{
  "mcpServers": {
    "phyro": {
      "command": "/opt/phyro/phyro-cli",
      "args": ["mcp"]
    }
  }
}
```

Use the editor and CLI from the same build. Run the MCP server and editor on the same machine under the same OS account. Ask the assistant to call `connect`, then enter its two-minute pairing code in Phyro while the MCP server is still running. If the code expires or the client restarts, call `connect` again and use the code it returns. Each client process pairs with one editor tab. [Artist controls](editor-guide.md#review-assistant-edits)

Use `get_context` for the document handle, live revision, clip ID, selection and playhead. `tick` is zero-based and `frame_number` is the number shown in the editor. Call `inspect` with `clip_id`, `tick` and a query such as `{"kind":"selection"}`, `{"kind":"id","object_id":20}`, `{"kind":"name","name":"Sleeve fabric"}` or `{"kind":"all"}`. Multiple exact-name matches are marked ambiguous. Read subsequent pages using `next_offset`. IDs are scoped to the returned clip and exposure start.

Send `propose_edits` the context's `document` and `expected_revision` (from `revision`), a unique `request_id` and an ordered `operations` array. The MCP tool schema and `phyro-cli describe` list the supported operations. A repeated request ID with the same batch returns its existing proposal. Poll `get_proposal` with the returned `proposal_id`. An applied result includes the committed revision and per-operation IDs. Call `render_frame` with the clip ID and tick to inspect the accepted pixels. It returns PNG image content and the sampled revision.

To draw progressively, call `begin_draft` with the document, expected revision and request ID. Send each complete visual change with `append_draft_step(proposal_id, step, label, operations)`, numbering steps from 1. The acknowledgement includes operation results scoped to their clip and exposure. Later steps can use these IDs. Retry an identical step to recover its acknowledgement, or correct a failed step at the same number. A draft accepts up to 128 operations and 1 MiB of step payload across all calls.

The first operation in a step chooses the preview clip and tick. Operations without a frame use their exposure start. Group operations that the artist can review together at that frame.

Call `finish_draft(proposal_id, last_step)` to mark a nonempty drawing ready for review. Poll `get_proposal` for `latest_step` and the artist's decision. The artist may close the draft during drawing. An applied decision identifies the kept step, committed revision and operation results through that step. Each result includes its step number and index within that step. A new drawing starts at the current committed revision.

A busy result can be retried after the artist finishes editing. For a stale proposal, inspect the new revision and use a new request ID. Reviews expire after ten minutes. Each successful new step and the first successful finish renew the timeout. Sessions accept up to 256 proposals and retain their decisions until sharing ends.

## Find a drawing or object

```sh
./phyro-cli inspect lantern-walk.vanim --clip 1 --frame 4
./phyro-cli lookup lantern-walk.vanim --clip 1 --frame 0 --name "Sleeve fabric"
```

Frames start at zero. The first command shows the layers, objects, bones and keys at the fifth frame. The second finds a case-sensitive name and returns its `object_id`, `clip_id` and `exposure_start`. Keep these values and the top-level `revision` when preparing an edit.

## Create or edit a project

```sh
./phyro-cli new --out scene.vanim --width 256 --height 256 --frames 24 --fps 12
```

To see a complete edit, run the bundled Python 3 example. It finds the sleeve, changes its fill and sets five position keys. Open `walk-output/edited.vanim` in the editor, or view `walk-output/pose.png` and the sprite sheet in `walk-output/sheet/`.

| Shell | Command |
| --- | --- |
| Windows PowerShell | `python agent-workflow.py lantern-walk.vanim walk-output --cli .\phyro-cli.exe` |
| Linux | `python3 agent-workflow.py lantern-walk.vanim walk-output --cli ./phyro-cli` |

For your own edits, write a JSON batch with `version` set to `1`, the input file’s `revision` and an ordered `operations` array. Run `./phyro-cli apply lantern-walk.vanim --batch edits.json --out edited.vanim`, or pass `--batch -` to read from standard input. The whole batch must succeed to produce the edited project. Read that file to get its revision before submitting another batch. Scripts can check the exit status and inspect the JSON `error` on failure. [Python example](../examples/agent-workflow.py)

## Render a preview or sprite sheet

```sh
./phyro-cli preview lantern-walk.vanim --clip 1 --frame 4 --out pose.png
./phyro-cli export lantern-walk.vanim --clip 1 --format sheet --columns 4 --out walk-sheet
```

With `--format sheet`, the output directory contains `sheet.png` and `sheet.json` with frame regions and timing. Use `--format frames` for individual PNGs, `sheet-and-frames` for a grid sheet and individual PNGs, or `packed` for a packed atlas.

Use `inspect` to find a set or clip ID. Both selectors support `--format png-sequence`, `separate-sheets` and `shared-sheet`. For a set, only clips with `include_in_set_export` enabled get their own animation entry. Direct clip export always includes the specified clip. Nested clips continue to render inside their parent. Every layout writes the image list, per-clip timing and pivots to `sheet.json` and `animation.phyro`.

```sh
./phyro-cli inspect lantern-walk.vanim
./phyro-cli export lantern-walk.vanim --set 1 --format png-sequence --out walk-frames
./phyro-cli export lantern-walk.vanim --set 1 --format separate-sheets --out walk-sheets
./phyro-cli export lantern-walk.vanim --clip 1 --format shared-sheet --out walk-atlas
```

In this example, the set is named **Animations**. The first two layouts produce filenames such as `animations_lantern_walk_1.png` and `animations_lantern_walk.png`. The shared sheet is named `animations.png`. A set defaults to a shared sheet. Use `--layout grid --columns 4` or `--layout packed` with `shared-sheet` or `separate-sheets` to override the saved layout. The named sheet formats produce one PNG per sheet. If a sheet exceeds its size limit, the command reports the problem before publishing any output. For existing single-clip scripts, `--format frames` keeps the original `frame_0000.png` naming. With `sheet-and-frames` these images are in the `frames` subfolder.

## Export the current frame

Use `export-artboard` for the full canvas or `export-selection` for selected object IDs. Both support `--format png`, `svg` and `both`. The selection defaults to one combined crop. Use `--mode individual` for one named asset per top-level object or group. Give `--out` a filename for a single image or a new directory for both formats and for batches. `--scale` changes the PNG resolution, including fractional sizes. For example, `--scale 0.256` exports a 1,000 px artboard at 256 px wide. SVG uses document units.

```sh
./phyro-cli export-artboard lantern-walk.vanim --clip 1 --frame 4 --format both --out scene
./phyro-cli export-selection lantern-walk.vanim --clip 1 --frame 4 --object-ids 20 --out sleeve.png
./phyro-cli export-selection lantern-walk.vanim --clip 1 --frame 4 --object-ids "19,20" --mode individual --format both --out parts
```

## Export a moving selection

```sh
./phyro-cli export-selection-animation lantern-sleeve-reach.vanim --clip 1 --frame 0 --object-ids 20 --from 1 --to 6 --format sheet-and-frames --columns 3 --scale 2 --out sleeve-reach
```

`--frame` identifies the artwork and `--from` and `--to` select an inclusive range. The frames share one crop and pivot so the exported part keeps its motion. To import the sheet with the Godot add-on, copy the output's `sheet.json` to `animation.phyro` beside `sheet.png`.
