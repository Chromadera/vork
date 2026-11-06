#!/usr/bin/env python3
"""Exercise the real POSIX and PowerShell installers against local release fixtures."""
import hashlib
import io
import os
import platform
from pathlib import Path
import subprocess
import tarfile
import tempfile
import shutil
import unittest
import zipfile

INSTALLER = Path(__file__).resolve().parents[1] / 'install.sh'
PS_INSTALLER = INSTALLER.with_name('install.ps1')

@unittest.skipIf(os.name == 'nt', 'POSIX installer')
class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.env = dict(os.environ, PREFIX=str(self.root / 'prefix'), HOME=str(self.root / 'home'),
                        VERSION='test', VORK_DOWNLOAD_BASE=self.root.as_uri())
        self.archive = self.root / ('vork-test-darwin-arm64.tar.gz' if platform.system() == 'Darwin' else 'vork-test-linux-x64.tar.gz')
        self.fixture()

    def tearDown(self):
        self.tmp.cleanup()

    def fixture(self, member='vork'):
        with tarfile.open(self.archive, 'w:gz') as tar:
            for name, data, mode in [(member, b'#!/bin/sh\necho fixture "$@"\n', 0o755),
                                     ('examples/scene.json', b'{}', 0o644),
                                     ('slang/bin/slangc', b'compiler', 0o755),
                                     ('oidn/lib/model', b'model', 0o644),
                                     ('NOTICE', b'notices', 0o644)]:
                info = tarfile.TarInfo(name); info.size = len(data); info.mode = mode
                tar.addfile(info, io.BytesIO(data))
        self.checksum()

    def checksum(self):
        (self.root / 'SHA256SUMS').write_text(hashlib.sha256(self.archive.read_bytes()).hexdigest() +
                                            '  ' + self.archive.name + '\n')

    def run_installer(self, *args):
        return subprocess.run(['sh', str(INSTALLER), *args], cwd=self.root, env=self.env,
                              capture_output=True, text=True)

    def test_global_install_and_setup(self):
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.root / 'prefix/lib/vork/NOTICE').exists())
        self.assertIn('fixture setup', result.stdout)
        self.assertTrue((self.root / 'prefix/bin/vork').is_symlink())
        self.assertTrue((self.root / 'prefix/lib/vork/slang/bin/slangc').exists())
        self.assertTrue((self.root / 'prefix/lib/vork/oidn/lib/model').exists())

    def test_skip_setup(self):
        self.env['VORK_SKIP_SETUP'] = '1'
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn('fixture setup', result.stdout)
        self.assertIn('fixture --version', result.stdout)

    def test_relative_prefix(self):
        self.env['PREFIX'] = 'relative-prefix'
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.root / 'relative-prefix/bin/vork').resolve().is_file())

    def test_corruption_fails_before_install(self):
        self.archive.write_bytes(self.archive.read_bytes() + b'corruption')
        self.assertNotEqual(self.run_installer().returncode, 0)
        self.assertFalse((self.root / 'prefix/bin/vork').exists())

    def test_missing_checksum_fails(self):
        (self.root / 'SHA256SUMS').write_text('')
        self.assertNotEqual(self.run_installer().returncode, 0)

    def test_duplicate_checksum_fails(self):
        sums = self.root / 'SHA256SUMS'; sums.write_text(sums.read_text() * 2)
        self.assertNotEqual(self.run_installer().returncode, 0)

    def test_traversal_fails(self):
        self.fixture('../vork')
        self.assertNotEqual(self.run_installer().returncode, 0)
        self.assertFalse((self.root / 'prefix/bin/vork').exists())

    def test_rerun_removes_files_a_release_dropped(self):
        self.assertEqual(self.run_installer().returncode, 0)
        stale = self.root / 'prefix/lib/vork/dropped-by-new-release'
        stale.write_text('old')
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(stale.exists())
        self.assertTrue((self.root / 'prefix/lib/vork/NOTICE').exists())
        self.assertEqual([p.name for p in (self.root / 'prefix/lib').iterdir()], ['vork'])

    def test_failed_download_names_the_url_and_keeps_the_install(self):
        self.assertEqual(self.run_installer().returncode, 0)
        self.env['VORK_DOWNLOAD_BASE'] = (self.root / 'absent').as_uri()
        result = self.run_installer()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Could not download', result.stderr)
        self.assertIn('absent/' + self.archive.name, result.stderr)
        self.assertTrue((self.root / 'prefix/lib/vork/NOTICE').exists())

    def test_default_prefix_adds_path_once_for_a_new_shell(self):
        home = self.root / 'home'; home.mkdir()
        del self.env['PREFIX']
        self.env.update(SHELL='/bin/zsh', PATH='/usr/bin:/bin:/usr/sbin:/sbin')
        for _ in range(2):
            result = self.run_installer()
            self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((home / '.zshenv').read_text().count('export PATH="$HOME/.local/bin:$PATH"'), 1)
        self.assertIn('New shells that read', result.stdout)
        self.assertIn(str(home / '.local/bin/vork'), result.stdout)

    @unittest.skipUnless(shutil.which('bash'), 'needs bash')
    def test_bash_login_shell_finds_vork_past_a_stock_early_return(self):
        home = self.root / 'home'; home.mkdir()
        (home / '.bashrc').write_text('[ -z "$PS1" ] && return\nalias ll="ls -l"\n')
        (home / '.profile').write_text('if [ -f ~/.bashrc ]; then . ~/.bashrc; fi\nmesg n 2>/dev/null || true\n')
        del self.env['PREFIX']
        self.env.update(SHELL='/bin/bash', PATH='/usr/bin:/bin:/usr/sbin:/sbin')
        for _ in range(2):
            self.assertEqual(self.run_installer().returncode, 0)
        for name in ('.bashrc', '.profile'):
            self.assertEqual((home / name).read_text().count('export PATH="$HOME/.local/bin:$PATH"'), 1)
        env = {'HOME': str(home), 'PATH': '/usr/bin:/bin:/usr/sbin:/sbin'}
        login = subprocess.run(['bash', '-lc', 'vork --version'], env=env, capture_output=True, text=True)
        self.assertEqual(login.returncode, 0, login.stderr)
        self.assertIn('fixture --version', login.stdout)

    def test_bash_keeps_an_existing_bash_profile_as_the_login_file(self):
        home = self.root / 'home'; home.mkdir()
        (home / '.bash_profile').write_text('# mine\n')
        del self.env['PREFIX']
        self.env.update(SHELL='/bin/bash', PATH='/usr/bin:/bin')
        self.assertEqual(self.run_installer().returncode, 0)
        self.assertIn('.local/bin', (home / '.bash_profile').read_text())
        self.assertFalse((home / '.profile').exists())

    def test_custom_prefix_and_opt_out_leave_startup_files_alone(self):
        home = self.root / 'home'; home.mkdir()
        self.env.update(SHELL='/bin/zsh', PATH='/usr/bin:/bin:/usr/sbin:/sbin')
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('prefix/bin:$PATH', result.stdout)
        del self.env['PREFIX']
        self.env['VORK_NO_MODIFY_PATH'] = '1'
        self.assertEqual(self.run_installer().returncode, 0)
        self.assertFalse((home / '.zshenv').exists())

    def test_dry_run_downloads_and_writes_nothing(self):
        self.env['VORK_DOWNLOAD_BASE'] = 'https://github.com/voyager-so/vork/releases/download/vtest'
        result = self.run_installer('--dry-run')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('SHA256SUMS', result.stdout)
        self.assertFalse((self.root / 'prefix').exists())

@unittest.skipUnless(os.name == 'nt', 'PowerShell installer is Windows-only')
class PowerShellInstallerTests(unittest.TestCase):
    """VORK_TEST_BINARY (a real vork.exe on a GPU host) enables the full install and warm-up."""
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.env = dict(os.environ, PREFIX=str(self.root / 'prefix'), USERPROFILE=str(self.root / 'home'),
                        HOME=str(self.root / 'home'), VERSION='test', VORK_DOWNLOAD_BASE=self.root.as_uri())
        self.archive = self.root / 'vork-test-windows-x64.zip'
        binary = os.environ.get('VORK_TEST_BINARY')
        self.fixture('vork.exe', Path(binary).read_bytes() if binary else b'not an executable')

    def tearDown(self):
        self.tmp.cleanup()

    def fixture(self, member, binary=b'not an executable'):
        with zipfile.ZipFile(self.archive, 'w') as z:
            z.writestr(member, binary)
            z.writestr('examples/scene.json', '{}')
            z.writestr('NOTICE', 'notices')
            for name, variable in [('slang', 'VORK_TEST_SLANG'), ('oidn', 'VORK_TEST_OIDN')]:
                if os.environ.get(variable):
                    tool = Path(os.environ[variable])
                    for path in tool.rglob('*'):
                        if path.is_file():
                            z.write(path, name + '/' + path.relative_to(tool).as_posix())
        (self.root / 'SHA256SUMS').write_text(hashlib.sha256(self.archive.read_bytes()).hexdigest() +
                                            '  ' + self.archive.name + '\n')

    def run_installer(self, *args):
        shell = shutil.which('pwsh') or 'powershell'
        return subprocess.run([shell, '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', str(PS_INSTALLER), *args],
                              cwd=self.root, env=self.env, capture_output=True, text=True)

    @unittest.skipUnless(all(os.environ.get(k) for k in ('VORK_TEST_BINARY', 'VORK_TEST_SLANG', 'VORK_TEST_OIDN')), 'needs vork.exe, pinned toolchains and GPU')
    def test_global_install_and_setup(self):
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertTrue((self.root / 'prefix/NOTICE').exists())
        self.assertTrue((self.root / 'prefix/vork.exe').exists())

    def test_corruption_fails_before_install(self):
        self.archive.write_bytes(self.archive.read_bytes() + b'corruption')
        self.assertNotEqual(self.run_installer().returncode, 0)
        self.assertFalse((self.root / 'prefix/vork.exe').exists())

    def test_missing_checksum_fails(self):
        (self.root / 'SHA256SUMS').write_text('')
        self.assertNotEqual(self.run_installer().returncode, 0)

    def test_duplicate_checksum_fails(self):
        sums = self.root / 'SHA256SUMS'; sums.write_text(sums.read_text() * 2)
        self.assertNotEqual(self.run_installer().returncode, 0)

    def test_traversal_fails(self):
        self.fixture('../vork.exe')
        self.assertNotEqual(self.run_installer().returncode, 0)
        self.assertFalse((self.root / 'prefix/vork.exe').exists())

    def test_dry_run_downloads_and_writes_nothing(self):
        self.env['VORK_DOWNLOAD_BASE'] = 'https://github.com/voyager-so/vork/releases/download/vtest'
        result = self.run_installer('-DryRun')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('SHA256SUMS', result.stdout)
        self.assertFalse((self.root / 'prefix').exists())

if __name__ == '__main__':
    unittest.main()
