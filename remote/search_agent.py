"""Private Linux search deployment and bounded worker pipes, owned by SSH."""
import base64
import fcntl
import hashlib
import json
import os
import signal
import subprocess
import time
import zlib
from collections import deque


class WorkerBundle:
    def __init__(self, size, digest):
        if not isinstance(size, int) or not 0 < size <= 32 * 1024 * 1024:
            raise ValueError('Search worker exceeds its size limit')
        if not isinstance(digest, str) or len(digest) != 64:
            raise ValueError('Invalid search worker digest')
        self.size, self.digest = size, digest
        self.fd = os.memfd_create('loog-search', os.MFD_CLOEXEC | os.MFD_ALLOW_SEALING)
        self.decompress = zlib.decompressobj(16 + zlib.MAX_WBITS)
        self.hash, self.written, self.compressed = hashlib.sha256(), 0, 0
        self.ready = False

    def close(self):
        if self.fd >= 0:
            os.close(self.fd)
            self.fd = -1

    def append(self, offset, encoded):
        if self.ready or offset != self.compressed:
            raise ValueError('Invalid search worker upload offset')
        data = base64.b64decode(encoded, validate=True)
        if not 0 < len(data) <= 128 * 1024 or self.compressed + len(data) > 8 * 1024 * 1024:
            raise ValueError('Search worker upload exceeds budget')
        decoded = self.decompress.decompress(data, self.size - self.written + 1)
        if self.written + len(decoded) > self.size or self.decompress.unconsumed_tail:
            raise ValueError('Expanded search worker exceeds budget')
        self.hash.update(decoded)
        self.written += len(decoded)
        self.compressed += len(data)
        view = memoryview(decoded)
        while view:
            view = view[os.write(self.fd, view):]

    def finish(self):
        if self.written != self.size or not self.decompress.eof or self.decompress.unused_data:
            raise ValueError('Incomplete search worker upload')
        if self.hash.hexdigest() != self.digest:
            raise ValueError('Search worker integrity mismatch')
        os.fchmod(self.fd, 0o500)
        fcntl.fcntl(self.fd, fcntl.F_ADD_SEALS, fcntl.F_SEAL_WRITE | fcntl.F_SEAL_GROW
                    | fcntl.F_SEAL_SHRINK | fcntl.F_SEAL_SEAL)
        self.ready = True


class SearchWorker:
    def __init__(self, bundle, root, directory, path, options):
        if not bundle or not bundle.ready:
            raise ValueError('Search worker is unavailable')
        data = json.dumps(options, separators=(',', ':')).encode()
        if len(data) > 256 * 1024:
            raise ValueError('Search request exceeds budget')
        self.root, self.path = root, path
        self.child = subprocess.Popen(['/proc/self/fd/' + str(bundle.fd), str(directory), path],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            pass_fds=(bundle.fd, directory), start_new_session=True,
            env={'PATH': '/usr/bin:/bin', 'LANG': 'C.UTF-8'})
        for stream in [self.child.stdin, self.child.stdout, self.child.stderr]:
            os.set_blocking(stream.fileno(), False)
        self.input, self.pending, self.stderr = data, bytearray(), bytearray()
        self.pages, self.queued = deque(), 0
        self.progress = dict(searched=0, skipped_binary=0, skipped_encoding=0,
                             skipped_large=0, errors=0, first_error=None, limit_hit=False)
        self.complete, self.eof = False, False
        self.started, self.touched = time.monotonic(), time.monotonic()

    def close(self):
        if self.child.poll() is None:
            try:
                os.killpg(self.child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        # WNOHANG avoids holding the Agent loop while a killed process exits.
        self.child.poll()
        for stream in [self.child.stdin, self.child.stdout, self.child.stderr]:
            stream.close()
        return self.child if self.child.poll() is None else None

    def pump(self):
        if not self.child.stdin.closed:
            try:
                count = os.write(self.child.stdin.fileno(), self.input)
                self.input = self.input[count:]
                if not self.input:
                    self.child.stdin.close()
            except BlockingIOError:
                pass
            except BrokenPipeError:
                self.child.stdin.close()
        if self.queued < 256 * 1024 and not self.eof:
            try:
                chunk = os.read(self.child.stdout.fileno(), 65536)
                if not chunk:
                    self.eof = True
                self.pending.extend(chunk)
            except BlockingIOError:
                pass
            while b'\n' in self.pending:
                line, _, rest = self.pending.partition(b'\n')
                self.pending = bytearray(rest)
                if len(line) > 256 * 1024:
                    raise ValueError('Search worker publication exceeds budget')
                page = json.loads(line)
                # Account for ASCII escaping in the outer Agent protocol frame.
                size = len(json.dumps(page, ensure_ascii=True, separators=(',', ':')).encode())
                if size > 768 * 1024:
                    raise ValueError('Search publication exceeds transport budget')
                self.pages.append((page, size))
                self.queued += size
            if len(self.pending) > 256 * 1024:
                raise ValueError('Search worker frame exceeds budget')
        try:
            error = os.read(self.child.stderr.fileno(), 4096)
            self.stderr.extend(error[:max(0, 4096 - len(self.stderr))])
        except BlockingIOError:
            pass
        if time.monotonic() - self.started > 130:
            raise ValueError('Search exceeded its time budget')
        if self.child.poll() is not None and self.eof and not self.pages and not self.complete:
            raise ValueError(self.stderr.decode('utf-8', errors='replace') or 'Search worker stopped before completion')

    def next(self, limit):
        if not isinstance(limit, int) or not 1 <= limit <= 32:
            raise ValueError('Invalid search page size')
        self.touched = time.monotonic()
        self.pump()
        files, used = [], 0
        while self.pages and len(files) < limit:
            page, size = self.pages[0]
            if used and used + size > 768 * 1024:
                break
            self.pages.popleft()
            self.queued -= size
            used += size
            files.extend(page['files'])
            self.progress = page['progress']
            self.complete = page['complete']
        return {'files': files, 'progress': self.progress, 'complete': self.complete}
