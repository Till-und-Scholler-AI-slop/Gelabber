"""Dependency recipe invariants; real Docker cache/runtime checks run separately."""
from pathlib import Path
import json
import shutil
import subprocess
import tempfile
import tomllib
import unittest

ROOT = Path(__file__).resolve().parents[3]
MEMBERS = ('api', 'media', 'shared')


class DependencyRecipe(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.input = Path(self.temp.name) / 'input'
        self.input.mkdir()
        for name in ('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml'):
            shutil.copyfile(ROOT / name, self.input / name)
        for member in MEMBERS:
            (self.input / member).mkdir()
            shutil.copyfile(ROOT / member / 'Cargo.toml', self.input / member / 'Cargo.toml')

    def recipe(self, name='output', check=True):
        output = Path(self.temp.name) / name
        result = subprocess.run(['sh', str(ROOT / 'docker/rust-deps-recipe.sh'),
                                 str(self.input), str(output)], capture_output=True, text=True)
        if check:
            self.assertEqual(result.returncode, 0, result.stderr)
        return output, result

    @staticmethod
    def contents(root):
        return {str(path.relative_to(root)): path.read_bytes()
                for path in root.rglob('*') if path.is_file()}

    def test_preserves_inputs_and_all_external_dependency_data(self):
        before = self.contents(self.input)
        output, _ = self.recipe()
        self.assertEqual(before, self.contents(self.input))
        original = tomllib.loads(before['Cargo.toml'].decode())
        original['workspace']['package']['version'] = '0.0.0'
        self.assertEqual(original, tomllib.loads((output / 'Cargo.toml').read_text()))
        lock = tomllib.loads(before['Cargo.lock'].decode())
        for package in lock['package']:
            if package['name'] in {f'gelabber-{member}' for member in MEMBERS}:
                self.assertNotIn('source', package)
                package['version'] = '0.0.0'
        self.assertEqual(lock, tomllib.loads((output / 'Cargo.lock').read_text()))
        for member in MEMBERS:
            self.assertEqual(before[f'{member}/Cargo.toml'],
                             (output / member / 'Cargo.toml').read_bytes())

    def test_source_and_release_version_changes_keep_recipe_identical(self):
        output, _ = self.recipe()
        original_version = tomllib.loads((self.input / 'Cargo.toml').read_text())['workspace']['package']['version']
        root = self.input / 'Cargo.toml'
        root.write_text(root.read_text().replace(f'version = "{original_version}"', 'version = "99.99.99"', 1))
        lock = self.input / 'Cargo.lock'
        text = lock.read_text()
        for member in MEMBERS:
            text = text.replace(f'name = "gelabber-{member}"\nversion = "{original_version}"',
                                f'name = "gelabber-{member}"\nversion = "99.99.99"')
            (self.input / member / 'src').mkdir()
            (self.input / member / 'src/lib.rs').write_text('compile_error!("real source is not a recipe input");')
        lock.write_text(text)
        changed, _ = self.recipe('changed')
        self.assertEqual(self.contents(output), self.contents(changed))

    def test_cargo_accepts_recipe_with_locked_local_packages_and_targets(self):
        output, _ = self.recipe()
        lock_before = (output / 'Cargo.lock').read_bytes()
        result = subprocess.run(
            ['cargo', 'metadata', '--locked', '--no-deps', '--format-version', '1',
             '--manifest-path', str(output / 'Cargo.toml')],
            capture_output=True, text=True, cwd=output)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(lock_before, (output / 'Cargo.lock').read_bytes())
        packages = {package['name']: package for package in json.loads(result.stdout)['packages']}
        self.assertEqual(set(packages), {f'gelabber-{member}' for member in MEMBERS})
        for member in MEMBERS:
            package = packages[f'gelabber-{member}']
            self.assertEqual(package['version'], '0.0.0')
            kinds = {tuple(target['kind']) for target in package['targets']}
            self.assertIn(('lib',), kinds)
            if member != 'shared':
                self.assertIn(('bin',), kinds)

    def test_dependency_features_and_toolchain_remain_cache_inputs(self):
        output, _ = self.recipe()
        path = self.input / 'api/Cargo.toml'
        path.write_text(path.read_text().replace('"ws"', '"ws", "http2"', 1))
        changed, _ = self.recipe('features')
        self.assertNotEqual(self.contents(output), self.contents(changed))
        path = self.input / 'rust-toolchain.toml'
        path.write_text(path.read_text() + '# changed toolchain input\n')
        changed_again, _ = self.recipe('toolchain')
        self.assertNotEqual(self.contents(changed), self.contents(changed_again))

    def test_missing_or_foreign_workspace_package_fails_closed(self):
        path = self.input / 'Cargo.lock'
        original = path.read_text()
        path.write_text(original.replace('name = "gelabber-api"', 'name = "foreign-api"', 1))
        _, result = self.recipe(check=False)
        self.assertNotEqual(result.returncode, 0)
        path.write_text(original.replace('name = "gelabber-api"',
                                        'name = "gelabber-api"\nsource = "registry+https://example.invalid"', 1))
        _, result = self.recipe('foreign', check=False)
        self.assertNotEqual(result.returncode, 0)

    def test_rejects_existing_output_to_protect_source(self):
        original = self.contents(self.input)
        result = subprocess.run(['sh', str(ROOT / 'docker/rust-deps-recipe.sh'),
                                 str(self.input), str(self.input)], capture_output=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(original, self.contents(self.input))


if __name__ == '__main__':
    unittest.main()
