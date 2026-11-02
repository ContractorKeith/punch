"""Exercise the release CLI without network access or a user store."""

import os
import hashlib
import io
import shutil
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest

SCRIPT = Path(__file__).with_name("release.py")
ROOT = SCRIPT.parent.parent


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.work = Path(self.temp.name)
        # A fixed release fixture keeps these tests valid after version bumps.
        self.repo = self.work / "repo"
        (self.repo / "packaging").mkdir(parents=True)
        self.script = self.repo / "packaging" / "release.py"
        shutil.copyfile(SCRIPT, self.script)
        (self.repo / "Cargo.toml").write_text('[workspace.package]\nversion = "0.1.0"\n')
        for name in ("LICENSE", "README.md"):
            shutil.copyfile(ROOT / name, self.repo / name)

    def run_cli(self, *args):
        return subprocess.run(
            [sys.executable, str(self.script), *args],
            cwd=self.repo,
            env={**os.environ, "PUNCH_STATE_DIR": str(self.work / "state")},
            capture_output=True,
            text=True,
        )

    def test_tag_must_exactly_match_manifest_version(self):
        result = self.run_cli("validate", "--tag", "v0.1.0")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), "0.1.0")
        for tag in ("0.1.0", "v0.2.0", "v0.1.0-rc.1", "v0.1.0/evil"):
            with self.subTest(tag=tag):
                result = self.run_cli("validate", "--tag", tag)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("tag must be v0.1.0", result.stderr)

    def test_package_contains_executable_and_license(self):
        binary = self.work / "punch"
        binary.write_bytes(b"release binary fixture\n")
        binary.chmod(0o755)
        result = self.run_cli(
            "package", "--target", "x86_64-unknown-linux-musl",
            "--binary", str(binary), "--out", str(self.work),
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        archive = self.work / "punch-v0.1.0-x86_64-unknown-linux-musl.tar.gz"
        with tarfile.open(archive) as package:
            self.assertEqual(set(package.getnames()), {"punch", "LICENSE", "README.md"})
            self.assertEqual(package.extractfile("punch").read(), binary.read_bytes())
            self.assertEqual(package.getmember("punch").mode, 0o755)
            self.assertEqual(package.extractfile("LICENSE").read(), (ROOT / "LICENSE").read_bytes())

    def test_assemble_requires_all_archives_and_uses_their_real_hashes(self):
        result = self.run_cli("assemble", "--dir", str(self.work))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("missing release archive", result.stderr)
        names = [
            "punch-v0.1.0-x86_64-unknown-linux-musl.tar.gz",
            "punch-v0.1.0-aarch64-apple-darwin.tar.gz",
            "punch-v0.1.0-x86_64-apple-darwin.tar.gz",
        ]
        for name in names:
            # Distinct test bytes, never published as release artifacts.
            (self.work / name).write_bytes(name.encode())
        result = self.run_cli("assemble", "--dir", str(self.work))
        self.assertEqual(result.returncode, 0, result.stderr)
        formula = (self.work / "punch.rb").read_text()
        checksums = (self.work / "SHA256SUMS").read_text()
        for name in names:
            digest = hashlib.sha256((self.work / name).read_bytes()).hexdigest()
            self.assertIn(f"{digest}  {name}\n", checksums)
            self.assertIn(f'sha256 "{digest}"', formula)
            self.assertIn(f"/releases/download/v0.1.0/{name}", formula)
        self.assertIn('bin.install "punch"', formula)
        self.assertIn('ENV["PUNCH_STATE_DIR"] = testpath', formula)
        self.assertIn('depends_on arch: :x86_64', formula)
        if shutil.which("ruby"):
            syntax = subprocess.run(
                ["ruby", "-c", str(self.work / "punch.rb")], capture_output=True, text=True,
            )
            self.assertEqual(syntax.returncode, 0, syntax.stderr)

    def test_smoke_runs_extracted_binary_with_scratch_state_and_fake_tools(self):
        binary = b'''#!/bin/sh
test -n "$PUNCH_STATE_DIR" || exit 91
test "$(command -v gh)" = "${PATH%%:*}/gh" || exit 92
test "$(command -v basecamp)" = "${PATH%%:*}/basecamp" || exit 93
if [ "$1" = "--help" ]; then echo 'Usage: punch'; exit 0; fi
test "$1 $2" = '--json list' || exit 94
printf '{"ok":true,"changed":false,"data":[],"warnings":[]}\n'
'''
        archive = self.work / "package.tar.gz"
        with tarfile.open(archive, "w:gz") as package:
            member = tarfile.TarInfo("punch")
            member.size = len(binary)
            member.mode = 0o755
            package.addfile(member, io.BytesIO(binary))
        result = self.run_cli("smoke", "--archive", str(archive))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('"ok":true', result.stdout)


if __name__ == "__main__":
    unittest.main()
