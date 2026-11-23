import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import call, patch

from scripts import draft_release as draft


class DraftReleaseTests(unittest.TestCase):
    version = "0.1.0-alpha.1"
    tag = "v" + version
    commit = "a" * 40

    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.directory = Path(temporary.name)
        self.summary = self.directory / "summary"
        self.enterContext(patch.dict(draft.os.environ, {
            "GITHUB_REF_TYPE": "branch", "GITHUB_STEP_SUMMARY": str(self.summary),
        }))
        self.create = self.enterContext(patch.object(draft, "create_draft"))
        self.upload = self.enterContext(patch.object(draft.subprocess, "run"))
        self.ensure_tag = self.enterContext(patch.object(draft, "ensure_tag"))
        self.sleep = self.enterContext(patch.object(draft.time, "sleep"))
        self.paths = []
        self.assets = []
        for name in (f"Phyro-{self.version}-windows-x64.zip",
                     f"Phyro-{self.version}-linux-x64.tar.gz", "SHA256SUMS"):
            path = self.directory / name
            data = name.encode()
            path.write_bytes(data)
            self.paths.append(str(path))
            self.assets.append({
                "name": name, "digest": "sha256:" + hashlib.sha256(data).hexdigest(),
                "state": "uploaded", "size": len(data),
            })

    def snapshot(self, assets, published=False):
        return {"tag_name": self.tag, "target_commitish": self.commit, "draft": not published,
                "assets": assets, "html_url": "https://example.invalid/release"}

    def run_with_list_responses(self, *responses):
        pages = [json.dumps([[] if release is None else [release]]) for release in responses]
        with patch.object(draft.subprocess, "check_output", side_effect=pages):
            draft.draft_release(self.version, self.commit, self.directory)

    def test_waits_for_created_draft_and_uploaded_digests(self):
        empty = self.snapshot([])
        waiting = self.snapshot([dict(asset, digest=None) for asset in self.assets])
        self.run_with_list_responses(None, None, empty, empty, waiting, self.snapshot(self.assets))
        self.create.assert_called_once()
        self.upload.assert_called_once_with(["gh", "release", "upload", self.tag, *self.paths], check=True)
        self.assertEqual(self.sleep.call_args_list, [call(1), call(1), call(2)])
        self.assertIn(self.tag, self.summary.read_text(encoding="utf-8"))

    def test_retry_only_uploads_missing_assets(self):
        self.run_with_list_responses(self.snapshot(self.assets[:1]), self.snapshot(self.assets))
        self.create.assert_not_called()
        self.upload.assert_called_once_with(
            ["gh", "release", "upload", self.tag, *self.paths[1:]], check=True
        )

    def test_refuses_existing_asset_with_different_bytes(self):
        different = self.snapshot([dict(self.assets[0], digest="sha256:" + "0" * 64)])
        with self.assertRaisesRegex(SystemExit, "different bytes"):
            self.run_with_list_responses(different)
        self.create.assert_not_called()
        self.upload.assert_not_called()

    def test_refuses_published_release(self):
        with self.assertRaisesRegex(SystemExit, "published release"):
            self.run_with_list_responses(self.snapshot([], published=True))
        self.create.assert_not_called()
        self.upload.assert_not_called()
        self.ensure_tag.assert_not_called()
