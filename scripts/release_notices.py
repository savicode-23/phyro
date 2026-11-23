"""Collect notices for the locked editor and CLI dependencies of one release target."""

import argparse
import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import xml.etree.ElementTree as ET


ROOT = Path(__file__).resolve().parents[1]
EXTRA = ROOT / "release" / "notices"


def command(*args):
    return subprocess.check_output(args, cwd=ROOT, encoding="utf-8")


def dependencies(target):
    # Metadata supplies the unpacked source directories. Its graph also includes
    # optional/dev features, so take the actual release package list from tree.
    graph = json.loads(command("cargo", "metadata", "--locked", "--format-version", "1",
                               "--filter-platform", target))
    tree = command("cargo", "tree", "--locked", "--target", target, "--edges", "normal,build",
                   "-p", "phyro", "-p", "phyro-cli", "--prefix", "none", "--format", "{p}")
    selected = set(re.findall(r"^([\w-]+) v(\S+)", tree, re.MULTILINE))
    packages = sorted(
        (p for p in graph["packages"] if (p["name"], p["version"]) in selected),
        key=lambda p: (p["name"], p["version"]),
    )
    if not selected or len(packages) != len(selected):
        raise ValueError("Could not match the release dependency tree to Cargo metadata.")
    return [p for p in packages if p["id"] not in graph["workspace_members"]]


def protocol_notices(name, source, output):
    # Generated Wayland and GL/EGL bindings have additional notices in their
    # protocol definitions, separate from the Rust wrapper's own license.
    if name.startswith("wayland-"):
        files = sorted(source.rglob("*.xml"))
    elif name == "khronos_api":
        files = sorted(source.glob("api/xml/*.xml")) + [source / "api_egl/api/egl.xml"]
    else:
        return
    entries = []
    for file in files:
        for event, node in ET.iterparse(file, events=("comment", "end")):
            if event == "comment" or node.tag in ("copyright", "comment"):
                text = (node.text or "").strip()
                if "copyright" in text.lower() or "spdx-license-" in text.lower():
                    entries.append(f"{file.relative_to(source).as_posix()}\n{text}")
            node.clear()
    if entries:
        (output / "protocol-notices.txt").write_text("\n\n".join(entries) + "\n",
                                                     encoding="utf-8", newline="\n")


def package_notices(package, destination, rust_docs):
    name, version = package["name"], package["version"]
    source = Path(package["manifest_path"]).resolve().parent
    output = destination / f"{name}-{version}"
    output.mkdir()
    docs = {
        f for f in source.rglob("*")
        if re.search(r"licen[sc]e|copyright|copying|notice|attribution", f.name, re.I)
        and f.is_file()
    }
    if package["license_file"]:
        docs.add(source / package["license_file"])
    if name == "epaint_default_fonts":
        for filename in ["Hack-Regular.txt", "emoji-icon-font-mit-license.txt", "OFL.txt", "UFL.txt"]:
            docs.add(source / "fonts" / filename)
        (output / "fonts").mkdir()
        shutil.copyfile(EXTRA / "font-credits.txt", output / "fonts" / "Font-credits.txt")
    for path in docs:
        relative = path.resolve().relative_to(source)
        dest = output / relative
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(path, dest)
    protocol_notices(name, source, output)

    if not package["license_file"] and not any(f.parent == source for f in docs):
        # These crates publish their SPDX declaration but omit the crate license
        # file. Use the declared Apache option where offered. The two remaining
        # texts come from the exact source revisions of the locked packages.
        declared = package["license"]
        if declared in ("Apache-2.0", "MIT OR Apache-2.0") or (
            name == "epaint_default_fonts"
            and declared == "(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0"
        ):
            fallback = rust_docs / "licenses" / "Apache-2.0.txt"
        elif (name, version, declared) == ("i_key_sort", "0.12.0", "MIT"):
            # github.com/iShape-Rust/iKeySort/blob/688424e09b7758d04e62e350160d164c7384f54b/LICENSE
            fallback = EXTRA / "i_key_sort-MIT.txt"
        elif (name, version, declared) == ("clipboard-win", "5.4.3", "BSL-1.0"):
            # github.com/DoumanAsh/clipboard-win/blob/ed27008ae5d1f10cfbb266de039f0fb35fd05a56/LICENSE
            fallback = EXTRA / "BSL-1.0.txt"
        else:
            raise ValueError(f"Missing license text for {name} {version} ({declared}).")
        shutil.copyfile(fallback, output / fallback.name)

    return "\n".join(filter(None, (
        f"{name} {version}    {package['license'] or package['license_file']}",
        ", ".join(package["authors"]), package["repository"],
    )))


def generate(target, output):
    if output.exists():
        raise ValueError(f"Choose a new notices directory: {output}")
    rust_docs = Path(command("rustc", "--print", "sysroot").strip()) / "share" / "doc" / "rust"
    rust_notice = rust_docs / "COPYRIGHT-library.html"
    if not rust_notice.is_file():
        raise ValueError("Install rustup component rust-docs for the toolchain used to build Phyro.")
    packages = dependencies(target)
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=output.parent) as temporary:
        staging = Path(temporary) / "notices"
        staging.mkdir()
        entries = [package_notices(p, staging, rust_docs) for p in packages]
        standard_library = staging / "rust-standard-library"
        standard_library.mkdir()
        shutil.copyfile(rust_notice, standard_library / rust_notice.name)
        title = (f"Phyro third-party notices for {target}\n"
                 f"Toolchain: {command('rustc', '--version').strip()}\n"
                 "The directories below contain notices for the editor and CLI runtime and build "
                 "dependencies. Font credits and terms are in epaint_default_fonts-*/fonts. "
                 "Rust standard library notices are in rust-standard-library.")
        (staging / "INDEX.txt").write_text("\n\n".join([title, *entries]) + "\n",
                                           encoding="utf-8", newline="\n")
        staging.rename(output)
    print(f"Collected {len(packages)} dependency notice directories for {target}.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True, help="Rust target triple used for the binaries")
    parser.add_argument("--out", required=True, type=Path, help="New directory for collected notices")
    args = parser.parse_args()
    generate(args.target, args.out.resolve())
