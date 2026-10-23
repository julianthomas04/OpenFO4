"""The collision worker spells the meshes folder like the installer's InstallLayout (src/installer/install_layout.cs)."""

import re
import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO_ROOT / "src" / "installer" / "python"))
import fo4_collision_worker as worker  # noqa: E402

INSTALL_LAYOUT = REPO_ROOT / "src" / "installer" / "install_layout.cs"
MESHES_FOLDER_DECLARATION = re.compile(r'public const string MeshesFolder = "([^"]*)";')


class MeshesFolderTests(unittest.TestCase):
    def test_the_worker_keys_the_manifest_below_the_meshes_folder_of_the_installer_layout(self):
        declaration = MESHES_FOLDER_DECLARATION.search(INSTALL_LAYOUT.read_text(encoding="utf-8"))

        self.assertIsNotNone(declaration, "InstallLayout no longer declares MeshesFolder as a string constant")
        self.assertEqual(declaration.group(1), worker.MESHES_FOLDER)


if __name__ == "__main__":
    unittest.main()
