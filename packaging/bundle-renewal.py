"""Build a private credential transport archive; never print credential values."""
import io
import json
import os
from pathlib import Path
import tarfile

root = Path(__file__).resolve().parents[1]
directory = root / "deploy/secrets/renewal"
credentials = json.loads((directory / "credentials.json").read_text())
os.umask(0o077)
with (directory / "transport.tar").open("xb") as output:
    with tarfile.open(fileobj=output, mode="w") as archive:
        for name, key in (("access-key-id", "access_key_id"),
                          ("access-key-secret", "access_key_secret")):
            data = credentials[key].encode()
            info = tarfile.TarInfo(name)
            info.mode = 0o600
            info.size = len(data)
            archive.addfile(info, io.BytesIO(data))
print("Private renewal transport archive prepared.")
