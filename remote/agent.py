"""Plugin-owned, bounded filesystem agent. No application imports or UI policy."""
import base64
import atexit
import ctypes
import errno
import hashlib
import heapq
import json
import os
import pty
import re
import fcntl
import secrets
import select
import signal
import stat
import struct
import subprocess
import sys
import termios
import time
from collections import OrderedDict, deque
from contextlib import contextmanager
from urllib.parse import quote, unquote_to_bytes, urlsplit

# Bundled bootstrap concatenates this module before the Agent. Direct Python
# tests import the same source normally; neither path contains a second scanner.
if 'SearchWorker' not in globals():
    from search_agent import SearchWorker, WorkerBundle

PROTOCOL = 1
MAX_FRAME = 1024 * 1024
MAX_CHUNK = 256 * 1024
MAX_RECEIPTS = 256
MAX_FILE = 64 * 1024 * 1024
LEASE_SECONDS = 45
DIRECTORY_FLAGS = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC


class AgentError(Exception):
    def __init__(self, kind, message):
        super().__init__(message)
        self.kind = kind


class Watch:
    """Bounded recursive inotify subscription, owned by one mounted root.

    Directory descriptors prevent following links during registration. Directory
    moves and kernel overflow rebuild the subscription and require a rescan.
    https://man7.org/linux/man-pages/man7/inotify.7.html
    """
    def __init__(self, agent, root, path):
        self.agent, self.root = agent, root
        self.path = agent.path(root, path)
        self.fd = -1
        self.ready = False
        self.sequence = 0
        self.reset()

    def close(self):
        if self.fd >= 0:
            os.close(self.fd)
            self.fd = -1

    def reset(self):
        library = ctypes.CDLL(None, use_errno=True)
        init = library.inotify_init1
        init.argtypes, init.restype = [ctypes.c_int], ctypes.c_int
        add = library.inotify_add_watch
        add.argtypes, add.restype = [ctypes.c_int, ctypes.c_char_p, ctypes.c_uint32], ctypes.c_int
        fd = init(os.O_NONBLOCK | os.O_CLOEXEC)
        if fd < 0:
            raise OSError(ctypes.get_errno(), "cannot start filesystem monitoring")
        paths, seen, stack = {}, set(), [(self.path, 0)]
        entries = 0
        try:
            while stack:
                path, depth = stack.pop()
                if len(paths) >= 4096 or depth > 64:
                    raise AgentError("limit_exceeded", "filesystem watch directory budget exceeded")
                try:
                    with self.agent.parent(self.root, path) as (parent, name, canonical):
                        directory = os.open(name, DIRECTORY_FLAGS, dir_fd=parent)
                        try:
                            metadata = os.fstat(directory)
                            identity = (metadata.st_dev, metadata.st_ino)
                            if identity in seen:
                                continue
                            seen.add(identity)
                            # CLOSE_WRITE, ATTRIB, moves, creation/deletion and self
                            # movement. Register the opened inode, never a symlink.
                            wd = add(fd, f"/proc/self/fd/{directory}".encode(), 0x01000FCC)
                            if wd < 0:
                                raise OSError(ctypes.get_errno(), "cannot monitor workspace directory")
                            paths[wd] = canonical
                            with os.scandir(directory) as children:
                                for child in children:
                                    entries += 1
                                    if entries > 65536:
                                        raise AgentError("limit_exceeded", "filesystem watch entry budget exceeded")
                                    if child.is_dir(follow_symlinks=False):
                                        child.name.encode("utf-8")
                                        if len(stack) >= 4096:
                                            raise AgentError("limit_exceeded", "filesystem watch directory budget exceeded")
                                        stack.append((os.path.join(canonical, child.name), depth + 1))
                        finally:
                            os.close(directory)
                except (FileNotFoundError, NotADirectoryError):
                    # A descendant can disappear or become a file between
                    # enumeration and registration. Its watched parent reports
                    # the topology change; losing the subscription root remains
                    # an explicit failure. Never follow a replacement symlink.
                    if path == self.path:
                        raise
        except BaseException:
            os.close(fd)
            raise
        self.close()
        self.fd, self.paths = fd, paths

    def poll(self):
        if not self.ready:
            return None
        changed, rescan, rebuild, observed, path_bytes = set(), False, False, False, 0
        for _ in range(16):
            try:
                data = os.read(self.fd, 65536)
            except BlockingIOError:
                break
            if not data:
                break
            offset = 0
            while offset < len(data):
                wd, mask, _cookie, length = struct.unpack_from("iIII", data, offset)
                name = data[offset + 16:offset + 16 + length].split(b"\0", 1)[0]
                offset += 16 + length
                observed = True
                if mask & (0x4000 | 0x8000 | 0x2000 | 0x400 | 0x800):
                    rescan = rebuild = True
                parent = self.paths.get(wd)
                if parent is None:
                    rescan = True
                    continue
                if mask & 0x40000000 and mask & (0x40 | 0x80 | 0x100 | 0x200):
                    rescan = rebuild = True
                try:
                    path = os.path.join(parent, name.decode("utf-8")) if name else parent
                except UnicodeDecodeError:
                    rescan = True
                    continue
                for affected in (parent, path):
                    if affected not in changed:
                        path_bytes += len(affected.encode("utf-8"))
                        if len(changed) < 512 and path_bytes <= 128 * 1024:
                            changed.add(affected)
                        else:
                            rescan = True
        else:
            rescan = rebuild = True
        if not observed:
            return None
        if rebuild:
            self.reset()
        self.sequence += 1
        return {"kind": "changed", "sequence": self.sequence,
                "paths": sorted(changed), "rescan": rescan}


def terminal_size(cols, rows):
    if not all(isinstance(value, int) and not isinstance(value, bool) and 1 <= value <= 1000
               for value in (cols, rows)):
        raise AgentError("invalid_request", "invalid terminal dimensions")
    return struct.pack("HHHH", rows, cols, 0, 0)


class TerminalChild:
    def __init__(self, program, args, directory, env, size):
        pid, master = pty.fork()
        if pid == 0:
            try:
                os.fchdir(directory)
                fcntl.ioctl(0, termios.TIOCSWINSZ, size)
                os.execvpe(program, [program] + args, env)
            except BaseException:
                os.write(2, b"Could not start terminal program\r\n")
                # Never run the copied parent's Agent cleanup in the fork.
                os._exit(127)
        self.pid = pid
        self.returncode = None
        self.stdin = self.stdout = os.fdopen(master, "r+b", buffering=0)
        self.stderr = None

    def poll(self):
        if self.returncode is None:
            pid, status = os.waitpid(self.pid, os.WNOHANG)
            if pid:
                self.returncode = os.waitstatus_to_exitcode(status)
        return self.returncode

    def wait(self, timeout):
        deadline = time.monotonic() + timeout
        while self.poll() is None:
            if time.monotonic() >= deadline:
                raise subprocess.TimeoutExpired("terminal", timeout)
            time.sleep(0.001)
        return self.returncode


class ProcessSession:
    INPUT_LIMIT = 256 * 1024
    OUTPUT_CHUNK = 16 * 1024

    def __init__(self, root, program, args, directory, env, size=None):
        self.root = root
        self.ready = False
        self.initial_credit = True
        self.input = deque()
        self.input_bytes = 0
        self.closing_input = False
        self.stop_deadline = None
        self.exit_deadline = None
        self.terminal = size is not None
        self.directory_check = 0
        self.directory_token = object()
        # Argument arrays, separate binary pipes and a new process group.
        # No implicit shell and no local platform/path substitutions.
        if self.terminal:
            self.child = TerminalChild(program, args, directory, env, size)
            self.outputs = [(self.child.stdout, False)]
        else:
            self.child = subprocess.Popen([program] + args,
                cwd=f"/proc/{os.getpid()}/fd/{directory}", env=env,
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                start_new_session=True, bufsize=0)
            self.outputs = [(self.child.stdout, False), (self.child.stderr, True)]
        for stream in (self.child.stdin, self.child.stdout, self.child.stderr):
            if stream:
                os.set_blocking(stream.fileno(), False)

    def stop(self, force=False):
        self.ready = True
        groups = {self.child.pid}
        # Interactive shells put foreground jobs in separate groups of the same
        # session. Reclaim those groups too; processes that deliberately create
        # a different session are outside this lease's process-session boundary.
        with os.scandir("/proc") as entries:
            for entry in entries:
                if not entry.name.isdigit():
                    continue
                try:
                    with open(entry.path + "/stat") as stream:
                        fields = stream.read(4096).rsplit(")", 1)[1].split()
                    if int(fields[3]) == self.child.pid:
                        groups.add(int(fields[2]))
                except (OSError, ValueError, IndexError):
                    pass
        for group in groups:
            try:
                os.killpg(group, signal.SIGKILL if force else signal.SIGTERM)
            except ProcessLookupError:
                pass
        self.closing_input = True
        self.input.clear()
        self.input_bytes = 0
        if not force:
            self.stop_deadline = time.monotonic() + 1

    def close(self):
        self.stop(force=True)
        try:
            self.child.wait(timeout=2)
        except subprocess.TimeoutExpired:
            pass
        for stream in (self.child.stdin, self.child.stdout, self.child.stderr):
            if stream and not stream.closed:
                stream.close()

    def write(self, data):
        if self.closing_input or self.child.stdin.closed or self.child.poll() is not None:
            raise AgentError("unavailable", "process input is closed")
        if len(data) > MAX_CHUNK or self.input_bytes + len(data) > self.INPUT_LIMIT:
            raise AgentError("limit_exceeded", "process input window is full")
        if data:
            self.input.append(data)
            self.input_bytes += len(data)

    def poll(self, read_output=True):
        events = []
        if self.stop_deadline is not None and time.monotonic() >= self.stop_deadline:
            self.stop(force=True)
            self.stop_deadline = None
        if not self.ready:
            return events, False
        if self.initial_credit:
            events.append({"kind": "writable", "bytes": self.INPUT_LIMIT})
            self.initial_credit = False
        freed = 0
        for _ in range(16):
            if not self.input or self.child.stdin.closed:
                break
            try:
                count = os.write(self.child.stdin.fileno(), self.input[0])
            except BlockingIOError:
                break
            except BrokenPipeError:
                self.input.clear()
                self.input_bytes = 0
                self.closing_input = True
                break
            if not count:
                break
            chunk = self.input.popleft()
            if count < len(chunk):
                self.input.appendleft(chunk[count:])
            freed += count
            self.input_bytes -= count
        if freed:
            events.append({"kind": "writable", "bytes": freed})
        if not self.terminal and self.closing_input and not self.input and not self.child.stdin.closed:
            self.child.stdin.close()
        for stream, stderr in list(self.outputs) if read_output else []:
            try:
                data = os.read(stream.fileno(), self.OUTPUT_CHUNK)
            except BlockingIOError:
                continue
            except OSError as error:
                if self.terminal and error.errno == errno.EIO:
                    data = b""
                else:
                    raise
            if data:
                events.append({"kind": "output", "stderr": stderr,
                    "data": base64.b64encode(data).decode("ascii")})
            else:
                stream.close()
                self.outputs.remove((stream, stderr))
        code = self.child.poll()
        if code is not None:
            if not read_output and self.exit_deadline is not None:
                self.exit_deadline = time.monotonic() + 2
            if self.exit_deadline is not None and any(event["kind"] == "output" for event in events):
                self.exit_deadline = time.monotonic() + 2
            if self.exit_deadline is None:
                # A background descendant must not outlive the process lease.
                self.stop()
                self.exit_deadline = time.monotonic() + 2
            if not self.outputs or time.monotonic() >= self.exit_deadline:
                self.close()
                events.append({"kind": "exited", "code": code})
                return events, True
        return events, False


class Agent:
    def __init__(self):
        self.roots = {}
        self.root_fds = {}
        self.next_root = 0
        self.receipts = OrderedDict()
        self.receipt_bytes = 0
        self.last_mutation = 0
        self.writes = {}
        self.next_write = 0
        self.watches = {}
        self.next_watch = 0
        self.processes = {}
        self.next_process = 0
        self.search_bundle = None
        self.searches = {}
        self.retired_searches = []
        self.next_search = 0

    def mount(self, path):
        # A lone "~" stands for the login directory; no other path is expanded.
        root = os.path.realpath(os.path.expanduser("~") if path == "~" else path)
        if not os.path.isdir(root):
            raise AgentError("not_found", "workspace directory does not exist")
        for handle, existing in self.roots.items():
            if existing == root:
                return {"root": handle, "path": root}
        if len(self.roots) >= 128:
            raise AgentError("limit_exceeded", "too many workspace roots")
        self.next_root += 1
        self.root_fds[self.next_root] = os.open(root, DIRECTORY_FLAGS)
        self.roots[self.next_root] = root
        return {"root": self.next_root, "path": root}

    def path(self, root_id, value, follow=True):
        root = self.roots.get(root_id)
        if root is None:
            raise AgentError("unavailable", "workspace root is not mounted")
        if not isinstance(value, str) or not value or "\0" in value:
            raise AgentError("invalid_request", "invalid resource path")
        candidate = value if os.path.isabs(value) else os.path.join(root, value)
        candidate = os.path.normpath(candidate)
        resolved = os.path.realpath(candidate) if follow else os.path.join(
            os.path.realpath(os.path.dirname(candidate)), os.path.basename(candidate)
        )
        if os.path.commonpath([root, resolved]) != root:
            raise AgentError("permission_denied", "resource is outside the workspace")
        return resolved

    def close(self):
        for search in self.searches.values():
            self.retire_search(search)
        self.searches.clear()
        deadline = time.monotonic() + 1
        for child in self.retired_searches:
            try:
                child.wait(timeout=max(0, deadline - time.monotonic()))
            except subprocess.TimeoutExpired:
                pass
        self.reap_searches()
        if self.search_bundle:
            self.search_bundle.close()
            self.search_bundle = None
        for process in self.processes.values():
            process.stop(force=True)
        for process in self.processes.values():
            process.close()
        self.processes.clear()
        for watch in self.watches.values():
            watch.close()
        self.watches.clear()
        for write in list(self.writes):
            self.abort_write(write)
        for descriptor in self.root_fds.values():
            os.close(descriptor)
        self.root_fds.clear()
        self.roots.clear()

    def reap_searches(self):
        self.retired_searches = [child for child in self.retired_searches if child.poll() is None]

    def retire_search(self, search):
        child = search.close()
        if child is not None:
            self.retired_searches.append(child)

    def watch_events(self):
        self.reap_searches()
        for handle, watch in list(self.watches.items()):
            try:
                event = watch.poll()
            except (AgentError, OSError, UnicodeError) as error:
                event = {"kind": "watch_stopped", "error": {
                    "kind": error.kind if isinstance(error, AgentError) else "unavailable",
                    "message": "Filesystem monitoring stopped; refresh to retry"}}
                watch.close()
                del self.watches[handle]
            if event:
                event["watch"] = handle
                yield {"event": event}

    def process_events(self, read_output=True):
        for handle, process in list(self.processes.items()):
            events, ended = process.poll(read_output)
            if ended:
                del self.processes[handle]
            for event in events:
                if process.terminal:
                    event["kind"] = {"output": "terminal_output", "writable": "terminal_writable",
                        "exited": "terminal_exited"}[event["kind"]]
                    event.pop("stderr", None)
                event["handle"] = handle
                yield {"event": event}
            if process.terminal and process.ready and not ended and time.monotonic() >= process.directory_check:
                process.directory_check = time.monotonic() + 0.5
                # Provider-owned Linux lookup; no shell hooks or host path parsing.
                # https://man7.org/linux/man-pages/man5/proc_pid_cwd.5.html
                try:
                    directory = self.path(process.root, os.readlink(f"/proc/{process.child.pid}/cwd"))
                    if len(directory.encode("utf-8")) > 32768:
                        directory = None
                except (OSError, AgentError, UnicodeError):
                    directory = None
                if directory != process.directory_token:
                    process.directory_token = directory
                    yield {"event": {"kind": "terminal_directory", "handle": handle, "path": directory}}

    def spawn(self, root, params, terminal=False):
        if len(self.processes) >= 16:
            raise AgentError("limit_exceeded", "too many running processes")
        program, args, env = params.get("program"), params.get("args", []), params.get("env", [])
        size = terminal_size(params["cols"], params["rows"]) if terminal else None
        if terminal and program is None:
            program = os.environ.get("SHELL") or "/bin/sh"
        valid = lambda text: isinstance(text, str) and "\0" not in text and len(text.encode("utf-8")) <= 8192
        if not valid(program) or not program or not isinstance(args, list) or len(args) > 128 or not all(valid(arg) for arg in args):
            raise AgentError("invalid_request", "invalid process arguments")
        if not isinstance(env, list) or len(env) > 128 or any(
            not isinstance(pair, list) or len(pair) != 2 or not all(valid(value) for value in pair)
            or not pair[0] or "=" in pair[0] for pair in env):
            raise AgentError("invalid_request", "invalid process environment")
        if len(json.dumps([program, args, env]).encode()) > 128 * 1024:
            raise AgentError("limit_exceeded", "process arguments exceed the budget")
        environment = os.environ.copy()
        environment.update(env)
        if terminal:
            environment["TERM"] = "xterm-256color"
        with self.parent(root, params["cwd"]) as (parent, name, canonical):
            excluded = params.get("excluded_roots", [])
            if not isinstance(excluded, list) or len(excluded) > 128:
                raise AgentError("invalid_request", "invalid execution boundaries")
            for value in excluded:
                if not isinstance(value, str) or "\0" in value or not os.path.isabs(value):
                    raise AgentError("invalid_request", "invalid execution boundary")
                boundary = os.path.realpath(value)
                mounted = self.roots[root]
                if boundary != mounted and os.path.commonpath([mounted, boundary]) == mounted \
                        and os.path.commonpath([boundary, canonical]) == boundary:
                    raise AgentError("permission_denied", "program directory is in an untrusted nested root")
            directory = os.open(name, DIRECTORY_FLAGS, dir_fd=parent)
            try:
                # The child's chdir resolves this pinned parent descriptor
                # before Popen returns; it is not an untrusted cwd string.
                process = ProcessSession(root, program, args, directory, environment, size)
            finally:
                os.close(directory)
        self.next_process += 1
        self.processes[self.next_process] = process
        return {"terminal" if terminal else "process": self.next_process}

    def abort_write(self, write):
        stage = self.writes.pop(write, None)
        if stage is not None:
            try:
                if stage["temporary"] is not None:
                    os.unlink(stage["temporary"], dir_fd=stage["parent"])
            finally:
                os.close(stage["fd"])
                os.close(stage["parent"])

    def staged_write(self, method, params):
        root = params["root"]
        if method == "write_begin":
            size = params["size"]
            if not isinstance(size, int) or size < 0 or size > MAX_FILE:
                raise AgentError("limit_exceeded", "invalid staged file size")
            if len(self.writes) >= 16 or sum(w["size"] for w in self.writes.values()) + size > 256 * 1024 * 1024:
                raise AgentError("limit_exceeded", "staged write budget exceeded")
            with self.parent(root, params["path"]) as (parent, name, _):
                try:
                    metadata = os.stat(name, dir_fd=parent, follow_symlinks=False)
                except FileNotFoundError:
                    metadata = None
                if metadata and not stat.S_ISREG(metadata.st_mode):
                    raise AgentError("invalid_request", "resource is not a regular file")
                expected = params.get("expected_revision")
                if expected is not None and (metadata is None or self.revision(parent, name) != expected):
                    raise AgentError("conflict", "file differs from the saved revision")
                mode = stat.S_IMODE(metadata.st_mode) if metadata else 0o600
                if not mode & 0o222:
                    raise AgentError("permission_denied", "file is read-only")
                temporary = ".loog-save-" + secrets.token_hex(16)
                retained = os.dup(parent)
                try:
                    fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC, mode, dir_fd=retained)
                except BaseException:
                    os.close(retained)
                    raise
                self.next_write += 1
                self.writes[self.next_write] = dict(root=root, parent=retained, name=name, temporary=temporary,
                    fd=fd, size=size, offset=0, expected=expected, mode=mode, touched=time.monotonic())
                return {"write": self.next_write}
        write = params["write"]
        stage = self.writes.get(write)
        if stage is None:
            if method == "write_abort":
                return {}
            raise AgentError("unavailable", "staged write is no longer available")
        if stage["root"] != root:
            raise AgentError("permission_denied", "staged write belongs to another root")
        stage["touched"] = time.monotonic()
        if method == "write_abort":
            self.abort_write(write)
            return {}
        if method == "write_append":
            data = base64.b64decode(params["data"], validate=True)
            if params["offset"] != stage["offset"] or len(data) > MAX_CHUNK or stage["offset"] + len(data) > stage["size"]:
                raise AgentError("invalid_request", "invalid staged write offset or size")
            remaining = memoryview(data)
            while remaining:
                count = os.write(stage["fd"], remaining)
                if count <= 0:
                    raise AgentError("failed", "staged write made no progress")
                stage["offset"] += count
                remaining = remaining[count:]
            return {}
        if method == "write_commit":
            if stage["offset"] != stage["size"]:
                raise AgentError("invalid_request", "staged write is incomplete")
            try:
                os.fchmod(stage["fd"], stage["mode"])
                os.fsync(stage["fd"])
                if stage["expected"] is not None:
                    try:
                        current = self.revision(stage["parent"], stage["name"])
                    except FileNotFoundError:
                        current = None
                    if current != stage["expected"]:
                        raise AgentError("conflict", "file changed before replacement")
                os.replace(stage["temporary"], stage["name"], src_dir_fd=stage["parent"], dst_dir_fd=stage["parent"])
                stage["temporary"] = None
                revision = self.signature(os.fstat(stage["fd"]))
                try:
                    os.fsync(stage["parent"])
                except OSError as error:
                    raise AgentError("unknown_outcome", "file replaced but directory sync failed") from error
                return {"revision": revision}
            finally:
                self.abort_write(write)
        raise AgentError("unsupported", "unknown staged write operation")

    @contextmanager
    def parent(self, root_id, value, follow=True):
        """Resolve links, then walk from the retained root with NOFOLLOW.

        A path swapped to a symlink after resolution is refused, never followed.
        Directory handles remain the authority for the subsequent operation.
        """
        path = self.path(root_id, value, follow)
        relative = os.path.relpath(path, self.roots[root_id])
        parts = relative.split(os.sep)
        descriptor = os.dup(self.root_fds[root_id])
        try:
            for part in parts[:-1]:
                child = os.open(part, DIRECTORY_FLAGS, dir_fd=descriptor)
                os.close(descriptor)
                descriptor = child
            yield descriptor, parts[-1], path
        finally:
            os.close(descriptor)

    @staticmethod
    @contextmanager
    def regular(parent, name):
        descriptor = os.open(name, os.O_RDONLY | os.O_NONBLOCK | os.O_NOFOLLOW | os.O_CLOEXEC,
                             dir_fd=parent)
        try:
            metadata = os.fstat(descriptor)
            if not stat.S_ISREG(metadata.st_mode):
                raise AgentError("invalid_request", "resource is not a regular file")
            if metadata.st_size > MAX_FILE:
                raise AgentError("limit_exceeded", "file exceeds the editable size budget")
            with os.fdopen(descriptor, "rb", closefd=False) as stream:
                yield stream
        finally:
            os.close(descriptor)

    @classmethod
    def revision(cls, parent, name):
        with cls.regular(parent, name) as stream:
            return cls.signature(os.fstat(stream.fileno()))

    @staticmethod
    def signature(metadata):
        # Opaque observation token: inode identity plus size and nanosecond times.
        # Shared by stat/read/write; it is not a content hash or a strict CAS lock.
        return ":".join(str(value) for value in (
            metadata.st_dev, metadata.st_ino, metadata.st_size,
            metadata.st_mtime_ns, metadata.st_ctime_ns))

    def info(self, parent, name, path, include_revision=True):
        path.encode("utf-8")
        metadata = os.stat(name, dir_fd=parent, follow_symlinks=False)
        kind = ("symlink" if stat.S_ISLNK(metadata.st_mode) else
                "directory" if stat.S_ISDIR(metadata.st_mode) else
                "file" if stat.S_ISREG(metadata.st_mode) else "other")
        return {"path": path, "name": os.path.basename(path), "kind": kind,
                "size": metadata.st_size, "readonly": not bool(metadata.st_mode & 0o222),
                "revision": self.revision(parent, name) if kind == "file" and include_revision else None}

    @staticmethod
    def rename_noreplace(source_fd, source, target_fd, target):
        # Linux renameat2 RENAME_NOREPLACE checks and renames atomically.
        # https://man7.org/linux/man-pages/man2/renameat.2.html
        library = ctypes.CDLL(None, use_errno=True)
        function = getattr(library, "renameat2", None)
        if function is None:
            raise AgentError("unsupported", "atomic no-replace rename is unavailable")
        function.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_uint]
        function.restype = ctypes.c_int
        if function(source_fd, os.fsencode(source), target_fd, os.fsencode(target), 1) != 0:
            code = ctypes.get_errno()
            if code == errno.EEXIST:
                raise AgentError("conflict", "target already exists")
            if code in (errno.ENOSYS, errno.EINVAL, errno.EOPNOTSUPP):
                raise AgentError("unsupported", "atomic no-replace rename is unavailable")
            raise OSError(code, os.strerror(code))

    def dispatch(self, method, params):
        self.reap_searches()
        for handle, search in list(self.searches.items()):
            if time.monotonic() - search.touched > 30:
                self.retire_search(search)
                del self.searches[handle]
        for write, stage in list(self.writes.items()):
            if time.monotonic() - stage["touched"] > 120:
                self.abort_write(write)
        if method == "hello":
            return {"protocol": PROTOCOL, "platform": sys.platform, "machine": os.uname().machine,
                    "lease_seconds": LEASE_SECONDS,
                    "capabilities": ["files_read", "files_write", "watch", "process", "terminal"],
                    "max_chunk": MAX_CHUNK,
                    "write_guarantee": "revision-check-then-atomic-replace"}
        if method == "ping":
            return {}
        if method == "mount":
            return self.mount(params["path"])
        if method == 'search_bundle_begin':
            if self.search_bundle:
                raise AgentError('conflict', 'search bundle was already initialized')
            self.search_bundle = WorkerBundle(params['size'], params['sha256'])
            return {}
        if method in ('search_bundle_append', 'search_bundle_finish'):
            if not self.search_bundle:
                raise AgentError('unavailable', 'search bundle is not initialized')
            if method == 'search_bundle_append':
                self.search_bundle.append(params['offset'], params['data'])
            else:
                self.search_bundle.finish()
            return {}
        root_id = params["root"]
        if method == 'search':
            if len(self.searches) + len(self.retired_searches) >= 4:
                raise AgentError('limit_exceeded', 'too many concurrent searches')
            with self.parent(root_id, params['path']) as (parent, name, path):
                directory = os.open(name, DIRECTORY_FLAGS, dir_fd=parent)
                try:
                    search = SearchWorker(self.search_bundle, root_id, directory, path, params['options'])
                finally:
                    os.close(directory)
            self.next_search += 1
            self.searches[self.next_search] = search
            return {'search': self.next_search}
        if method in ('search_next', 'search_close'):
            handle = params['search']
            search = self.searches.get(handle)
            if search is None:
                if method == 'search_close':
                    return {}
                raise AgentError('unavailable', 'search is no longer available')
            if search.root != root_id:
                raise AgentError('permission_denied', 'search belongs to another root')
            if method == 'search_close':
                self.retire_search(search)
                del self.searches[handle]
                return {}
            return search.next(params['limit'])
        if method == "spawn":
            return self.spawn(root_id, params)
        if method == "terminal_open":
            return self.spawn(root_id, params, terminal=True)
        if method in {"terminal_ready", "terminal_input", "terminal_resize", "terminal_close"}:
            process = self.processes.get(params["terminal"])
            if process is None:
                if method in {"terminal_ready", "terminal_close"}:
                    return {}
                raise AgentError("unavailable", "terminal has exited")
            if process.root != root_id or not process.terminal:
                raise AgentError("permission_denied", "terminal belongs to another scope")
            if method == "terminal_ready":
                process.ready = True
            elif method == "terminal_input":
                process.write(base64.b64decode(params["data"], validate=True))
            elif method == "terminal_resize":
                fcntl.ioctl(process.child.stdin.fileno(), termios.TIOCSWINSZ,
                    terminal_size(params["cols"], params["rows"]))
            else:
                process.stop()
            return {}
        if method in {"process_ready", "process_input", "process_close_input", "process_kill"}:
            process = self.processes.get(params["process"])
            if process is None:
                if method in {"process_ready", "process_kill"}:
                    return {}
                raise AgentError("unavailable", "process has exited")
            if process.root != root_id or process.terminal:
                raise AgentError("permission_denied", "process belongs to another root")
            if method == "process_ready":
                process.ready = True
            elif method == "process_input":
                process.write(base64.b64decode(params["data"], validate=True))
            elif method == "process_close_input":
                process.closing_input = True
            else:
                process.stop()
            return {}
        if method == "watch":
            if len(self.watches) >= 16:
                raise AgentError("limit_exceeded", "too many filesystem subscriptions")
            watch = Watch(self, root_id, params["path"])
            self.next_watch += 1
            self.watches[self.next_watch] = watch
            return {"watch": self.next_watch}
        if method in {"unwatch", "watch_ready"}:
            watch = self.watches.get(params["watch"])
            if watch:
                if watch.root != root_id:
                    raise AgentError("permission_denied", "watch belongs to another root")
                if method == "unwatch":
                    watch.close()
                    del self.watches[params["watch"]]
                else:
                    watch.ready = True
            return {}
        if method == "parent_path":
            path = self.path(root_id, params["path"])
            return {"path": None if path == self.roots[root_id] else os.path.dirname(path)}
        if method in {"resolve_path", "relative_path"}:
            base = self.path(root_id, params["base"])
            with self.parent(root_id, base) as (parent, leaf, _):
                directory = os.open(leaf, DIRECTORY_FLAGS, dir_fd=parent)
                os.close(directory)
            if method == "resolve_path":
                relative = params["relative"]
                if not isinstance(relative, str) or not relative or "\0" in relative or len(relative.encode("utf-8")) > 32 * 1024:
                    raise AgentError("invalid_request", "invalid relative resource path")
                return {"path": self.path(root_id, os.path.join(base, relative))}
            path = self.path(root_id, params["path"])
            if os.path.commonpath([base, path]) != base:
                return {"components": None}
            parts = [] if path == base else os.path.relpath(path, base).split(os.sep)
            if len(parts) > 128 or sum(len(part.encode("utf-8")) for part in parts) > 32 * 1024:
                raise AgentError("limit_exceeded", "relative resource path exceeds its budget")
            return {"components": parts}
        if method == "resource_uri":
            path = self.path(root_id, params["path"])
            # RFC 8089: POSIX filenames are encoded as UTF-8; a backslash is a
            # filename character here, not a Windows separator.
            # https://www.rfc-editor.org/rfc/rfc8089
            return {"uri": "file://" + quote(path, safe="/")}
        if method == "resolve_uri":
            uri = params["uri"]
            if not isinstance(uri, str) or not uri or len(uri) > 128 * 1024 \
                    or not uri.isascii() or any(ord(char) <= 32 or ord(char) == 127 for char in uri) \
                    or any(char in uri for char in "?#\\") or re.search(r"%(?![0-9a-fA-F]{2})", uri):
                raise AgentError("invalid_request", "invalid resource URI")
            parsed = urlsplit(uri)
            if parsed.scheme.lower() != "file" or parsed.netloc.lower() not in {"", "localhost"} \
                    or not parsed.path.startswith("/"):
                raise AgentError("unsupported", "URI does not name a file in this environment")
            try:
                path = unquote_to_bytes(parsed.path).decode("utf-8", errors="strict")
            except UnicodeDecodeError as error:
                raise AgentError("invalid_request", "URI path is not UTF-8") from error
            if len(path.encode("utf-8")) > 32 * 1024:
                raise AgentError("limit_exceeded", "URI path exceeds the resource budget")
            return {"path": self.path(root_id, path)}
        if method == "child_path":
            name = params["name"]
            if not isinstance(name, str) or name in {"", ".", ".."} or "/" in name or "\0" in name:
                raise AgentError("invalid_request", "enter a single file or folder name")
            if len(os.fsencode(name)) > 255:
                raise AgentError("invalid_request", "name is too long")
            with self.parent(root_id, params["parent"]) as (parent, leaf, path):
                directory = os.open(leaf, DIRECTORY_FLAGS, dir_fd=parent)
                os.close(directory)
                return {"path": os.path.join(path, name)}
        if method == "create_file":
            with self.parent(root_id, params["path"], follow=False) as (parent, name, path):
                descriptor = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600, dir_fd=parent)
                try:
                    os.fsync(descriptor)
                    os.fsync(parent)
                    return self.info(parent, name, path)
                except OSError as error:
                    raise AgentError("unknown_outcome", "file created but confirmation failed") from error
                finally:
                    os.close(descriptor)
        if method in {"write_begin", "write_append", "write_commit", "write_abort"}:
            return self.staged_write(method, params)
        if method == "stat":
            with self.parent(root_id, params["path"], follow=False) as (parent, name, path):
                return self.info(parent, name, path)
        if method == "list":
            limit = max(1, min(int(params.get("limit", 512)), 512))
            cursor = params.get("cursor")
            with self.parent(root_id, params["path"]) as (parent, leaf, path):
                directory = os.open(leaf, DIRECTORY_FLAGS, dir_fd=parent)
                try:
                    # Keep at most one page in memory, even for huge directories.
                    with os.scandir(directory) as scan:
                        names = heapq.nsmallest(limit + 1, (
                            item.name for item in scan if cursor is None or item.name > cursor))
                    page = names[:limit]
                    entries = []
                    for name in page:
                        try:
                            entries.append(self.info(directory, name, os.path.join(path, name),
                                                     include_revision=False))
                        except FileNotFoundError:
                            continue
                    return {"entries": entries, "cursor": page[-1] if len(names) > limit else None}
                finally:
                    os.close(directory)
        if method == "read":
            offset = int(params.get("offset", 0))
            limit = max(1, min(int(params.get("limit", MAX_CHUNK)), MAX_CHUNK))
            if offset < 0:
                raise AgentError("invalid_request", "negative read offset")
            with self.parent(root_id, params["path"]) as (parent, name, _), self.regular(parent, name) as stream:
                before = os.fstat(stream.fileno())
                if not stat.S_ISREG(before.st_mode):
                    raise AgentError("invalid_request", "resource is not a regular file")
                stream.seek(offset)
                data = stream.read(limit)
                after = os.fstat(stream.fileno())
            if self.signature(before) != self.signature(after):
                raise AgentError("conflict", "file changed during read")
            return {"data": base64.b64encode(data).decode("ascii"),
                    "eof": offset + len(data) >= after.st_size,
                    "revision": self.signature(after),
                    "observation": [after.st_size, after.st_mtime_ns, after.st_ctime_ns]}
        if method == "write":
            data = base64.b64decode(params["data"], validate=True)
            if len(data) > MAX_CHUNK:
                raise AgentError("limit_exceeded", "write exceeds the chunk budget")
            expected = params.get("expected_revision")
            with self.parent(root_id, params["path"]) as (parent, name, _):
                try:
                    metadata = os.stat(name, dir_fd=parent, follow_symlinks=False)
                except FileNotFoundError:
                    metadata = None
                if metadata and not stat.S_ISREG(metadata.st_mode):
                    raise AgentError("invalid_request", "resource is not a regular file")
                if expected is not None and (metadata is None or self.revision(parent, name) != expected):
                    raise AgentError("conflict", "file differs from the saved revision")
                mode = stat.S_IMODE(metadata.st_mode) if metadata else 0o600
                if not mode & 0o222:
                    raise AgentError("permission_denied", "file is read-only")
                temporary = ".loog-save-" + secrets.token_hex(16)
                descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC,
                                     mode, dir_fd=parent)
                try:
                    with os.fdopen(descriptor, "wb", closefd=False) as stream:
                        stream.write(data)
                        os.fchmod(stream.fileno(), mode)
                        stream.flush()
                        os.fsync(stream.fileno())
                    # External writers can still race this check and replacement;
                    # hello advertises this guarantee rather than strict CAS.
                    if expected is not None and self.revision(parent, name) != expected:
                        raise AgentError("conflict", "file changed before replacement")
                    os.replace(temporary, name, src_dir_fd=parent, dst_dir_fd=parent)
                    temporary = None
                    revision = self.signature(os.fstat(descriptor))
                    try:
                        os.fsync(parent)
                    except OSError as error:
                        raise AgentError("unknown_outcome", "file replaced but directory sync failed") from error
                finally:
                    os.close(descriptor)
                    if temporary is not None:
                        os.unlink(temporary, dir_fd=parent)
            return {"revision": revision}
        if method == "mkdir":
            with self.parent(root_id, params["path"], follow=False) as (parent, name, _):
                os.mkdir(name, dir_fd=parent)
            return {}
        if method in {"rename", "relocate"}:
            with self.parent(root_id, params["source"], False) as (source_fd, source, source_path), \
                    self.parent(root_id, params["target"], False) as (target_fd, target, target_path):
                if source_path == self.roots[root_id] or target_path == self.roots[root_id]:
                    raise AgentError("permission_denied", "cannot rename a mounted root")
                entries = []
                if method == "relocate":
                    tracked = params["tracked"]
                    if not isinstance(tracked, list) or len(tracked) > 1024:
                        raise AgentError("limit_exceeded", "too many tracked resources")
                    # Path interpretation stays in the provider. No client-side
                    # prefix test or platform separator is needed for descendants.
                    normalized = {value: self.path(root_id, value, follow=False) for value in tracked}
                    for value, old in normalized.items():
                        if os.path.commonpath([source_path, old]) == source_path:
                            suffix = os.path.relpath(old, source_path)
                            new = target_path if suffix == "." else os.path.join(target_path, suffix)
                            entries.append({"source": value, "target": new, "name": os.path.basename(new)})
                    moved = {normalized[entry["source"]] for entry in entries}
                    occupied = set(normalized.values()) - moved
                    if any(entry["target"] in occupied for entry in entries):
                        raise AgentError("conflict", "target is already open as another resource")
                    if len(json.dumps(entries).encode()) > MAX_FRAME // 2:
                        raise AgentError("limit_exceeded", "rename mapping exceeds the result budget")
                if method == "rename" and params.get("overwrite", False):
                    os.rename(source, target, src_dir_fd=source_fd, dst_dir_fd=target_fd)
                else:
                    self.rename_noreplace(source_fd, source, target_fd, target)
                try:
                    os.fsync(source_fd)
                    os.fsync(target_fd)
                except OSError as error:
                    raise AgentError("unknown_outcome", "resource renamed but directory sync failed") from error
            return {"entries": entries} if method == "relocate" else {}
        if method == "remove":
            with self.parent(root_id, params["path"], follow=False) as (parent, name, path):
                if path == self.roots[root_id]:
                    raise AgentError("permission_denied", "cannot remove a mounted root")
                metadata = os.stat(name, dir_fd=parent, follow_symlinks=False)
                if stat.S_ISDIR(metadata.st_mode):
                    if params.get("recursive", False):
                        raise AgentError("unsupported", "recursive deletion is not available")
                    os.rmdir(name, dir_fd=parent)
                else:
                    os.unlink(name, dir_fd=parent)
            return {}
        raise AgentError("unsupported", "unknown agent operation")

    def handle(self, request):
        request_id = request.get("id")
        if not isinstance(request_id, int) or isinstance(request_id, bool) or request_id < 1:
            return {"id": request_id, "error": {"kind": "invalid_request", "message": "invalid request id"}}
        digest = hashlib.sha256(json.dumps(request, sort_keys=True, separators=(",", ":")).encode()).digest()
        mutating = request.get("method") in {"write", "mkdir", "rename", "relocate", "create_file", "remove", "write_begin", "write_append", "write_commit", "write_abort", "watch", "unwatch", "watch_ready",
            "spawn", "process_ready", "process_input", "process_close_input", "process_kill",
            "terminal_open", "terminal_ready", "terminal_input", "terminal_resize", "terminal_close",
            "search", "search_next", "search_close", "search_bundle_begin", "search_bundle_append", "search_bundle_finish"}
        if mutating and request_id in self.receipts:
            previous, response = self.receipts[request_id]
            if previous == digest:
                return response
            return {"id": request_id, "error": {"kind": "conflict", "message": "request id reused with different content"}}
        # Mutations are submitted in monotonically increasing request order.
        # An evicted receipt is unknown, never permission to replay its operation.
        if mutating and request_id <= self.last_mutation:
            return {"id": request_id, "error": {"kind": "unknown_outcome", "message": "mutation receipt is no longer retained"}}
        try:
            response = {"id": request_id, "result": self.dispatch(request["method"], request.get("params", {}))}
        except AgentError as error:
            response = {"id": request_id, "error": {"kind": error.kind, "message": str(error)}}
        except FileNotFoundError:
            response = {"id": request_id, "error": {"kind": "not_found", "message": "resource does not exist"}}
        except FileExistsError:
            response = {"id": request_id, "error": {"kind": "conflict", "message": "resource already exists"}}
        except PermissionError:
            response = {"id": request_id, "error": {"kind": "permission_denied", "message": "resource access denied"}}
        except UnicodeError:
            response = {"id": request_id, "error": {"kind": "unsupported", "message": "this provider requires UTF-8 resource names"}}
        except (OSError, ValueError, KeyError, TypeError) as error:
            response = {"id": request_id, "error": {"kind": "failed", "message": str(error)}}
        if mutating:
            self.last_mutation = request_id
            self.receipts[request_id] = (digest, response)
            self.receipt_bytes += len(json.dumps(response, ensure_ascii=True, separators=(',', ':')).encode())
            while len(self.receipts) > MAX_RECEIPTS or self.receipt_bytes > 4 * MAX_FRAME:
                _, (_, retired) = self.receipts.popitem(last=False)
                self.receipt_bytes -= len(json.dumps(retired, ensure_ascii=True, separators=(',', ':')).encode())
        return response


def main(lease_seconds=LEASE_SECONDS):
    agent = Agent()
    atexit.register(agent.close)
    def stopped(signum, _frame):
        # SSH hangup and ordinary termination must remove unfinished staged
        # files. SIGKILL and machine loss cannot run cleanup.
        raise SystemExit(128 + signum)
    for signum in (signal.SIGHUP, signal.SIGTERM, signal.SIGINT):
        signal.signal(signum, stopped)
    outgoing, outgoing_bytes = deque(), 0
    def emit(response):
        nonlocal outgoing_bytes
        encoded = json.dumps(response, ensure_ascii=True, separators=(",", ":")).encode() + b"\n"
        if len(encoded) > MAX_FRAME:
            encoded = json.dumps({"id": response.get("id"), "error": {
                "kind": "limit_exceeded", "message": "response frame budget exceeded"}}).encode() + b"\n"
        if outgoing_bytes + len(encoded) > 4 * MAX_FRAME:
            raise SystemExit("agent output budget exceeded")
        outgoing.append(encoded)
        outgoing_bytes += len(encoded)

    def flush_output():
        nonlocal outgoing_bytes
        for _ in range(32):
            if not outgoing:
                return
            try:
                count = os.write(sys.stdout.fileno(), outgoing[0])
            except BlockingIOError:
                return
            except BrokenPipeError:
                raise SystemExit(0)
            if not count:
                return
            chunk = outgoing.popleft()
            if count < len(chunk):
                outgoing.appendleft(chunk[count:])
            outgoing_bytes -= count

    def request(line):
        try:
            value = json.loads(line)
            if not isinstance(value, dict):
                raise ValueError("request must be an object")
            response = agent.handle(value)
        except (ValueError, UnicodeDecodeError) as error:
            response = {"id": None, "error": {"kind": "invalid_request", "message": str(error)}}
        emit(response)
        return "result" in response or response.get("id") is not None

    os.set_blocking(sys.stdin.fileno(), False)
    os.set_blocking(sys.stdout.fileno(), False)
    pending = bytearray()
    last_input = time.monotonic()
    while True:
        if time.monotonic() - last_input > lease_seconds:
            return
        # read1 also consumes bytes buffered by the verified bootstrap. A raw
        # read distinguishes EAGAIN from EOF when BufferedReader returns empty.
        try:
            chunk = sys.stdin.buffer.read1(65536)
            if not chunk:
                chunk = os.read(sys.stdin.fileno(), 65536)
        except BlockingIOError:
            chunk = None
        if chunk == b"":
            return
        if chunk:
            pending.extend(chunk)
            while b"\n" in pending:
                line, _, remaining = pending.partition(b"\n")
                if len(line) + 1 > MAX_FRAME:
                    raise SystemExit("agent frame budget exceeded")
                pending = bytearray(remaining)
                if request(line):
                    last_input = time.monotonic()
            if len(pending) >= MAX_FRAME:
                raise SystemExit("agent frame budget exceeded")
        for event in agent.watch_events():
            emit(event)
        read_output = outgoing_bytes < 2 * MAX_FRAME
        for event in agent.process_events(read_output):
            emit(event)
        flush_output()
        descriptors = [sys.stdin.fileno()] + [
            watch.fd for watch in agent.watches.values() if watch.ready]
        writable = [sys.stdout.fileno()] if outgoing else []
        for process in agent.processes.values():
            if process.ready:
                if read_output:
                    descriptors.extend(stream.fileno() for stream, _ in process.outputs)
                if process.input and not process.child.stdin.closed:
                    writable.append(process.child.stdin.fileno())
        select.select(descriptors, writable, [], 0 if chunk else 0.1)


if __name__ == "__main__":
    main()
