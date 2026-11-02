"""Small, offline release helpers. Run from any directory with Python 3.11+."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parent.parent
TARGETS = (
    "x86_64-unknown-linux-musl",
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
)


def version():
    with (ROOT / "Cargo.toml").open("rb") as manifest:
        value = tomllib.load(manifest)["workspace"]["package"]["version"]
    if not re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", value):
        raise ValueError("release version must be stable major.minor.patch")
    return value


def archive_name(release_version, target):
    return f"punch-v{release_version}-{target}.tar.gz"


def package(args, release_version):
    if not args.binary.is_file() or not args.binary.stat().st_mode & 0o111:
        raise ValueError("binary must be an executable file")
    args.out.mkdir(parents=True, exist_ok=True)
    archive = args.out / archive_name(release_version, args.target)
    with tarfile.open(archive, "w:gz") as output:
        for source, name in (
            (args.binary, "punch"), (ROOT / "LICENSE", "LICENSE"),
            (ROOT / "README.md", "README.md"),
        ):
            output.add(source, arcname=name, recursive=False)
    print(archive)


def assemble(directory, release_version):
    hashes = {}
    for target in TARGETS:
        name = archive_name(release_version, target)
        archive = directory / name
        if not archive.is_file():
            raise ValueError(f"missing release archive: {name}")
        hashes[target] = hashlib.sha256(archive.read_bytes()).hexdigest()
    (directory / "SHA256SUMS").write_text("".join(
        f"{hashes[target]}  {archive_name(release_version, target)}\n"
        for target in TARGETS
    ))
    base_url = f"https://github.com/ContractorKeith/punch/releases/download/v{release_version}"
    formula = [
        "# Generated from release archives by packaging/release.py; do not hand-edit hashes.",
        "class Punch < Formula",
        '  desc "Terminal punch list for you and your coding agents"',
        '  homepage "https://github.com/ContractorKeith/punch"',
        f'  version "{release_version}"',
        '  license "MIT"',
        "",
        "  on_macos do",
    ]
    for arch, target in (("arm", TARGETS[1]), ("intel", TARGETS[2])):
        formula.extend([
            f"    on_{arch} do",
            f'      url "{base_url}/{archive_name(release_version, target)}"',
            f'      sha256 "{hashes[target]}"',
            "    end",
        ])
    formula.extend([
        "  end", "", "  on_linux do",
        "    depends_on arch: :x86_64",
        f'    url "{base_url}/{archive_name(release_version, TARGETS[0])}"',
        f'    sha256 "{hashes[TARGETS[0]]}"',
        "  end", "", "  def install", '    bin.install "punch"', "  end", "",
        "  test do", '    ENV["PUNCH_STATE_DIR"] = testpath',
        '    assert_match "Usage:", shell_output("#{bin}/punch --help")',
        '    assert_match \'"ok":true\', shell_output("#{bin}/punch --json list")',
        "  end", "end", "",
    ])
    (directory / "punch.rb").write_text("\n".join(formula))


def smoke(archive):
    with tempfile.TemporaryDirectory(prefix="punch-package-") as temporary:
        scratch = Path(temporary)
        binary = scratch / "punch"
        # Read just the binary, never extract arbitrary archive paths or links.
        with tarfile.open(archive) as package_file:
            member = package_file.getmember("punch")
            if not member.isfile():
                raise ValueError("package punch must be a regular file")
            with package_file.extractfile(member) as source:
                binary.write_bytes(source.read())
        binary.chmod(0o755)
        fake_bin = scratch / "fake-bin"
        fake_bin.mkdir()
        for tool in ("gh", "basecamp"):
            fake = fake_bin / tool
            fake.write_text('#!/bin/sh\necho "unexpected external tool call" >&2\nexit 97\n')
            fake.chmod(0o755)
        env = {**os.environ, "PUNCH_STATE_DIR": str(scratch / "state"),
               "PATH": str(fake_bin) + os.pathsep + os.environ["PATH"]}
        for tool in ("gh", "basecamp"):
            if shutil.which(tool, path=env["PATH"]) != str(fake_bin / tool):
                raise ValueError(f"fake {tool} must be first on PATH")
        subprocess.run([str(binary), "--help"], env=env, check=True, timeout=30)
        result = subprocess.run([str(binary), "--json", "list"], env=env,
                                check=True, capture_output=True, text=True, timeout=30)
        expected = {"ok": True, "changed": False, "data": [], "warnings": []}
        if json.loads(result.stdout) != expected:
            raise ValueError(f"unexpected empty-store list: {result.stdout}")
        print(result.stdout, end="")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    validate = commands.add_parser("validate", help="validate tag against Cargo.toml")
    validate.add_argument("--tag")
    pack = commands.add_parser("package", help="archive a built release binary")
    pack.add_argument("--target", choices=TARGETS, required=True)
    pack.add_argument("--binary", type=Path, required=True)
    pack.add_argument("--out", type=Path, required=True)
    combine = commands.add_parser("assemble", help="write checksums and Homebrew formula")
    combine.add_argument("--dir", type=Path, required=True)
    check = commands.add_parser("smoke", help="run an extracted package in isolation")
    check.add_argument("--archive", type=Path, required=True)
    args = parser.parse_args()
    release_version = version()
    if args.command == "validate":
        if args.tag is not None and args.tag != f"v{release_version}":
            raise ValueError(f"tag must be v{release_version}")
        print(release_version)
    elif args.command == "package":
        package(args, release_version)
    elif args.command == "assemble":
        assemble(args.dir, release_version)
    elif args.command == "smoke":
        smoke(args.archive)


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, tarfile.TarError, subprocess.SubprocessError) as error:
        sys.exit(str(error))
