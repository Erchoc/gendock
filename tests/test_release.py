"""Boundary and version regression checks for the weekly release planner."""
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('release', Path(__file__).parents[1] / 'scripts/release.py')
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class ReleasePlanTests(unittest.TestCase):
    def check_plan(self, count, tags='', force=False, current='0.1.0'):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        root = Path(temporary.name)
        (root / 'Cargo.toml').write_text(f'[package]\nversion = "{current}"\n')
        (root / 'Cargo.lock').write_text(f'[[package]]\nname = "gendock"\nversion = "{current}"\n\n[[package]]\nname = "dependency"\nversion = "1.0.0"\n')
        output = root / 'output'

        def command(*args):
            if args[:2] == ('git', 'tag'):
                return tags
            if args[:2] == ('git', 'rev-list'):
                return str(count)
            if args[:2] == ('git', 'rev-parse'):
                return 'abc123'
            return ''

        with patch.object(release, 'ROOT', root), patch.object(release, 'STATE', root / 'target/plan.json'), patch.object(release, 'run', side_effect=command), patch.dict(os.environ, {'FORCE': str(force).lower(), 'GITHUB_OUTPUT': str(output)}):
            release.plan()
        return root, output.read_text()

    def test_three_commits_skip_without_version_change(self):
        root, output = self.check_plan(3)
        self.assertEqual(output, 'release=false\n')
        self.assertFalse((root / 'target').exists())

    def test_four_commits_release_and_increment_patch(self):
        root, output = self.check_plan(4, tags='v0.1.0')
        self.assertEqual(output, 'release=true\n')
        self.assertEqual(json.loads((root / 'target/plan.json').read_text())['version'], '0.1.1')
        lock = (root / 'Cargo.lock').read_text()
        self.assertIn('name = "gendock"\nversion = "0.1.1"', lock)
        self.assertIn('name = "dependency"\nversion = "1.0.0"', lock)

    def test_first_forced_release_keeps_current_version(self):
        root, _ = self.check_plan(0, force=True)
        self.assertEqual(json.loads((root / 'target/plan.json').read_text())['version'], '0.1.0')

    def test_higher_declared_version_is_preserved(self):
        root, _ = self.check_plan(4, tags='v0.1.5', current='0.2.0')
        self.assertEqual(json.loads((root / 'target/plan.json').read_text())['version'], '0.2.0')
