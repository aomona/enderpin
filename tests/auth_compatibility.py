"""Opt-in official-JAR adapter and Knot class-loading smoke; synthetic IPC, no account.

Build first, then run with --java-home /path/to/jdk17 --java-home /path/to/jdk21 --java-home /path/to/jdk25
Downloads hash-verified vendor artifacts into target/auth-compat (reuses the Enderpin cache).
This tests raw vendor classes through Knot, not Minecraft initialization, UI, or real login.
"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import socket
import struct
import sys
import subprocess
import threading
import urllib.request

REPO = Path(__file__).resolve().parents[1]
ROOT = REPO / 'target/auth-compat'
CACHE = Path.home() / 'Library/Caches/enderpin/sha512/runtime'


def fetch(url):
    with urllib.request.urlopen(url, timeout=90) as response:
        return response.read()


def acquire(url, digest, algorithm, filename):
    destination = ROOT / 'downloads' / digest / filename
    candidates = [destination, ROOT / filename, *(CACHE / f'{algorithm}-{digest}').glob('*')]
    for candidate in candidates:
        if candidate.is_file() and hashlib.new(algorithm, candidate.read_bytes()).hexdigest() == digest:
            return candidate
    data = fetch(url)
    assert hashlib.new(algorithm, data).hexdigest() == digest, f'vendor hash mismatch: {filename}'
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_bytes(data)
    return destination


def official(download, filename):
    return acquire(download['url'], download['sha1'], 'sha1', filename)


def fixture(channel, errors, commands):
    def read(size):
        data = b''
        while len(data) < size:
            chunk = channel.recv(size - len(data))
            if not chunk:
                raise EOFError()
            data += chunk
        return data
    try:
        while True:
            size = struct.unpack('>I', read(4))[0]
            assert 0 < size <= 262144
            request = json.loads(read(size)); command = request['command']; kind = command['type']
            commands.append(kind)
            if command == {'type': 'join', 'server_hash': 'dead'}:
                payload = json.dumps({'id': request['id'], 'error': 'unauthorized'}).encode()
                channel.sendall(struct.pack('>I', len(payload)) + payload)
                continue
            if kind == 'hello':
                result = {'protocol': 1}
            elif kind == 'join':
                assert command == {'type': 'join', 'server_hash': '123abc'}
                result = None
            elif kind == 'properties':
                result = {
                    'privileges': {k: {'enabled': False} for k in ['onlineChat', 'multiplayerServer', 'multiplayerRealms', 'telemetry']},
                    'profanityFilterPreferences': {'profanityFilterOn': True},
                    'banStatus': {'bannedScopes': {}},
                    'friendsPreferences': {'friends': 'DISABLED', 'acceptInvites': 'DISABLED'},
                    'chatPreferences': {'textCommunication': 'DISABLED'},
                }
            elif kind == 'block_list':
                result = {'blockedProfiles': []}
            elif kind == 'certificate':
                result = {'keyPair': {'privateKey': 'MONALAUNCHER_REMOTE_CHAT_KEY:' + 'f' * 64}}
            elif kind == 'sign':
                assert command == {'type': 'sign', 'key_id': 'f' * 64, 'message': 'AQID'}
                result = {'signature': base64.b64encode(bytes([2]) * 256).decode()}
            else:
                raise AssertionError('unexpected IPC operation: ' + kind)
            payload = json.dumps({'id': request['id'], 'result': result}).encode()
            channel.sendall(struct.pack('>I', len(payload)) + payload)
    except EOFError:
        pass
    except Exception as error:
        errors.append(type(error).__name__)
    finally:
        channel.close()


def run(java, bridge, version, jars, names, loader):
    parent, child = socket.socketpair()
    errors, commands = [], []
    worker = threading.Thread(target=fixture, args=(parent, errors, commands), daemon=True)
    worker.start()
    fd = child.fileno()
    command = [str(java), '-XX:-UsePerfData', f'-Denderpin.minecraft={version}',
               f'-javaagent:{bridge / "auth-bridge.jar"}={bridge / "libauth-bridge.dylib" if os.uname().sysname == "Darwin" else bridge / "libauth-bridge.so"}',
               '-cp', os.pathsep.join(map(str, [ROOT / 'classes', *jars])),
               'me.aomona.smoke.LoaderSmoke' if loader != 'vanilla' else 'me.aomona.smoke.VendorSmoke', *names]
    try:
        trampoline = [sys.executable, "-c",
                      "import os,sys; os.dup2(int(sys.argv[1]),3); os.execv(sys.argv[2],sys.argv[2:])", str(fd)]
        result = subprocess.run(trampoline + command, capture_output=True, text=True, timeout=60,
                                pass_fds=(fd,), cwd=ROOT)
    finally:
        child.close()
    worker.join(timeout=5)
    log = ROOT / f'{version}-{loader}.log'
    log.write_text(result.stdout + result.stderr)
    assert result.returncode == 0 and not errors, f'{version}/{loader} failed: see {log}'
    expected = ['hello', 'join', *(['join'] if version == '1.20.1' else []), 'properties', 'block_list', 'certificate', 'sign']
    assert commands == expected, commands
    assert 'VENDOR_JOIN_ATTRIBUTES_CERTIFICATE_REMOTE_KEY_OK' in result.stdout
    if loader != 'vanilla':
        assert 'KNOT_VENDOR_AUTH_OK' in result.stdout
    return {'minecraft': version, 'loader': loader, 'vendor_adapter': True,
            'synthetic_ipc': True, 'knot_class_loading': loader != 'vanilla', 'real_account': False}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--java-home', type=Path, action='append', required=True, help='Repeat for each required Java major')
    parser.add_argument('--minecraft', action='append', help='Limit the bundled matrix to these versions')
    args = parser.parse_args()
    if os.name != 'posix':
        parser.error('this socketpair smoke requires POSIX; Windows JNI is covered separately')
    ROOT.mkdir(parents=True, exist_ok=True)
    homes = {}
    for home in args.java_home:
        release = dict(line.split('=', 1) for line in (home / 'release').read_text().splitlines() if '=' in line)
        major = int(release['JAVA_VERSION'].strip('"').split('.')[0])
        homes[major] = home.resolve()
    compiler = homes[max(homes)] / 'bin/javac'
    bridge = max((REPO / 'target/debug/build').glob('enderpin-*/out/auth-bridge.jar'), key=lambda p: p.stat().st_mtime).parent
    subprocess.run([str(compiler), '--release', '17', '-d', str(ROOT / 'classes'),
                    str(REPO / 'java/auth-bridge-smoke/LoaderSmoke.java'), str(REPO / 'java/auth-bridge-smoke/VendorSmoke.java'),
                    str(REPO / 'java/auth-bridge-smoke/me/aomona/auth/AdapterSmoke.java'),
                    str(REPO / 'java/auth-bridge/me/aomona/auth/MethodAdapter.java')], check=True)
    manifest = json.loads(fetch('https://piston-meta.mojang.com/mc/game/version_manifest_v2.json'))
    entries = {row['id']: row for row in manifest['versions']}
    results = []
    for line in (REPO / 'java/auth-bridge/auth-adapters.properties').read_text().splitlines():
        if line.startswith('#') or not line:
            continue
        version, value = line.split('='); fields = value.split('|')
        if args.minecraft and version not in args.minecraft:
            continue
        meta = entries[version]
        metadata = json.loads(official(meta, version + '.json').read_text())
        assert metadata['javaVersion']['majorVersion'] == int(fields[0])
        java = homes[int(fields[0])] / 'bin/java'
        jars = [official(metadata['downloads']['client'], version + '.jar')]
        identities = {}
        authlib = tinyfd = None
        for lib in metadata['libraries']:
            if ':natives-' in lib['name'] or 'artifact' not in lib.get('downloads', {}):
                continue
            artifact = lib['downloads']['artifact']
            jar = official(artifact, Path(artifact['path']).name)
            jars.append(jar)
            identities[jar] = ':'.join(lib['name'].split(':')[:2])
            if lib['name'].startswith('org.lwjgl:lwjgl-tinyfd:'):
                tinyfd = jar
            if lib['name'].startswith('com.mojang:authlib:'):
                authlib = jar
        assert authlib and authlib.name == fields[1]
        assert hashlib.sha256(authlib.read_bytes()).hexdigest() == fields[2]
        probe = [str(java), '-cp', str(ROOT / 'classes'), 'me.aomona.auth.AdapterSmoke',
                 'auth', str(authlib), 'com/mojang/authlib/minecraft/client/MinecraftClient']
        if fields[1] == 'authlib-4.0.43.jar':
            probe += ['legacy-session', str(authlib), 'com/mojang/authlib/yggdrasil/YggdrasilMinecraftSessionService']
        for mode, name in zip(['crypt', 'desktop', 'realms', 'drop'], fields[4:]):
            probe += [mode, str(jars[0]), name]
        assert tinyfd is not None or version == '26.3'
        file_dialog = ['org/lwjgl/util/tinyfd/TinyFileDialogs'] if tinyfd else []
        if tinyfd:
            probe += ['file', str(tinyfd), *file_dialog]
        subprocess.run(probe, check=True, capture_output=True, text=True)
        for loader in ['vanilla', '0.19.5', '0.20.1']:
            extra = []
            inherited = set()
            if loader != 'vanilla':
                profile = json.loads(fetch(f'https://meta.fabricmc.net/v2/versions/loader/{version}/{loader}/profile/json'))
                for lib in profile['libraries']:
                    group, artifact, release = lib['name'].split(':')
                    inherited.add(group + ':' + artifact)
                    filename = f'{artifact}-{release}.jar'
                    url = lib['url'] + group.replace('.', '/') + f'/{artifact}/{release}/{filename}'
                    digest = lib.get('sha512') or fetch(url + '.sha512').decode().split()[0]
                    extra.append(acquire(url, digest, 'sha512', filename))
            # Launcher inheritance replaces a library by Maven group/artifact, not version.
            effective = [jar for jar in jars if identities.get(jar) not in inherited]
            row = run(java, bridge, version, [*extra, *effective], [*fields[4:], *file_dialog], loader)
            row['java_major'] = int(fields[0])
            results.append(row)
            print(json.dumps(row), flush=True)
            (ROOT / 'smoke-results.json').write_text(json.dumps(results, indent=2) + '\n')


if __name__ == '__main__':
    main()
