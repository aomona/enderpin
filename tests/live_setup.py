"""Opt-in real TUI setup smoke test (macOS/Linux, Python 3.11+).

python3 tests/live_setup.py --binary target/release/enderpin
Uses disposable directories and the normal download cache. Never launches Minecraft.
"""
import argparse
import errno
import fcntl
import json
import os
from pathlib import Path
import pty
import re
import select
import signal
import struct
import subprocess
import tempfile
import termios
import time
import tomllib


ANSI = re.compile(r'\x1b\[[0-?]*[ -/]*[@-~]')


class Terminal:
    def __init__(self, binary, root, args):
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.chdir(root)
            os.environ['TERM'] = 'xterm-256color'
            fcntl.ioctl(1, termios.TIOCSWINSZ, struct.pack('HHHH', 40, 120, 0, 0))
            os.execv(str(binary), [str(binary), *args])
        self.buffer = ''
        self.transcript = ''
        self.reaped = False

    def expect(self, text, timeout=180):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if text in self.buffer:
                self.buffer = self.buffer.split(text, 1)[1]
                return
            if select.select([self.fd], [], [], .2)[0]:
                try:
                    data = os.read(self.fd, 65536)
                except OSError as error:
                    if error.errno != errno.EIO:
                        raise
                    data = b''
                assert data, 'terminal exited before ' + text + '\n' + self.transcript[-6000:]
                decoded = ANSI.sub('', data.decode(errors='replace'))
                self.buffer += decoded
                self.transcript += decoded
        raise AssertionError('timeout waiting for ' + text + '\n' + self.transcript[-6000:])

    def send(self, text):
        os.write(self.fd, text.encode())

    def finish(self, success=True):
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            pid, status = os.waitpid(self.pid, os.WNOHANG)
            if pid:
                self.reaped = True
                assert (os.waitstatus_to_exitcode(status) == 0) == success, self.transcript[-6000:]
                return
            time.sleep(.1)
        raise AssertionError('setup did not exit')

    def close(self):
        if not self.reaped:
            os.kill(self.pid, signal.SIGKILL)
            os.waitpid(self.pid, 0)
        os.close(self.fd)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path('target/release/enderpin'))
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix='enderpin-setup-tui-') as directory:
        root = Path(directory)
        terminal = Terminal(binary, root, ['quick'])
        try:
            terminal.expect('Minecraft version')
            terminal.send('1.21.1 (release)\r')
            terminal.expect('Mod loader')
            terminal.send('fabric\r')
            terminal.expect('Search Modrinth mods')
            terminal.send('lithium\r')
            terminal.expect('Mods — select to add/remove')
            terminal.send('lithium\r')
            terminal.expect('[x]')
            terminal.send('lithium\r')
            terminal.expect('[ ] Lithium')
            terminal.send('lithium\r')
            terminal.expect('[x]')
            terminal.send('Finish selection\r')
            terminal.expect('Create workspace and download files?')
            terminal.send('y\r')
            terminal.expect('Setup complete.', timeout=600)
            terminal.finish()
        finally:
            terminal.close()
        manifest = tomllib.loads((root / 'enderpin.toml').read_text())
        lock = tomllib.loads((root / 'enderpin.lock').read_text())
        assert manifest['minecraft'] == '1.21.1', manifest['minecraft']
        assert manifest['client']['loader'] == 'fabric'
        assert 'lithium' in manifest['client']['packages']
        assert set(lock['runtimes']) == {'client'}
        assert any(package['name'] == 'lithium' for package in lock['targets']['client']['packages'])
        assert list((root / 'client/mods').glob('*.jar'))
        assert not (root / 'client/logs/latest.log').exists(), 'setup launched Minecraft'
        assert not (root / '.enderpin/client/permissions.toml').exists(), 'setup auto-approved permissions'
        assert not (root / 'server/eula.txt').exists()
    with tempfile.TemporaryDirectory(prefix='enderpin-setup-both-') as directory:
        root = Path(directory)
        result = subprocess.run([str(binary), 'quick', '--server', '--no-interactive',
            '--version', '1.21.1', '--loader', 'fabric', '--mods', 'lithium,sodium'],
            cwd=root, capture_output=True, text=True, timeout=600)
        assert result.returncode == 0, result.stderr
        lock = tomllib.loads((root / 'enderpin.lock').read_text())
        assert set(lock['runtimes']) == {'client', 'server'}
        client_mods = {p['name'] for p in lock['targets']['client']['packages']}
        server_mods = {p['name'] for p in lock['targets']['server']['packages']}
        assert {'lithium', 'sodium'} <= client_mods
        assert 'lithium' in server_mods and 'sodium' not in server_mods
        assert not (root / 'client/logs/latest.log').exists()
        assert not (root / 'server/eula.txt').exists()
    with tempfile.TemporaryDirectory(prefix='enderpin-setup-cancel-') as directory:
        root = Path(directory)
        terminal = Terminal(binary, root, ['quick', '--version', '26.2'])
        try:
            terminal.expect('Mod loader')
            terminal.send('\x1b')
            terminal.expect('setup cancelled')
            terminal.finish(success=False)
        finally:
            terminal.close()
        assert not list(root.iterdir()), 'cancelled setup wrote files'
    print(json.dumps({'version_search': True, 'loader_search': True, 'mod_search_and_selection': True,
                      'download_and_lock': True, 'both_targets_and_mod_sides': True,
                      'cancel_without_writes': True, 'game_launched': False}, indent=2))


if __name__ == '__main__':
    main()
