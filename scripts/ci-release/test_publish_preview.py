"""Exercise the publication sequence without mutating GitHub releases."""
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from publish_preview import publish, staged_assets
from verify_artifacts import PLATFORM_EXTENSIONS, expected_groups, sha256_file, verify


REPO = "example/swarmcraft"
COMMIT = "a" * 40
TAG = f"main-snapshot-{COMMIT}"


class PreviewPublisherTests(unittest.TestCase):
    def setUp(self):
        tmp = self.enterContext(tempfile.TemporaryDirectory())
        root = Path(tmp)
        self.staging = root / "staging"
        self.notes = root / "notes.md"
        self.notes.write_text("Preview notes\n")
        source = root / "source"
        for platform, group in expected_groups("preview", COMMIT).items():
            directory = source / group
            directory.mkdir(parents=True)
            package = directory / f"SwarmCraft_0.5.1_{platform}{PLATFORM_EXTENSIONS[platform][0]}"
            package.write_text(platform)
            package.with_name(package.name + ".sha256").write_text(f"{sha256_file(package)}  {package.name}\n")
        verify(source, self.staging, "preview", COMMIT, "0.5.1")
        self.expected = staged_assets(self.staging, COMMIT)
        self.releases = {"main-latest": {"draft": False, "assets": ["previous-installer"]}}
        self.refs = []
        self.interrupt = False
        self.bad_digest = False
        self.uploads = []
        self.promotions = 0

    def gh(self, command, endpoint, *args):
        if command == "release" and endpoint == "upload":
            tag, flag, repo, *paths = args
            self.assertEqual((tag, flag, repo), (TAG, "--repo", REPO))
            draft = self.releases[tag]
            self.assertTrue(draft["draft"])
            self.uploads.append([Path(path).name for path in paths])
            for name in self.uploads[-1]:
                size, digest = self.expected[name]
                draft["assets"].append({"name": name, "state": "uploaded", "size": size, "digest": digest})
                if self.interrupt:
                    raise subprocess.CalledProcessError(1, "simulated upload interruption")
            if self.bad_digest:
                draft["assets"][-1]["digest"] = "sha256:" + "0" * 64
            return ""
        self.assertEqual(command, "api")
        if endpoint == f"repos/{REPO}/releases?per_page=100":
            return json.dumps(self.releases[TAG]) if TAG in self.releases else ""
        if endpoint == f"repos/{REPO}/releases":
            self.assertIn("draft=true", args)
            self.releases[TAG] = {
                "id": 42, "tag_name": TAG, "prerelease": True, "draft": True,
                "target_commitish": COMMIT, "assets": [],
            }
            return json.dumps(self.releases[TAG])
        if endpoint == f"repos/{REPO}/git/matching-refs/tags/{TAG}":
            return json.dumps(self.refs)
        if endpoint == f"repos/{REPO}/git/refs":
            self.assertIn(f"sha={COMMIT}", args)
            self.refs = [{"ref": f"refs/tags/{TAG}", "object": {"type": "commit", "sha": COMMIT}}]
            return json.dumps(self.refs[0])
        self.assertEqual(endpoint, f"repos/{REPO}/releases/42")
        if args:
            self.assertEqual(args, ("--method", "PATCH", "-F", "draft=false", "-f", "make_latest=false"))
            self.assertEqual({asset["name"] for asset in self.releases[TAG]["assets"]}, self.expected.keys())
            self.assertTrue(self.refs)
            self.releases[TAG]["draft"] = False
            self.promotions += 1
        return json.dumps(self.releases[TAG])

    def publish(self):
        with patch("publish_preview.gh", side_effect=self.gh):
            publish(REPO, COMMIT, self.staging, self.notes)

    def test_interrupted_upload_preserves_last_release_and_can_resume(self):
        self.interrupt = True
        with self.assertRaises(subprocess.CalledProcessError):
            self.publish()
        self.assertEqual(self.releases["main-latest"], {"draft": False, "assets": ["previous-installer"]})
        self.assertTrue(self.releases[TAG]["draft"])
        self.assertEqual(self.promotions, 0)
        self.assertEqual(self.refs, [])

        self.interrupt = False
        self.publish()
        self.publish()
        self.assertEqual(set(self.uploads[1]), self.expected.keys() - {self.uploads[0][0]})
        self.assertEqual(len(self.uploads), 2)
        self.assertEqual(self.promotions, 1)

    def test_corrupt_remote_asset_leaves_release_private(self):
        self.bad_digest = True
        with self.assertRaisesRegex(ValueError, "remote preview asset conflicts"):
            self.publish()
        self.assertTrue(self.releases[TAG]["draft"])
        self.assertEqual(self.promotions, 0)

    def test_existing_wrong_tag_blocks_publication(self):
        self.refs = [{"ref": f"refs/tags/{TAG}", "object": {"type": "commit", "sha": "b" * 40}}]
        with self.assertRaisesRegex(ValueError, "tag is missing or points to another commit"):
            self.publish()
        self.assertTrue(self.releases[TAG]["draft"])
        self.assertEqual(self.promotions, 0)


if __name__ == "__main__":
    unittest.main()
