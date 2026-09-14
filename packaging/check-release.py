"""Verify public release archives do not contain private connection artifacts."""
import json
from pathlib import Path
import zipfile

root = Path(__file__).resolve().parents[1]
tokens = [json.loads((root / "dist/cloud-20260912" / role /
                      "chicklink-connection.json").read_text())["access_token"].encode()
          for role in ("mac", "windows")]
assets = list((root / "dist").glob("*-v0.3.4.zip"))
assert len(assets) == 2
for asset in assets:
    with zipfile.ZipFile(asset) as archive:
        assert archive.testzip() is None, "Corrupt archive"
        for name in archive.namelist():
            parts = Path(name).parts
            assert "secrets" not in parts, "Private directory in release"
            assert Path(name).name not in ("chicklink-connection.json", "connection.json", ".env"), "Private config in release"
            data = archive.read(name)
            assert all(token not in data for token in tokens), "Private token in release"
    print(asset.name + ": public archive check passed")
binary = root / "dist/ChickLink-windows-x64-v0.3.4.exe"
assert all(token not in binary.read_bytes() for token in tokens)
print("Public executable token check passed")
