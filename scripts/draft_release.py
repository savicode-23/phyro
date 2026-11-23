"""Attach tested packages to an unpublished GitHub Release, using the runner's Git and gh."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time


def remote_commit(tag):
    ref = f"refs/tags/{tag}"
    # An annotated tag advertises both its tag object and its peeled commit.
    lines = subprocess.check_output(
        ["git", "ls-remote", "origin", ref, ref + "^{}"], text=True
    ).splitlines()
    refs = {name: sha for sha, name in (line.split() for line in lines)}
    return refs.get(ref + "^{}", refs.get(ref))


def ensure_tag(tag, commit, create):
    target = remote_commit(tag)
    if target is None and create:
        # A concurrent tag creation is fine when it points to the same commit.
        subprocess.run(["git", "push", "origin", f"{commit}:refs/tags/{tag}"], check=False)
        target = remote_commit(tag)
    if target != commit:
        raise SystemExit(f"Expected {tag} at tested commit {commit}; remote tag is {target!r}.")


def find_draft(tag):
    # This authenticated endpoint includes drafts. Looking up only published
    # releases by tag could miss a draft left by an interrupted upload.
    pages = json.loads(subprocess.check_output([
        "gh", "api", "--paginate", "--slurp", "repos/{owner}/{repo}/releases?per_page=100",
    ]))
    matches = [release for page in pages for release in page if release["tag_name"] == tag]
    if len(matches) > 1:
        raise SystemExit(f"Found multiple releases for {tag}. Review them on GitHub.")
    if matches and not matches[0]["draft"]:
        raise SystemExit(f"{tag} already has a published release. Choose a new version.")
    return matches[0] if matches else None


def wait_for_draft(tag, hashes=None):
    # GitHub's release list can briefly lag behind a successful create or upload.
    for delay in (0, 1, 2, 4, 8, 16, 32):
        if delay:
            time.sleep(delay)
        release = find_draft(tag)
        if release is None:
            continue
        if hashes is None:
            return release
        uploaded = {asset["name"]: asset.get("digest") for asset in release["assets"]}
        if all(uploaded.get(path.name) == digest for path, digest in hashes.items()):
            return release
    if release is None:
        raise SystemExit(f"GitHub did not return the draft for {tag} after a minute.")
    raise SystemExit("GitHub did not return matching digests for the packages and SHA256SUMS after a minute.")


def create_draft(version, commit, windows, linux):
    tag = f"v{version}"
    highlights = Path("release/notes", f"{version}.md")
    notes = highlights.read_text(encoding="utf-8").strip() + "\n\n" if highlights.is_file() else ""
    notes += (
        "## Downloads\n\n"
        f"- On **Windows x64**, extract `{windows.name}` and run `phyro.exe`.\n"
        f"- On **Ubuntu 24.04 x64**, extract `{linux.name}` and run `./phyro`.\n\n"
        "Both packages include the editor, the command-line tools and example projects. "
        "Use `SHA256SUMS` to check the downloaded archives.\n"
    )
    with tempfile.TemporaryDirectory() as directory:
        body = Path(directory, "notes.md")
        body.write_text(notes, encoding="utf-8")
        command = [
            "gh", "release", "create", tag, "--draft", "--verify-tag", "--target", commit,
            "--title", f"Phyro {version}", "--notes-file", str(body),
        ]
        if "-" in version.split("+", 1)[0]:
            command.append("--prerelease")
        subprocess.run(command, check=True)


def draft_release(version, commit, directory):
    tag = f"v{version}"
    name = "Phyro-" + version.replace("+", "_")
    windows = directory / f"{name}-windows-x64.zip"
    linux = directory / f"{name}-linux-x64.tar.gz"
    checksums = directory / "SHA256SUMS"
    hashes = {}
    for asset in (windows, linux, checksums):
        with asset.open("rb") as file:
            hashes[asset] = "sha256:" + hashlib.file_digest(file, "sha256").hexdigest()

    release = find_draft(tag)
    if release is not None:
        target = release["target_commitish"]
        if re.fullmatch(r"[0-9a-fA-F]{40}|[0-9a-fA-F]{64}", target) and target.lower() != commit:
            raise SystemExit(f"The existing draft targets {target}, but this build is {commit}.")
    ensure_tag(tag, commit, release is None and os.environ["GITHUB_REF_TYPE"] == "branch")
    if release is None:
        create_draft(version, commit, windows, linux)
        release = wait_for_draft(tag)

    existing = {asset["name"]: asset for asset in release["assets"]}
    missing = []
    unfinished = []
    for asset, digest in hashes.items():
        previous = existing.get(asset.name)
        if previous is None:
            missing.append(str(asset))
        elif previous["state"] == "starter" and previous["size"] == 0 and not previous.get("digest"):
            # GitHub can leave an empty placeholder when an upload returns HTTP 502.
            unfinished.append(previous["id"])
            missing.append(str(asset))
        elif previous.get("digest") != digest:
            raise SystemExit(
                f"The draft already contains different bytes for {asset.name}. "
                "Re-run the original failed job to reuse its packages, or review the draft on GitHub."
            )
    for asset_id in unfinished:
        subprocess.run([
            "gh", "api", "--method", "DELETE", f"repos/{{owner}}/{{repo}}/releases/assets/{asset_id}",
        ], check=True)
    if missing:
        # Keep existing notes and assets. A retry after a partial upload sends only missing files.
        subprocess.run(["gh", "release", "upload", tag, *missing], check=True)
    ensure_tag(tag, commit, create=False)
    release = wait_for_draft(tag, hashes)
    with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as summary:
        summary.write(f"Draft **{tag}** is ready for review in [Releases]({release['html_url']}).\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version")
    parser.add_argument("commit")
    parser.add_argument("directory", type=Path)
    args = parser.parse_args()
    draft_release(args.version, args.commit, args.directory)
