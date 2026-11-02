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

    def write_archive(self, path, binary=b"fixture", docs=True):
        members = {"punch": binary}
        if docs:
            members.update({"LICENSE": b"license", "README.md": b"readme"})
        with tarfile.open(path, "w:gz") as archive:
            for name, data in members.items():
                member = tarfile.TarInfo(name)
                member.mode = 0o755 if name == "punch" else 0o644
                member.size = len(data)
                archive.addfile(member, io.BytesIO(data))

    def test_package_rejects_symlink_before_creating_output(self):
        binary = self.work / "real"
        binary.write_bytes(b"fixture")
        binary.chmod(0o755)
        link = self.work / "link"
        link.symlink_to(binary)
        output = self.work / "output"
        result = self.run_cli("package", "--target", "x86_64-unknown-linux-musl",
                              "--binary", str(link), "--out", str(output))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("regular", result.stderr)
        self.assertFalse(output.exists())

    def test_smoke_missing_binary_is_concise(self):
        archive = self.work / "empty.tar.gz"
        with tarfile.open(archive, "w:gz"):
            pass
        result = self.run_cli("smoke", "--archive", str(archive))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(str(archive), result.stderr)
        self.assertIn("missing punch", result.stderr)
        self.assertNotIn("Traceback", result.stderr)

    def test_assemble_rejects_corrupt_or_incomplete_archives(self):
        for target in ("x86_64-unknown-linux-musl", "aarch64-apple-darwin",
                       "x86_64-apple-darwin"):
            self.write_archive(self.work / f"punch-v0.1.0-{target}.tar.gz")
        archive = self.work / "punch-v0.1.0-x86_64-apple-darwin.tar.gz"
        for corrupt in (True, False):
            if corrupt:
                archive.write_bytes(b"bad\n")
            else:
                self.write_archive(archive, docs=False)
            result = self.run_cli("assemble", "--dir", str(self.work))
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse((self.work / "SHA256SUMS").exists())
            self.assertFalse((self.work / "punch.rb").exists())

    def test_smoke_rejects_invalid_json_and_wrong_envelopes(self):
        for value in ("not-json", '{"ok":true}',
                      '{"ok":true,"changed":false,"data":[1],"warnings":[]}'):
            binary = ("#!/bin/sh\nif [ \"$1\" = --help ]; then exit 0; fi\n"
                      "printf '%s\\n' '" + value + "'\n").encode()
            archive = self.work / "bad-response.tar.gz"
            self.write_archive(archive, binary)
            result = self.run_cli("smoke", "--archive", str(archive))
            self.assertNotEqual(result.returncode, 0, value)
            self.assertNotIn("Traceback", result.stderr)

    def test_assemble_checks_gzip_trailer_before_writing_metadata(self):
        for target in ("x86_64-unknown-linux-musl", "aarch64-apple-darwin",
                       "x86_64-apple-darwin"):
            self.write_archive(self.work / f"punch-v0.1.0-{target}.tar.gz")
        archive = self.work / "punch-v0.1.0-x86_64-unknown-linux-musl.tar.gz"
        original = archive.read_bytes()
        for damaged in (original[:-8], original[:-8] + b"\x00" * 8):
            archive.write_bytes(damaged)
            result = self.run_cli("assemble", "--dir", str(self.work))
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(str(archive), result.stderr)
            self.assertNotIn("Traceback", result.stderr)
            self.assertFalse((self.work / "SHA256SUMS").exists())
            self.assertFalse((self.work / "punch.rb").exists())

    def test_smoke_rejects_truncated_gzip_without_traceback_or_execution(self):
        marker = self.work / "executed"
        binary = f'#!/bin/sh\ntouch "{marker}"\n'.encode()
        archive = self.work / "truncated.tar.gz"
        self.write_archive(archive, binary)
        original = archive.read_bytes()
        for damaged in (original[:100], original[:-8]):
            archive.write_bytes(damaged)
            result = self.run_cli("smoke", "--archive", str(archive))
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(str(archive), result.stderr)
            self.assertNotIn("Traceback", result.stderr)
            self.assertFalse(marker.exists())

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
            self.write_archive(self.work / name, name.encode())
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
        self.assertNotIn('  version "', formula)
        self.assertIn('    on_intel do\n      url "' +
                      'https://github.com/ContractorKeith/punch/releases/download/v0.1.0/' +
                      names[0] + '"', formula)
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
