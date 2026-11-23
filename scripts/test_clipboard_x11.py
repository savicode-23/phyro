"""Exercise the native image clipboard on the private display created by xvfb-run."""

import json
import os
import shutil
import subprocess
import tempfile
import time
from pathlib import Path

from PIL import Image


ROOT = Path(__file__).resolve().parents[1]
BIN = ROOT / "target" / "debug"


def objects(project):
    output = subprocess.check_output(
        [BIN / "phyro-cli", "inspect", project, "--clip", "1", "--frame", "0"],
        timeout=15,
    )
    drawing = json.loads(output)["result"]["clips"][0]["drawing"]
    return {obj["object_id"]: obj for obj in drawing["objects"]}


def wait_for_title(editor, pattern, stage):
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if editor.poll() is not None:
            raise RuntimeError(f"Editor exited with status {editor.returncode} while {stage}")
        result = subprocess.run(
            ["xdotool", "search", "--onlyvisible", "--name", pattern],
            capture_output=True, text=True, timeout=3,
        )
        if result.returncode == 0:
            return result.stdout.splitlines()[0]
        time.sleep(0.15)
    raise RuntimeError(f"Timed out {stage} (expected window title matching {pattern!r})")


def check_paste(directory):
    project = directory / "clipboard.vanim"
    shutil.copyfile(ROOT / "examples" / "moving-rectangle.vanim", project)
    other = directory / "scene.vanim"
    shutil.copyfile(ROOT / "examples" / "lantern-walk.vanim", other)
    other_bytes = other.read_bytes()
    original = objects(project)
    expected = Image.new("RGBA", (8, 6), (240, 32, 64, 255))
    expected.paste((18, 210, 121, 255), (0, 0, 3, 4))
    png = directory / "clipboard.png"
    expected.save(png)

    env = os.environ.copy()
    env.pop("WAYLAND_DISPLAY", None)
    for key in ("XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME"):
        env[key] = str(directory / key)

    editor = subprocess.Popen([BIN / "phyro", project, other], env=env)
    try:
        saved_title = r"^clipboard\.vanim [^*]*Phyro$"
        other_title = r"^scene\.vanim [^*]*Phyro$"
        window = wait_for_title(editor, other_title, "opening the second project tab")
        subprocess.run(["xdotool", "windowfocus", "--sync", window], check=True, timeout=5)
        subprocess.run(["xdotool", "key", "--clearmodifiers", "ctrl+Tab"], check=True, timeout=5)
        wait_for_title(editor, saved_title, "switching to the first project tab")

        # Only image/png is offered. A text clipboard owner would also generate
        # an egui Paste event and could mask failures on image-only clipboards.
        xclip = ["xclip", "-selection", "clipboard", "-target", "image/png"]
        with png.open("rb") as fixture:
            subprocess.run([*xclip, "-in"], stdin=fixture, check=True, timeout=5)
        if subprocess.check_output([*xclip, "-out"], timeout=5) != png.read_bytes():
            raise RuntimeError("Could not seed the virtual X11 clipboard with the fixture PNG")
        subprocess.run(
            ["xdotool", "keydown", "Control_L", "key", "v", "keyup", "Control_L"],
            check=True, timeout=5,
        )
        wait_for_title(
            editor, r"^clipboard\.vanim \* .*Phyro$", "waiting for Ctrl+V to modify the drawing"
        )
        subprocess.run(["xdotool", "key", "--clearmodifiers", "ctrl+s"], check=True, timeout=5)
        wait_for_title(editor, saved_title, "waiting for Ctrl+S to save the drawing")
        subprocess.run(["xdotool", "key", "--clearmodifiers", "ctrl+w"], check=True, timeout=5)
        wait_for_title(editor, other_title, "closing the saved tab and returning to the scene")
    finally:
        editor.kill()
        editor.wait(timeout=5)

    if other.read_bytes() != other_bytes:
        raise RuntimeError("Saving the first tab changed the second project file")
    saved = objects(project)
    added = [obj for key, obj in saved.items() if key not in original]
    if not original.keys() <= saved.keys() or len(added) != 1:
        raise RuntimeError(
            f"Expected one new object after paste; before {list(original)}, after {list(saved)}"
        )
    image = added[0]
    if image.get("image_size") != list(expected.size) or not image.get("include_in_export"):
        raise RuntimeError(f"Incorrect saved image dimensions or export setting: {image}")

    frames = directory / "frames"
    subprocess.run(
        [BIN / "phyro-cli", "export", project, "--clip", "1", "--format", "frames",
         "--scale", "1", "--out", frames],
        check=True, timeout=20,
    )
    x, y = map(int, image["position"])
    with Image.open(frames / "frame_0000.png") as frame:
        crop = frame.convert("RGBA").crop((x, y, x + expected.width, y + expected.height))
    if crop.tobytes() != expected.tobytes():
        raise RuntimeError(f"Exported pixels at ({x}, {y}) differ from the pasted PNG")
    print(f"Native Ctrl+V added one {expected.width}×{expected.height} image. Its exported pixels match.")


if __name__ == "__main__":
    if not Path(os.environ.get("XAUTHORITY", "")).parent.name.startswith("xvfb-run."):
        raise SystemExit("Run this check on its own display with xvfb-run -a.")
    with tempfile.TemporaryDirectory() as temporary:
        check_paste(Path(temporary))
