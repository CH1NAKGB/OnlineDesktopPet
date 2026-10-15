"""Generate installer comparisons for explicitly retired public keys."""

from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "packaging/superseded-keys"


def main() -> None:
    if len(sys.argv) not in (2, 3):
        raise SystemExit("usage: generate-superseded.py OUTPUT.nsh [SOURCE_DIRECTORY]")
    source = Path(sys.argv[2]) if len(sys.argv) == 3 else SOURCE
    lines = ["; Generated from packaging/superseded-keys/*.pem"]
    keys = sorted(source.glob("*.pem"))
    for index, key in enumerate(keys):
        data = key.read_bytes()
        if (b"-----BEGIN PUBLIC KEY-----" not in data
                or b"PRIVATE KEY" in data):
            raise ValueError(f"Not a public-only PEM: {key.name}")
        path = str(key.resolve()).replace("\\", "/")
        if '"' in path or "\n" in path:
            raise ValueError("Unsupported path to superseded public key")
        name = f"superseded-{index}.pem"
        lines += [
            'SetOutPath "$PLUGINSDIR"',
            f'File /oname={name} "{path}"',
            'nsExec::Exec \'"$SYSDIR\\fc.exe" /B '
            f'"$APPDATA\\ChickLink\\relay-public.pem" "$PLUGINSDIR\\{name}"\'',
            'Pop $R0',
            '${If} $R0 == 0',
            '  StrCpy $KeyMatches 1',
            '${EndIf}',
        ]
    Path(sys.argv[1]).write_text("\n".join(lines) + "\n")
    print(f"Superseded public keys included: {len(keys)}")


if __name__ == "__main__":
    main()
