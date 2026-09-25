"""Offline permissions editor regression test (macOS/Linux, Python 3.11+).

python3 -B tests/permissions_tui.py --binary target/debug/enderpin
Uses disposable workspaces; does not download or launch Minecraft.
"""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile
import tomllib

from live_setup import Terminal


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path('target/debug/enderpin'))
    binary = parser.parse_args().binary.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix='enderpin-permissions-') as directory:
        root = Path(directory)
        subprocess.run([str(binary), 'init', '--minecraft', '1.21.1'], cwd=root, check=True, capture_output=True)
        original = (root / 'enderpin.toml').read_bytes()
        local = root / '.enderpin/client/permissions.toml'
        for cancel in [True, False]:
            terminal = Terminal(binary, root, ['permissions'])
            try:
                terminal.expect('Editing client permissions.')
                terminal.expect('Game permissions')
                terminal.send('\r')  # Game permissions is the initial selection.
                terminal.expect('Allowed access')
                terminal.send(' \r')  # Deny network; keep other defaults.
                terminal.expect('Save and review')
                if cancel:
                    terminal.send('\x1b')
                    terminal.expect('no changes saved')
                else:
                    terminal.send('\x1b[A\r')  # Save and review.
                    terminal.expect('Network (internet, LAN, listening): denied')
                    terminal.expect('Approve this game and these packages on this PC?')
                    terminal.send('y\r')
                    terminal.expect('Permissions approved')
                terminal.finish()
            finally:
                terminal.close()
            if cancel:
                assert (root / 'enderpin.toml').read_bytes() == original
                assert not local.exists()
        manifest = tomllib.loads((root / 'enderpin.toml').read_text())
        assert manifest['client']['sandbox'] == {'network': False}, manifest
        assert 'sandbox' not in manifest['server']
        plan = json.loads(subprocess.check_output([str(binary), 'permissions', 'show', '--json'], cwd=root))
        assert plan['approved']
        assert plan['plan']['effective']['network'] is False
        assert plan['plan']['effective']['account_authentication'] is True
        # Add a direct host grant from the TUI; declining review keeps it unapproved.
        with tempfile.TemporaryDirectory(prefix='enderpin-external-folder-') as external:
            folder = Path(external).resolve()
            terminal = Terminal(binary, root, ['permissions'])
            try:
                terminal.expect('Game permissions')
                terminal.send('\x1b[B' * 2 + '\r')
                terminal.expect('Add external folder')
                terminal.send('\r')
                terminal.expect('Existing folder path')
                terminal.send(str(folder) + '\r')
                terminal.expect('Folder access')
                terminal.send('\x1b[B\r')
                terminal.expect('Save and review')
                terminal.send('\x1b[A\r')
                terminal.expect(str(folder) + ' — read/write')
                terminal.expect('Approve this game and these packages on this PC?')
                terminal.send('n\r')
                terminal.expect('permissions were not approved')
                terminal.finish(success=False)
            finally:
                terminal.close()
            saved = tomllib.loads(local.read_text())
            assert saved['folders'] == [{'path': str(folder), 'write': True}]
            assert 'approved' not in saved
        # A server editor offers no desktop controls, and saving alone never approves.
        terminal = Terminal(binary, root, ['--target', 'server', 'permissions'])
        try:
            terminal.expect('Game permissions')
            terminal.send('\x1b[B' * 3 + '\r')  # Save without approving.
            terminal.expect('Permissions saved; approval is pending.')
            terminal.finish()
            assert 'Advanced desktop permissions' not in terminal.transcript
        finally:
            terminal.close()
        assert 'approved' not in tomllib.loads((root / '.enderpin/server/permissions.toml').read_text())
        # A direct review uses the same summary and defaults to declining approval.
        terminal = Terminal(binary, root, ['--target', 'server', 'permissions', 'approve'])
        try:
            terminal.expect('Approve this game and these packages on this PC?')
            terminal.send('\r')
            terminal.expect('permissions were not approved')
            terminal.finish(success=False)
        finally:
            terminal.close()
        assert 'approved' not in tomllib.loads((root / '.enderpin/server/permissions.toml').read_text())
    print(json.dumps({'cancel_without_writes': True, 'compact_settings': True,
                      'hash_free_approval': True, 'server_controls': True,
                      'save_without_approval': True, 'default_decline': True, 'external_folder_editor': True}))


if __name__ == '__main__':
    main()
