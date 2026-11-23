"""Build a portable Phyro release archive from native Cargo release binaries."""

import argparse
import filecmp
import gzip
import json
import mmap
import os
from pathlib import Path
import re
import subprocess
import sys
import tarfile
import tempfile
import zipfile


ROOT = Path(__file__).resolve().parents[1]
EXAMPLES = (
    "agent-workflow.py",
    "bouncing-ball.vanim",
    "draw-sparks-over-motion.vanim",
    "guided-flame.vanim",
    "lantern-elbow.vanim",
    "lantern-patrol.vanim",
    "lantern-sleeve-reach.vanim",
    "lantern-walk-timeline-study.vanim",
    "lantern-walk.vanim",
)


def package_files(binaries, notices, windows):
    suffix = ".exe" if windows else ""
    files = {name + suffix: binaries / (name + suffix) for name in ("phyro", "phyro-cli")}
    files.update({name: ROOT / "examples" / name for name in EXAMPLES})
    files["START-HERE.txt"] = ROOT / "release" / "START-HERE.txt"
    if (ROOT / "LICENSE").is_file():
        files["LICENSE"] = ROOT / "LICENSE"
    notice_files = sorted(path for path in notices.rglob("*") if path.is_file())
    if not notice_files:
        raise ValueError(f"No third-party notices found in {notices}")
    for path in notice_files:
        files["third-party-notices/" + path.relative_to(notices).as_posix()] = path
    return files


def archive_mode(name):
    return 0o755 if name in ("phyro", "phyro-cli", "phyro.exe", "phyro-cli.exe") else 0o644


def package_guides(staging):
    images = (
        "welcome.png", "editor-workspace.png", "bouncing-ball-sheet.png", "morph-window.png",
        "folder-library.png", "library-placement.png", "timeline-frame-selection.png",
    )
    files = {"docs/" + name: ROOT / "docs" / name for name in images}
    for name in ("getting-started.md", "editor-guide.md", "command-line.md"):
        text = (ROOT / "docs" / name).read_text(encoding="utf-8")
        # The samples sit beside the executables in portable downloads.
        text = text.replace("](../examples/", "](../")
        text = text.replace("](../integrations/", "](https://github.com/savicode-23/phyro/blob/main/integrations/")
        guide = staging / name
        guide.write_text(text, encoding="utf-8", newline="\n")
        files["docs/" + name] = guide
    return files


def write_zip(archive, folder, files):
    with zipfile.ZipFile(archive, "x") as out:
        for name, source in sorted(files.items()):
            # Explicit ZIP timestamps and modes keep host metadata out of the release.
            info = zipfile.ZipInfo(f"{folder}/{name}", date_time=(1980, 1, 1, 0, 0, 0))
            info.create_system = 3
            info.external_attr = (0o100000 | archive_mode(name)) << 16
            out.writestr(info, source.read_bytes(), compress_type=zipfile.ZIP_DEFLATED, compresslevel=9)


def write_tar_gz(archive, folder, files):
    with archive.open("xb") as raw, gzip.GzipFile(filename="", fileobj=raw, mode="wb", mtime=0) as gz:
        with tarfile.open(fileobj=gz, mode="w|", format=tarfile.GNU_FORMAT) as out:
            for name, source in sorted(files.items()):
                info = tarfile.TarInfo(f"{folder}/{name}")
                info.size = source.stat().st_size
                info.mode = archive_mode(name)
                with source.open("rb") as data:
                    out.addfile(info, data)


def verify(archive, extracted, folder, files, windows, version, revision):
    if windows:
        with zipfile.ZipFile(archive) as bundle:
            bundle.extractall(extracted)
    else:
        with tarfile.open(archive) as bundle:
            bundle.extractall(extracted, filter="data")
    contents = extracted / folder
    paths = {path.relative_to(contents).as_posix() for path in contents.rglob("*") if path.is_file()}
    if paths != files.keys():
        raise ValueError("Extracted archive contents differ from the package manifest")
    for name, source in files.items():
        if not filecmp.cmp(source, contents / name, shallow=False):
            raise ValueError(f"Packaged file differs from its source: {name}")
    if revision:
        editor = contents / ("phyro.exe" if windows else "phyro")
        with editor.open("rb") as binary, mmap.mmap(binary.fileno(), 0, access=mmap.ACCESS_READ) as data:
            if data.find(revision.encode("ascii")) < 0:
                raise ValueError("Packaged editor is missing the expected build revision")
    native_platform = "win32" if windows else "linux"
    if sys.platform == native_platform:
        cli = contents / ("phyro-cli.exe" if windows else "phyro-cli")
        output = subprocess.check_output([str(cli), "describe"], cwd=contents)
        metadata = json.loads(output)["result"]
        if metadata["program"] != "phyro-cli":
            raise ValueError("Packaged CLI must identify itself as phyro-cli")
        if metadata["package_version"] != version:
            raise ValueError(f"Packaged CLI version must match release version {version}")
        output = subprocess.check_output([str(cli), "inspect", "lantern-walk.vanim"], cwd=contents)
        result = json.loads(output)
        if not result["ok"] or not result["result"]["clips"]:
            raise ValueError("Packaged CLI could not inspect the included Lantern Walk project")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True, help="Version without a leading v, e.g. 0.1.0-alpha.1")
    parser.add_argument("--platform", required=True, choices=("windows-x64", "linux-x64"))
    parser.add_argument("--bin-dir", type=Path, required=True, help="Cargo release binary directory")
    parser.add_argument("--notices-dir", type=Path, required=True, help="Generated third-party notices")
    parser.add_argument("--out-dir", type=Path, required=True)
    parser.add_argument("--revision", help="Expected 40-character Git commit embedded in the editor")
    args = parser.parse_args()
    identifiers = r"[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*"
    if not re.fullmatch(rf"[0-9]+\.[0-9]+\.[0-9]+(?:-{identifiers})?(?:\+{identifiers})?", args.version):
        parser.error("--version must be a numeric three-part version with optional prerelease and build identifiers")
    if args.revision and not re.fullmatch(r"[a-f0-9]{40}", args.revision):
        parser.error("--revision must be a full lowercase Git commit hash")
    windows = args.platform == "windows-x64"
    # GitHub release asset names use a portable filename for SemVer's '+'.
    filename_version = args.version.replace("+", "_")
    folder = f"Phyro-{filename_version}-{args.platform}"
    output = args.out_dir.resolve()
    output.mkdir(parents=True, exist_ok=True)
    final = output / (folder + (".zip" if windows else ".tar.gz"))
    if final.exists():
        parser.exit(1, f"Release archive already exists: {final}\n")
    files = package_files(args.bin_dir, args.notices_dir, windows)
    with tempfile.TemporaryDirectory(dir=output) as staging:
        files.update(package_guides(Path(staging)))
        archive = Path(staging) / final.name
        if windows:
            write_zip(archive, folder, files)
        else:
            write_tar_gz(archive, folder, files)
        verify(archive, Path(staging) / "check", folder, files, windows, args.version, args.revision)
        # Both operations refuse an existing destination, including in a race.
        if os.name == "nt":
            archive.rename(final)
        else:
            os.link(archive, final)
    print(final)


if __name__ == "__main__":
    main()
