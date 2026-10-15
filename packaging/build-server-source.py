"""Build a public server source archive from tracked files only."""

from pathlib import Path
import subprocess
import sys
import tempfile
import zipfile

from public_archive import check_archive, check_member


ROOT = Path(__file__).resolve().parents[1]
SOURCE_PATHS = (
    "Cargo.toml", "Cargo.lock", "README.md", ".dockerignore",
    "shared-protocol", "relay-server", "pet-client", "deploy",
)


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit("usage: build-server-source.py OUTPUT.zip")
    destination = Path(sys.argv[1]).resolve()
    tracked = subprocess.check_output(
        ["git", "ls-files", "-z", "--", *SOURCE_PATHS], cwd=ROOT
    ).decode().split("\0")
    tracked = [name for name in tracked if name]
    if not tracked:
        raise RuntimeError("No tracked source files found")
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(
        suffix=".zip", prefix="chicklink-source-", dir=destination.parent,
        delete=False,
    ) as temporary:
        pending = Path(temporary.name)
    try:
        with zipfile.ZipFile(pending, "w", zipfile.ZIP_DEFLATED) as archive:
            for name in tracked:
                source = ROOT / name
                if not source.is_file() or source.is_symlink():
                    raise ValueError(f"Tracked source is missing or a symlink: {name}")
                data = source.read_bytes()
                check_member(name, data)
                archive.write(source, arcname=name)
        check_archive(pending)
        pending.replace(destination)
    finally:
        pending.unlink(missing_ok=True)
    print(f"Public server source archive ready: {destination}")


if __name__ == "__main__":
    main()
