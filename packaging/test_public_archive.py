"""Regression checks for the private files once included in v0.3.4."""

from pathlib import Path
import tempfile
import unittest
import zipfile

from public_archive import check_archive


class PublicArchiveTests(unittest.TestCase):
    def check_fixture(self, name: str, content: bytes) -> None:
        with tempfile.TemporaryDirectory() as directory:
            archive = Path(directory) / "source.zip"
            with zipfile.ZipFile(archive, "w") as output:
                output.writestr(name, content)
            with self.assertRaises(ValueError):
                check_archive(archive)

    def test_lan_test_directory_is_rejected(self) -> None:
        self.check_fixture("deploy/lan-test.local/client-tokens.env", b"test")

    def test_private_pem_is_rejected_even_under_an_innocent_name(self) -> None:
        self.check_fixture("deploy/notes.txt", b"-----BEGIN PRIVATE KEY-----\n")

    def test_token_mapping_is_rejected(self) -> None:
        self.check_fixture("deploy/access-tokens.json", b"{}")

    def test_tracked_source_is_accepted(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            archive = Path(directory) / "source.zip"
            with zipfile.ZipFile(archive, "w") as output:
                output.writestr("deploy/relay.sh", b"#!/bin/sh\n")
            check_archive(archive)


if __name__ == "__main__":
    unittest.main()
