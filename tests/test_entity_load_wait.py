"""Exercise the command oracle's entity-loading deadline without Minecraft."""
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


@unittest.skipUnless(shutil.which("javac") and shutil.which("java"), "Java is not installed")
class EntityLoadWaitTest(unittest.TestCase):
    def test_readiness_and_timeout(self):
        helper = ROOT / "mods/horizon/src/test/java/dev/horizonlab/parity/EntityLoadWait.java"
        fixture = helper.with_name("EntityLoadWaitTest.java")
        with tempfile.TemporaryDirectory() as classes:
            subprocess.run(["javac", "-d", classes, str(helper), str(fixture)], check=True)
            subprocess.run(["java", "-cp", classes, "dev.horizonlab.parity.EntityLoadWaitTest"], check=True)
