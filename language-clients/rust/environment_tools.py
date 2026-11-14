"""Plugin-private, pinned Linux language-tool preparation. No host imports.

Executed with isolated Python (-I); stdout and stdin belong exclusively to the
language server. Package-manager lifecycle scripts are never executed.
"""
import base64
import gzip
import hashlib
import json
import os
import pathlib
import platform
import re
import shutil
import stat
import tarfile
import tempfile
import time
import urllib.parse
import urllib.request

MAX_DOWNLOAD = 64 * 1024 * 1024
MAX_DOWNLOADS = 128 * 1024 * 1024
MAX_EXPANDED = 512 * 1024 * 1024
MAX_FILES = 30000
MAX_CACHE_BYTES = 1024 * 1024 * 1024
MAX_CACHE_ENTRIES = 4
BLOCK = 1024 * 1024


def relative(value):
    path = pathlib.PurePosixPath(value)
    if not value or path.is_absolute() or '..' in path.parts or '\\' in value or '\0' in value:
        raise ValueError('Invalid tool archive path')
    return path


def contained(directory, name):
    path = os.path.join(directory, *relative(name).parts)
    if os.path.commonpath([os.path.realpath(path), os.path.realpath(directory)]) != os.path.realpath(directory):
        raise ValueError('Tool archive path leaves its directory')
    return path


def digest(path):
    value = hashlib.sha256()
    with open(path, 'rb') as source:
        while True:
            data = source.read(BLOCK)
            if not data:
                return value.hexdigest()
            value.update(data)


class Budget:
    def __init__(self, root):
        self.root, self.downloaded, self.expanded = root, 0, 0
        self.files = {}

    def copy(self, source, name, executable=False):
        if len(self.files) >= MAX_FILES or name in self.files:
            raise ValueError('Duplicate tool file or file budget exceeded')
        target = contained(self.root, name)
        os.makedirs(os.path.dirname(target), mode=0o700, exist_ok=True)
        checksum, size = hashlib.sha256(), 0
        with open(target, 'xb') as output:
            while True:
                data = source.read(BLOCK)
                if not data:
                    break
                size += len(data)
                self.expanded += len(data)
                if self.expanded > MAX_EXPANDED:
                    raise ValueError('Expanded language tools exceed their budget')
                output.write(data)
                checksum.update(data)
        os.chmod(target, 0o700 if executable else 0o600)
        self.files[name] = {'sha256': checksum.hexdigest(), 'size': size, 'executable': executable}


def https(url):
    parsed = urllib.parse.urlsplit(url)
    if parsed.scheme != 'https' or parsed.username or parsed.password:
        raise ValueError('Language tools require HTTPS sources')
    return url


class Redirects(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, new_url):
        return super().redirect_request(request, fp, code, message, headers, https(new_url))


def download(spec, target, budget):
    if 'sha256' in spec:
        algorithm, expected = 'sha256', bytes.fromhex(spec['sha256'])
    else:
        algorithm, encoded = spec['integrity'].split('-', 1)
        if algorithm != 'sha512':
            raise ValueError('Unsupported package integrity algorithm')
        expected = base64.b64decode(encoded, validate=True)
    checksum, size = hashlib.new(algorithm), 0
    if len(expected) != checksum.digest_size:
        raise ValueError('Invalid pinned tool digest')
    opener = urllib.request.build_opener(Redirects())
    with opener.open(https(spec['url']), timeout=45) as response, open(target, 'xb') as output:
        while True:
            data = response.read(BLOCK)
            if not data:
                break
            size += len(data)
            budget.downloaded += len(data)
            if size > MAX_DOWNLOAD or budget.downloaded > MAX_DOWNLOADS:
                raise ValueError('Language tool download budget exceeded')
            output.write(data)
            checksum.update(data)
    if checksum.digest() != expected:
        raise ValueError('Language tool integrity mismatch')


def unpack(archive, budget, destination='', member=None, executable=False):
    found = False
    with tarfile.open(archive, 'r:*') as source:
        for item in source:
            if member is not None and item.name != member:
                continue
            parts = relative(item.name).parts
            if member is None and (not parts or parts[0] != 'package'):
                raise ValueError('Unexpected package archive root')
            if item.isdir():
                continue
            if not item.isfile() or item.size > MAX_EXPANDED:
                raise ValueError('Tool archives may only contain regular files and directories')
            name = destination if member is not None else str(pathlib.PurePosixPath(destination, *parts[1:]))
            with source.extractfile(item) as data:
                budget.copy(data, name, executable)
            found = True
    if not found:
        raise ValueError('Tool archive lacks its expected files')


def valid_cache(directory, key, required=()):
    try:
        if os.path.islink(directory):
            return False
        with open(os.path.join(directory, '.loog-install.json'), encoding='utf-8') as source:
            record = json.loads(source.read(8 * 1024 * 1024 + 1))
        files = record['files']
        if record['key'] != key or not 0 < len(files) <= MAX_FILES or any(name not in files for name in required):
            return False
        total = 0
        for name, expected in files.items():
            path = contained(directory, name)
            info = os.lstat(path)
            total += info.st_size
            if not stat.S_ISREG(info.st_mode) or info.st_size != expected['size'] or total > MAX_EXPANDED:
                return False
            if expected['executable'] and not info.st_mode & stat.S_IXUSR:
                return False
            if digest(path) != expected['sha256']:
                return False
        return True
    except (OSError, ValueError, KeyError, TypeError):
        return False


def reserve_cache(parent):
    # Count old versions and interrupted stages without deleting any existing
    # directory. A hard-killed install consumes one of these bounded slots.
    pending = []
    with os.scandir(parent) as entries:
        for entry in entries:
            if entry.name != '.install.lock':
                pending.append(entry.path)
            if len(pending) >= MAX_CACHE_ENTRIES:
                raise ValueError('Language tool cache has too many versions or interrupted installs; choose a fresh cache directory')
    total, files = 0, 0
    while pending:
        path = pending.pop()
        info = os.lstat(path)
        files += 1
        if files > MAX_CACHE_ENTRIES * (MAX_FILES + 1) * 2:
            raise ValueError('Language tool cache entry budget exceeded')
        if stat.S_ISDIR(info.st_mode):
            with os.scandir(path) as children:
                for entry in children:
                    pending.append(entry.path)
                    if len(pending) > MAX_FILES * 2:
                        raise ValueError('Language tool cache entry budget exceeded')
        elif stat.S_ISREG(info.st_mode):
            total += info.st_size
        else:
            raise ValueError('Language tool cache contains an unexpected file type')
        if total + MAX_EXPANDED + MAX_DOWNLOAD > MAX_CACHE_BYTES:
            raise ValueError('Language tool cache size budget exceeded; choose a fresh cache directory')
    if shutil.disk_usage(parent).free < MAX_EXPANDED + MAX_DOWNLOAD:
        raise ValueError('Insufficient disk space for bounded language tool preparation')


def prepare(spec, cache='', system=None, machine=None):
    system, machine = system or platform.system(), machine or platform.machine()
    arch = {'x86_64': 'x86_64', 'amd64': 'x86_64', 'aarch64': 'aarch64', 'arm64': 'aarch64'}.get(machine.lower())
    if system != 'Linux' or arch is None:
        raise ValueError('Managed language tools currently require Linux x86_64 or aarch64')
    import fcntl
    name = spec['id']
    if not re.fullmatch(r'[a-z0-9]+(?:[.-][a-z0-9]+)+', name):
        raise ValueError('Invalid language tool package identity')
    target = 'linux-' + arch
    pinned = spec['platforms'][target]
    if spec['kind'] not in ('native', 'node'):
        raise ValueError('Unsupported managed tool recipe')
    encoded = json.dumps(spec, sort_keys=True, separators=(',', ':')).encode()
    key = target + '-' + hashlib.sha256(encoded).hexdigest()
    base = cache or os.environ.get('XDG_CACHE_HOME') or os.path.join(os.path.expanduser('~'), '.cache')
    if not os.path.isabs(base):
        raise ValueError('Managed tool cache must be an absolute target path')
    parent = os.path.join(os.path.abspath(base), 'loog-language-tools', name)
    os.makedirs(parent, mode=0o700, exist_ok=True)
    info = os.lstat(parent)
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.geteuid() or info.st_mode & 0o022:
        raise ValueError('Managed tool cache must be a private writable directory')
    final = os.path.join(parent, key)
    lock = os.open(os.path.join(parent, '.install.lock'), os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    try:
        info = os.fstat(lock)
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid() or info.st_mode & 0o022:
            raise ValueError('Language tool installation lock must be a private regular file')
        deadline = time.monotonic() + 180
        while True:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except BlockingIOError:
                if time.monotonic() >= deadline:
                    raise TimeoutError('Another language tool installation is still running')
                time.sleep(0.05)
        if os.path.lexists(final):
            required = ['server'] if spec['kind'] == 'native' else ['runtime', 'NODE-LICENSE', spec['entry']]
            if valid_cache(final, key, required):
                return final
            raise ValueError('Managed language cache failed integrity checks; choose a fresh cache directory')
        reserve_cache(parent)
        # This directory is created exclusively by this invocation. No existing
        # cache directory or caller-selected path is recursively removed.
        stage = tempfile.mkdtemp(prefix=key + '.partial-', dir=parent)
        try:
            budget = Budget(stage)
            archive = os.path.join(stage, '.download')
            download(pinned, archive, budget)
            if spec['kind'] == 'native':
                if pinned['format'] != 'gz':
                    raise ValueError('Unsupported native tool archive')
                with gzip.open(archive, 'rb') as data:
                    budget.copy(data, 'server', True)
            else:
                unpack(archive, budget, 'runtime', pinned['member'], True)
                unpack(archive, budget, 'NODE-LICENSE', pinned['license_member'])
            os.unlink(archive)
            packages = spec.get('packages', [])
            if len(packages) > 128:
                raise ValueError('Too many managed server packages')
            for package in packages:
                if not relative(package['path']).parts[0] == 'node_modules':
                    raise ValueError('Invalid managed server package path')
                download(package, archive, budget)
                unpack(archive, budget, package['path'])
                os.unlink(archive)
            for filename, license in spec.get('licenses', {}).items():
                if len(relative(filename).parts) != 1:
                    raise ValueError('Invalid license path')
                download(license, archive, budget)
                with open(archive, 'rb') as data:
                    budget.copy(data, filename)
                os.unlink(archive)
            with open(os.path.join(stage, '.loog-install.json'), 'x', encoding='utf-8') as output:
                json.dump({'key': key, 'files': budget.files}, output, sort_keys=True)
            os.rename(stage, final)
            stage = None
            return final
        finally:
            if stage is not None and os.path.dirname(os.path.realpath(stage)) == os.path.realpath(parent):
                shutil.rmtree(stage)
    finally:
        os.close(lock)


def run(options):
    if platform.system() != 'Linux':
        raise ValueError('Managed language tools currently require Linux')
    import signal
    signal.signal(signal.SIGALRM, lambda *_: (_ for _ in ()).throw(TimeoutError('Language tool preparation timed out')))
    signal.signal(signal.SIGTERM, lambda *_: (_ for _ in ()).throw(InterruptedError('Language tool preparation cancelled')))
    signal.alarm(270)
    spec = options['spec']
    directory = prepare(spec, options.get('cache', ''))
    signal.alarm(0)
    signal.signal(signal.SIGTERM, signal.SIG_DFL)
    if spec['kind'] == 'native':
        program, args = os.path.join(directory, 'server'), options['args']
    else:
        program = os.path.join(directory, 'runtime')
        args = [contained(directory, spec['entry'])] + options['args']
    os.execv(program, [program] + args)


if __name__ == '__main__':
    try:
        run(OPTIONS)
    except Exception as error:
        import sys
        print('loog-tool-error: ' + str(error).replace('\n', ' ')[:800], file=sys.stderr)
        sys.exit(78)
