import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class LinuxInstallTests(unittest.TestCase):
    def setUp(self):
        scratch = ROOT / "target" / "installer-tests"
        scratch.mkdir(parents=True, exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(dir=scratch)
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.bundle = self.base / "bundle with spaces"
        self.bundle.mkdir()
        shutil.copy2(ROOT / "packaging/linux/install.sh", self.bundle / "install.sh")
        (self.bundle / "bin").mkdir()
        for name in ["filmcraft", "filmcraft-cli"]:
            (self.bundle / "bin" / name).write_text("#!/bin/sh\nprintf 'FilmCraft installer test\\n'\n")
        (self.bundle / "share/doc/filmcraft").mkdir(parents=True)
        (self.bundle / "share/doc/filmcraft/LICENSE").write_text("test licence\n")
        for src, dst in [
            ("packaging/linux/ai.storyteller.filmcraft.desktop", "share/applications/ai.storyteller.filmcraft.desktop"),
            ("packaging/linux/ai.storyteller.filmcraft.mime.xml", "share/mime/packages/ai.storyteller.filmcraft.xml"),
            ("assets/app-icon/hicolor/256x256/apps/ai.storyteller.filmcraft.png", "share/icons/hicolor/256x256/apps/ai.storyteller.filmcraft.png"),
        ]:
            dest = self.bundle / dst
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / src, dest)
        self.prefix = self.base / "install with spaces"

    def run_install(self, *args, env=None):
        return subprocess.run(
            ["sh", str(self.bundle / "install.sh"), *args],
            cwd=ROOT,
            env=env,
            text=True,
            capture_output=True,
            check=False,
        )

    def test_install_from_another_directory_and_upgrade_preserve_unrelated_files(self):
        for _ in range(2):
            result = self.run_install("--prefix", str(self.prefix))
            self.assertEqual(result.returncode, 0, result.stderr)
            for name in ["filmcraft", "filmcraft-cli"]:
                binary = self.prefix / "bin" / name
                self.assertTrue(os.access(binary, os.X_OK))
                result = subprocess.run([str(binary), "--version"], capture_output=True, text=True, check=True)
                self.assertEqual(result.stdout, "FilmCraft installer test\n")
            self.assertEqual((self.prefix / "share/doc/filmcraft/LICENSE").read_text(), "test licence\n")
            marker = self.prefix / "share/user-data"
            if marker.exists():
                self.assertEqual(marker.read_text(), "keep\n")
            marker.write_text("keep\n")

    def test_desktop_entry_uses_the_chosen_prefix_without_a_path_entry(self):
        prefixes = [self.prefix, self.base / 'quotes" backslash\\ dollar$ tick` percent%F']
        for prefix in prefixes:
            result = self.run_install("--prefix", str(prefix))
            self.assertEqual(result.returncode, 0, result.stderr)
            desktop = prefix / "share/applications/ai.storyteller.filmcraft.desktop"
            validator = shutil.which("desktop-file-validate")
            if validator:
                result = subprocess.run([validator, str(desktop)], capture_output=True, text=True, check=False)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            # GLib parses both escaping layers, honors TryExec and expands desktop field codes.
            try:
                import gi
                gi.require_version("Gio", "2.0")
                from gi.repository import Gio
            except (ImportError, ValueError):
                continue
            info = Gio.DesktopAppInfo.new_from_filename(str(desktop))
            self.assertIsNotNone(info)
            marker = self.base / "launched"
            binary = prefix / "bin/filmcraft"
            binary.write_text("#!/bin/sh\nprintf '%s\\n' \"$1\" > \"$FILMCRAFT_INSTALL_TEST_MARKER\"\n")
            os.chmod(binary, 0o755)
            launch = Gio.AppLaunchContext()
            launch.setenv("PATH", "/usr/bin:/bin")
            launch.setenv("FILMCRAFT_INSTALL_TEST_MARKER", str(marker))
            self.assertTrue(info.launch([Gio.File.new_for_path(str(self.base / "media with spaces.mov"))], launch))
            import time
            for _ in range(100):
                if marker.exists():
                    break
                time.sleep(0.01)
            self.assertEqual(marker.read_text(), str(self.base / "media with spaces.mov") + "\n")
            marker.unlink()

    def test_invalid_arguments_do_not_create_a_destination(self):
        for args in [("--prefix",), ("--prefix", "relative"), ("--prefix", ""), ("--unknown",)]:
            result = self.run_install(*args)
            self.assertEqual(result.returncode, 2, result.stderr)
        self.assertFalse(self.prefix.exists())

    def test_incomplete_bundle_fails_before_installing(self):
        (self.bundle / "bin/filmcraft-cli").unlink()
        result = self.run_install("--prefix", str(self.prefix))
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertFalse(self.prefix.exists())

    def test_help_and_explicit_prefix_work_without_home(self):
        env = os.environ.copy()
        env.pop("HOME", None)
        self.assertEqual(self.run_install("--help", env=env).returncode, 0)
        result = self.run_install("--prefix", str(self.prefix), env=env)
        self.assertEqual(result.returncode, 0, result.stderr)
        result = self.run_install(env=env)
        self.assertEqual(result.returncode, 2)
        self.assertIn("--prefix", result.stderr)

    def test_help_exits_without_installing(self):
        result = self.run_install("--help")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("--prefix", result.stdout)
        self.assertFalse(self.prefix.exists())


if __name__ == "__main__":
    unittest.main()
