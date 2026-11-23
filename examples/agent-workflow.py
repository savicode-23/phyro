"""Recolor and animate the sleeve in Lantern Walk using the public CLI."""

import argparse
import json
import subprocess
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("project", type=Path)
    parser.add_argument("destination", type=Path)
    parser.add_argument("--cli", default="phyro-cli")
    args = parser.parse_args()

    def run(*command, batch=None):
        response = subprocess.run(
            [args.cli, *map(str, command)],
            input=json.dumps(batch).encode("utf-8") if batch is not None else None,
            stdout=subprocess.PIPE,
            check=False,
        )
        message = json.loads(response.stdout)
        if response.returncode or not message["ok"]:
            raise SystemExit(json.dumps(message["error"], indent=2))
        return message

    contract = run("describe")["result"]
    if contract["protocol_version"] != 1:
        raise SystemExit("This example uses Phyro CLI protocol 1.")

    match = run("lookup", args.project, "--name", "Sleeve fabric", "--frame", 0)
    drawing = match["result"]
    if drawing["exposure_duration"] < 5:
        raise SystemExit("The sleeve needs an exposure of at least five frames.")
    target = {key: drawing[key] for key in ("clip_id", "exposure_start", "object_id")}
    x, y = drawing["position"]
    edits = [dict(target, op="set_fill", fill=[220, 88, 54, 255])]
    for tick in range(5):
        edits.append(dict(target, op="set_position_key", frame=tick, position=[x + 2 * tick, y]))
    batch = {"version": 1, "revision": match["revision"], "operations": edits}

    args.destination.mkdir(parents=True, exist_ok=True)
    edited = run("apply", args.project, "--batch", "-", "--out",
                 args.destination / "edited.vanim", batch=batch)["result"]["output"]
    clip = drawing["clip_id"]
    preview = run("preview", edited, "--clip", clip, "--frame", 4,
                  "--out", args.destination / "pose.png")["result"]
    sheet = run("export", edited, "--clip", clip, "--out", args.destination / "sheet",
                "--format", "sheet", "--scale", 1, "--columns", 4)["result"]
    print(json.dumps({"project": edited, "preview": preview["output"], "sheet": sheet}, indent=2))


if __name__ == "__main__":
    main()
