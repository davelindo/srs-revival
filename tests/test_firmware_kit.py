import base64
from pathlib import Path
import secrets
import stat
import subprocess
import sys
import tempfile
import unittest


KIT = Path(__file__).resolve().parents[1] / "firmware/kit.py"


class OwnerProvisioning(unittest.TestCase):
    def test_new_owners_get_independent_credentials_and_keep_only_the_public_key(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            blob = b"\x00\x00\x00\x0bssh-ed25519\x00\x00\x00\x20" + secrets.token_bytes(32)
            key = "ssh-ed25519 " + base64.b64encode(blob).decode("ascii")
            public = root / "example.pub"
            public.write_text(key + " owner comment\n")

            def provision(directory):
                return subprocess.run(
                    [sys.executable, "-I", KIT, "owner", "--public-key", public,
                     "--country", "EU", "--interface", "wired", "--output", directory],
                    capture_output=True,
                )

            passwords = []
            for name in ("first", "second"):
                owner = root / name
                self.assertEqual(provision(owner).returncode, 0)
                self.assertEqual((owner / "authorized_keys").read_text(), key + "\n")
                self.assertEqual((owner / "CountryCode").read_text(), "EU\n")
                self.assertEqual((owner / "NetIFMode").read_text(), "wired\n")
                password = owner / "web-password"
                self.assertEqual(stat.S_IMODE(password.stat().st_mode), 0o600)
                self.assertEqual(stat.S_IMODE(owner.stat().st_mode), 0o700)
                passwords.append(password.read_text())

            self.assertNotEqual(*passwords)
            self.assertNotEqual(provision(root / "first").returncode, 0)
            self.assertEqual((root / "first/web-password").read_text(), passwords[0])
