"""Export native selections and play the imported sprite sheets in Godot."""

import argparse
import json
import shutil
import subprocess
import tempfile
from pathlib import Path


INTEGRATION = Path(__file__).resolve().parent
ROOT = INTEGRATION.parents[1]


def command(cli, *args):
    completed = subprocess.run([cli, *map(str, args)], stdout=subprocess.PIPE, check=False)
    result = json.loads(completed.stdout)
    if completed.returncode or not result["ok"]:
        raise RuntimeError(result["error"])
    return result["result"]


def export_case(cli, directory, filename, first, last, object_ids):
    project = ROOT / "examples" / filename
    clip = command(cli, "inspect", project, "--clip", 1)["clips"][0]
    scale = 2
    reference = directory / "reference"
    reference.mkdir(parents=True)
    frames = []
    for index, object_id in enumerate(object_ids):
        tick = first + index
        if object_id is None:
            frames.append(None)
            continue
        # The expected IDs are authored in the fixture. The single-frame exporter
        # samples each one independently of the animated name matching and layout.
        path = reference / f"frame_{index:04d}.png"
        asset, = command(cli, "export-selection", project, "--clip", 1, "--frame", tick,
                         "--object-ids", object_id, "--scale", scale, "--out", path)["assets"]
        frames.append({"origin": asset["origin"], "size": asset["size"]})
    events = [
        {"name": name, "frame": tick - first, "source_frame": tick}
        for tick in range(first, last + 1)
        for name in clip["events"].get(str(tick), [])
    ]
    assert len(frames) == last - first + 1
    expected = {"name": clip["name"], "fps": clip["fps"], "looping": clip["looping"],
                "pivot": [n * scale for n in clip["pivot"]], "frames": frames, "events": events,
                "first": first, "last": last}
    (directory / "expected.json").write_text(json.dumps(expected), encoding="utf-8")
    bundle = directory / "export"
    command(cli, "export-selection-animation", project, "--clip", 1, "--frame", 0,
            "--object-ids", 20, "--from", first, "--to", last, "--format", "sheet-and-frames",
            "--columns", 3, "--scale", scale, "--out", bundle)
    # Exercise the legacy manifest extension on the weighted walk as well.
    manifest = "animation.loopline" if filename == "lantern-walk.vanim" else "animation.phyro"
    shutil.copyfile(bundle / "sheet.json", bundle / manifest)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", default="phyro-cli")
    parser.add_argument("--godot", default="godot")
    args = parser.parse_args()
    target = ROOT / "target"
    target.mkdir(exist_ok=True)
    directory = Path(tempfile.mkdtemp(prefix="selection-godot-", dir=target))
    print(f"Godot acceptance project: {directory}", flush=True)
    # Keep generated engine imports and temporary output away from the example project.
    shutil.copytree(INTEGRATION / "addons", directory / "addons",
                    ignore=shutil.ignore_patterns("*.uid", "*.import"))
    shutil.copyfile(INTEGRATION / "verify_selection_animation.gd", directory / "verify_selection_animation.gd")
    settings = (INTEGRATION / "project.godot").read_text(encoding="utf-8")
    (directory / "project.godot").write_text(
        settings.replace("res://keeper.tscn", "res://reach.tscn")
                .replace("Phyro - Lantern swing", "Phyro - Sleeve Reach"), encoding="utf-8")
    cases = [
        ("sleeve-reach", "lantern-sleeve-reach.vanim", 1, 6, [20, 20, None, None, 35, 35]),
        ("empty-hold", "lantern-sleeve-reach.vanim", 3, 4, [None, None]),
        ("lantern-walk", "lantern-walk.vanim", 0, 7, [20] * 8),
    ]
    for name, filename, first, last, ids in cases:
        export_case(args.cli, directory / name, filename, first, last, ids)
    (directory / "reach.tscn").write_text(
        '''[gd_scene load_steps=3 format=3]

[ext_resource type="SpriteFrames" path="res://sleeve-reach/export/animation.phyro" id="1"]
[ext_resource type="Script" path="res://addons/phyro/phyro_sprite.gd" id="2"]

[node name="SleeveReach" type="AnimatedSprite2D"]
position = Vector2(180, 144)
sprite_frames = ExtResource("1")
animation = &"Sleeve reach"
autoplay = "Sleeve reach"
script = ExtResource("2")
''', encoding="utf-8")
    # Godot can exit zero after an early script parse error. Require its completion message.
    launch = [args.godot, "--headless", "--path", str(directory)]
    subprocess.run([*launch, "--log-file", str(directory / "import.log"), "--import"],
                   check=True, timeout=120)
    for name, *_ in cases:
        log = directory / f"{name}.log"
        subprocess.run([*launch, "--log-file", str(log), "--fixed-fps", "120",
                        "--script", "res://verify_selection_animation.gd", "--", name],
                       check=True, timeout=30)
        if f"PASS {name} " not in log.read_text(encoding="utf-8"):
            raise RuntimeError(f"Godot did not complete the {name} playback check. See {log}")


if __name__ == "__main__":
    main()
