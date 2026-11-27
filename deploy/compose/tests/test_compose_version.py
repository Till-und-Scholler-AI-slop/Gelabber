"""compose.yaml starts the app images of the backend workspace's version.

publish-images.yml pushes api, media and web under the version in
backend/Cargo.toml, and an operator who pulls compose.yaml gets the images it
names. This held only in the images job of a pull request that built an image
(GEL-390); here every change to compose.yaml or either version runs it.
"""
from pathlib import Path
import re
import tomllib
import unittest

ROOT = Path(__file__).resolve().parents[3]
IMAGES = ('api', 'media', 'web')


def pinned_tags(compose):
    """The default tag of each app image in compose.yaml, by image name."""
    found = {}
    for name, tag in re.findall(
            r'^\s+image: \$\{GELABBER_[A-Z]+_IMAGE:-ghcr\.io/[^/\s]+/[^/\s]+/([a-z]+):([^}\s]+)\}$',
            compose, re.M):
        found.setdefault(name, set()).add(tag)
    return found


class ComposeVersion(unittest.TestCase):
    def test_compose_pins_the_app_images_of_the_backend_version(self):
        version = tomllib.loads((ROOT / 'backend/Cargo.toml').read_text())['workspace']['package']['version']
        tags = pinned_tags((ROOT / 'deploy/compose/compose.yaml').read_text())
        for image in IMAGES:
            with self.subTest(image=image):
                self.assertEqual(tags.get(image), {f'v{version}'},
                                 f'compose.yaml must start {image}:v{version}, the backend\'s version')


if __name__ == '__main__':
    unittest.main()
