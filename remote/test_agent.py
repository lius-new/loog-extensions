import base64
import json
import os
import signal
import subprocess
import struct
import sys
import tempfile
import time
import unittest
from unittest.mock import patch
from agent import Agent, MAX_RECEIPTS


class FilesystemTests(unittest.TestCase):
    def test_relative_resource_paths_use_target_names_and_enforce_scope(self):
        base = os.path.join(self.directory.name, "src")
        os.mkdir(base)
        name = "中文\\image #%.png"
        path = os.path.join(base, name)
        self.assertEqual(self.call("resolve_path", base=base, relative=name)["result"]["path"], path)
        self.assertEqual(self.call("relative_path", base=self.directory.name, path=path)["result"]["components"], ["src", name])
        self.assertEqual(self.call("relative_path", base=base, path=base)["result"]["components"], [])
        self.assertIsNone(self.call("relative_path", base=base, path=self.directory.name)["result"]["components"])
        self.assertEqual(self.call("resolve_path", base=base, relative="../missing") ["result"]["path"], os.path.join(self.directory.name, "missing"))
        os.symlink("/etc", os.path.join(base, "outside"))
        for relative in ["../../outside", "outside/passwd", "/etc/passwd"]:
            self.assertEqual(self.call("resolve_path", base=base, relative=relative)["error"]["kind"], "permission_denied")
        for relative in ["", "bad\0path", "x" * (32 * 1024 + 1)]:
            self.assertEqual(self.call("resolve_path", base=base, relative=relative)["error"]["kind"], "invalid_request")

    def test_resource_uris_preserve_posix_names_and_reject_outside_or_ambiguous_locations(self):
        name = "中 😀%#?\\.rs"
        path = os.path.join(self.directory.name, name)
        with open(path, "w", encoding="utf-8") as output:
            output.write("contents")
        uri = self.call("resource_uri", path=path)["result"]["uri"]
        self.assertIn("%5C", uri)
        self.assertIn("%23%3F", uri)
        self.assertEqual(self.call("resolve_uri", uri=uri)["result"]["path"], path)
        self.assertEqual(self.call("resolve_uri", uri=uri.replace("file:///", "file://localhost/", 1))["result"]["path"], path)
        for invalid in ["file:///etc/passwd", "file://foreign/work", "file:///bad%0G", "file:///bad%FF", uri + "#", uri + "?", " " + uri, uri + "\n", "https://host/work", "file:///bad%00"]:
            self.assertIn("error", self.call("resolve_uri", uri=invalid), invalid)
        os.symlink("/etc", os.path.join(self.directory.name, "outside"))
        escaped = "file://" + self.directory.name + "/outside/passwd"
        self.assertEqual(self.call("resolve_uri", uri=escaped)["error"]["kind"], "permission_denied")

    def test_parent_paths_stop_at_the_mounted_root_and_keep_provider_spelling(self):
        name = "a\\b"
        path = os.path.join(self.directory.name, name)
        os.mkdir(path)
        self.assertEqual(self.call("parent_path", path=path)["result"]["path"], self.directory.name)
        self.assertEqual(self.call("parent_path", path=path + "/child.rs")["result"]["path"], path)
        self.assertIsNone(self.call("parent_path", path=self.directory.name)["result"]["path"])
        self.assertEqual(self.call("parent_path", path="/")["error"]["kind"], "permission_denied")

    def test_terminal_directory_reports_stay_in_the_provider_namespace(self):
        nested = os.path.join(self.directory.name, "中文")
        os.mkdir(nested)
        handle = self.call("terminal_open", program="/bin/sh", args=[], cwd=".", cols=80, rows=24)["result"]["terminal"]
        self.call("terminal_ready", terminal=handle)
        def directory(expected):
            deadline = time.monotonic() + 5
            while True:
                events = [item["event"] for item in self.agent.process_events()]
                if any(event["kind"] == "terminal_directory" and event["path"] == expected for event in events):
                    return
                self.assertLess(time.monotonic(), deadline, "cwd report did not arrive")
                time.sleep(0.01)
        directory(self.directory.name)
        self.call("terminal_input", terminal=handle, data=base64.b64encode("cd 中文\n".encode()).decode())
        directory(nested)
        self.call("terminal_input", terminal=handle, data=base64.b64encode(b"cd /\n").decode())
        directory(None)
        self.call("terminal_close", terminal=handle)
        self.drain_process(handle)

    def test_execution_excludes_nested_roots_without_host_path_interpretation(self):
        nested = os.path.join(self.directory.name, "nested")
        sibling = os.path.join(self.directory.name, "nested2")
        os.mkdir(nested)
        os.mkdir(sibling)
        params = dict(program=sys.executable, args=["-c", "pass"], excluded_roots=[nested])
        for method in ("spawn", "terminal_open"):
            size = dict(cols=80, rows=24) if method == "terminal_open" else {}
            self.assertEqual(self.call(method, cwd="nested", **params, **size)["error"]["kind"], "permission_denied")
        os.symlink(nested, os.path.join(self.directory.name, "alias"))
        self.assertEqual(self.call("spawn", cwd="alias", **params)["error"]["kind"], "permission_denied")
        handle = self.call("spawn", cwd="nested2", **params)["result"]["process"]
        self.call("process_ready", process=handle)
        self.assertEqual(self.drain_process(handle)[-1]["code"], 0)
        child_root = self.agent.mount(nested)["root"]
        handle = self.call("spawn", root=child_root, cwd=".", program=sys.executable,
            args=["-c", "pass"], excluded_roots=[self.directory.name])["result"]["process"]
        self.call("process_ready", root=child_root, process=handle)
        self.assertEqual(self.drain_process(handle)[-1]["code"], 0)

    def test_connection_lease_expires_even_when_protocol_output_is_backpressured(self):
        code = "import os,sys,time; print(os.getpid(),flush=True); sys.stdout.buffer.write(b'x'*8000000); time.sleep(60)"
        with subprocess.Popen([sys.executable, "-u", "-c", "import agent; agent.main(lease_seconds=1)"],
                cwd=os.path.dirname(__file__), stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE) as process:
            try:
                requests = [
                    {"id": 1, "method": "mount", "params": {"path": self.directory.name}},
                    {"id": 2, "method": "spawn", "params": {"root": 1, "program": sys.executable, "args": ["-u", "-c", code], "cwd": ".", "env": []}},
                    {"id": 3, "method": "process_ready", "params": {"root": 1, "process": 1}},
                ]
                for request in requests:
                    process.stdin.write(json.dumps(request).encode() + b"\n")
                    process.stdin.flush()
                    self.assertIn("result", json.loads(process.stdout.readline()))
                pid = None
                while pid is None:
                    event = json.loads(process.stdout.readline()).get("event", {})
                    if event.get("kind") == "output":
                        pid = int(base64.b64decode(event["data"]).split(b"\n", 1)[0])
                # Keep stdin open and stop reading stdout: neither SSH EOF nor
                # writable output can be relied on to wake the lease cleanup.
                process.wait(timeout=5)
                self.assertEqual(process.returncode, 0)
                try:
                    with open(f"/proc/{pid}/stat") as stream:
                        self.assertEqual(stream.read().split()[2], "Z")
                except FileNotFoundError:
                    pass
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait()

    def test_terminal_has_controlling_tty_unicode_resize_and_exit(self):
        code = ("import os,sys; print('SIZE:%dx%d'%os.get_terminal_size(),flush=True); "
            "line=sys.stdin.readline(); print('REPLY:'+line.strip(),flush=True); "
            "print('SIZE:%dx%d'%os.get_terminal_size(),flush=True)")
        handle = self.call("terminal_open", program=sys.executable, args=["-u", "-c", code],
            cwd=".", cols=80, rows=24)["result"]["terminal"]
        self.call("terminal_ready", terminal=handle)
        output, deadline = bytearray(), time.monotonic() + 5
        while b"SIZE:80x24" not in output:
            for entry in self.agent.process_events():
                if entry["event"]["kind"] == "terminal_output":
                    output.extend(base64.b64decode(entry["event"]["data"]))
            self.assertLess(time.monotonic(), deadline)
            time.sleep(0.001)
        self.call("terminal_resize", terminal=handle, cols=100, rows=40)
        self.call("terminal_input", terminal=handle, data=base64.b64encode("中文\n".encode()).decode())
        events = self.drain_process(handle)
        for event in events:
            if event["kind"] == "terminal_output":
                output.extend(base64.b64decode(event["data"]))
        self.assertIn("REPLY:中文".encode(), output)
        self.assertIn(b"SIZE:100x40", output)
        self.assertEqual(events[-1]["kind"], "terminal_exited")
        self.assertEqual(events[-1]["code"], 0)

    def test_terminal_close_reaps_separate_foreground_groups_and_rejects_bad_sizes(self):
        self.assertEqual(self.call("terminal_open", program=None, args=[], cwd=".",
            cols=0, rows=24)["error"]["kind"], "invalid_request")
        code = ("import os,subprocess,sys,time; "
            "p=subprocess.Popen([sys.executable,'-c','import time; time.sleep(60)'],process_group=0); "
            "os.tcsetpgrp(0,p.pid); print(p.pid,flush=True); time.sleep(60)")
        handle = self.call("terminal_open", program=sys.executable, args=["-u", "-c", code],
            cwd=".", cols=80, rows=24)["result"]["terminal"]
        self.call("terminal_ready", terminal=handle)
        pid, deadline = None, time.monotonic() + 5
        while pid is None:
            for entry in self.agent.process_events():
                event = entry["event"]
                if event["kind"] == "terminal_output":
                    pid = int(base64.b64decode(event["data"]).strip())
            self.assertLess(time.monotonic(), deadline)
            time.sleep(0.001)
        self.assertEqual(self.call("process_input", process=handle, data="eA==")["error"]["kind"], "permission_denied")
        self.call("terminal_close", terminal=handle)
        events = self.drain_process(handle)
        self.assertNotEqual(events[-1]["code"], 0)
        try:
            with open(f"/proc/{pid}/stat") as stream:
                self.assertEqual(stream.read().split()[2], "Z")
        except FileNotFoundError:
            pass

    def drain_process(self, handle, timeout=8):
        result = []
        deadline = time.monotonic() + timeout
        while handle in self.agent.processes:
            result.extend(entry["event"] for entry in self.agent.process_events())
            self.assertLess(time.monotonic(), deadline, "process did not exit")
            time.sleep(0.001)
        return result

    def test_process_binary_input_output_environment_and_explicit_close(self):
        program = "import os,sys; sys.stderr.write(os.getcwd()+chr(10)+os.environ['LOOG_TEST']); sys.stdout.buffer.write(sys.stdin.buffer.read())"
        handle = self.call("spawn", program=sys.executable, args=["-c", program], cwd=".",
            env=[["LOOG_TEST", "中文"]])["result"]["process"]
        self.assertEqual(list(self.agent.process_events()), [])
        payload = bytes(range(256)) * 1024
        self.assertIn("result", self.call("process_input", process=handle,
            data=base64.b64encode(payload).decode()))
        self.assertEqual(self.call("process_input", process=handle, data="eA==")["error"]["kind"], "limit_exceeded")
        self.call("process_ready", process=handle)
        self.call("process_close_input", process=handle)
        events = self.drain_process(handle)
        stdout = b"".join(base64.b64decode(event["data"]) for event in events
            if event["kind"] == "output" and not event["stderr"])
        stderr = b"".join(base64.b64decode(event["data"]) for event in events
            if event["kind"] == "output" and event["stderr"])
        self.assertEqual(stdout, payload)
        self.assertEqual(stderr.decode(), self.directory.name + "\n中文")
        self.assertEqual(sum(event["bytes"] for event in events if event["kind"] == "writable"),
            256 * 1024 + len(payload))
        self.assertEqual(events[-1]["kind"], "exited")
        self.assertEqual(events[-1]["code"], 0)

    def test_process_large_dual_output_and_cancellation_reap_process_group(self):
        program = "import sys; sys.stderr.buffer.write(b'e'*2000000); sys.stdout.buffer.write(b'o'*2000000)"
        handle = self.call("spawn", program=sys.executable, args=["-c", program], cwd=".", env=[])["result"]["process"]
        self.call("process_ready", process=handle)
        events = self.drain_process(handle)
        for stderr, byte in ((False, b"o"), (True, b"e")):
            data = b"".join(base64.b64decode(event["data"]) for event in events
                if event["kind"] == "output" and event["stderr"] == stderr)
            self.assertEqual(data, byte * 2000000)
        program = "import subprocess,sys,time; p=subprocess.Popen([sys.executable,'-c','import time; time.sleep(60)']); print(p.pid,flush=True); time.sleep(60)"
        handle = self.call("spawn", program=sys.executable, args=["-c", program], cwd=".", env=[])["result"]["process"]
        self.call("process_ready", process=handle)
        deadline, pid = time.monotonic() + 5, None
        while pid is None:
            for entry in self.agent.process_events():
                event = entry["event"]
                if event["kind"] == "output":
                    pid = int(base64.b64decode(event["data"]).strip())
            self.assertLess(time.monotonic(), deadline)
            time.sleep(0.001)
        self.call("process_kill", process=handle)
        events = self.drain_process(handle)
        self.assertNotEqual(events[-1]["code"], 0)
        try:
            with open(f"/proc/{pid}/stat") as stream:
                self.assertEqual(stream.read().split()[2], "Z")
        except FileNotFoundError:
            pass

    def test_process_cwd_scope_and_connection_cleanup(self):
        denied = self.call("spawn", program=sys.executable, args=["-c", "pass"], cwd="/", env=[])
        self.assertEqual(denied["error"]["kind"], "permission_denied")
        handle = self.call("spawn", program=sys.executable,
            args=["-c", "import time; time.sleep(60)"], cwd=".", env=[])["result"]["process"]
        child = self.agent.processes[handle].child
        self.assertIsNone(child.poll())
        self.agent.close()
        self.assertIsNotNone(child.poll())
        self.assertEqual(self.agent.processes, {})

    def test_watch_survives_directory_removed_during_recursive_registration(self):
        child = os.path.join(self.directory.name, "disappearing")
        os.mkdir(child)
        original = self.agent.parent
        def remove_before_open(root, path):
            if path == child and os.path.exists(child):
                os.rmdir(child)
            return original(root, path)
        with patch.object(self.agent, "parent", side_effect=remove_before_open):
            reply = self.call("watch", path=".")
        self.assertIn("result", reply)
        handle = reply["result"]["watch"]
        self.call("watch_ready", watch=handle)
        self.write("after-delete.txt", b"still monitored")
        events = list(self.agent.watch_events())
        self.assertTrue(any(event["event"]["kind"] == "changed" for event in events), events)
        self.assertFalse(any(event["event"]["kind"] == "watch_stopped" for event in events), events)

    def test_watch_requires_receipt_ack_and_tracks_nested_changes_and_renames(self):
        self.call("mkdir", path="nested")
        handle = self.call("watch", path=".")["result"]["watch"]
        self.write("nested/中文", b"one")
        self.assertEqual(list(self.agent.watch_events()), [])
        self.call("watch_ready", watch=handle)
        events = list(self.agent.watch_events())
        self.assertEqual(len(events), 1)
        self.assertIn(os.path.join(self.directory.name, "nested/中文"), events[0]["event"]["paths"])
        self.call("rename", source="nested", target="moved")
        event = list(self.agent.watch_events())[0]["event"]
        self.assertTrue(event["rescan"])
        self.write("moved/中文", b"two")
        event = list(self.agent.watch_events())[0]["event"]
        self.assertIn(os.path.join(self.directory.name, "moved/中文"), event["paths"])
        self.assertGreater(event["sequence"], 1)
        self.call("unwatch", watch=handle)
        self.assertEqual(list(self.agent.watch_events()), [])

    def test_watch_overflow_rescans_and_repeated_release_does_not_leak_descriptors(self):
        baseline = len(os.listdir("/proc/self/fd"))
        for _ in range(100):
            handle = self.call("watch", path=".")["result"]["watch"]
            self.call("watch_ready", watch=handle)
            with patch("agent.os.read", side_effect=[struct.pack("iIII", -1, 0x4000, 0, 0), BlockingIOError()]):
                event = list(self.agent.watch_events())[0]["event"]
            self.assertTrue(event["rescan"])
            self.call("unwatch", watch=handle)
        self.assertEqual(len(os.listdir("/proc/self/fd")), baseline)

    def test_watch_excludes_external_links_and_reports_loss_of_the_watched_directory(self):
        with tempfile.TemporaryDirectory() as outside:
            os.symlink(outside, os.path.join(self.directory.name, "outside"))
            handle = self.call("watch", path=".")["result"]["watch"]
            self.call("watch_ready", watch=handle)
            with open(os.path.join(outside, "private"), "wb") as stream:
                stream.write(b"outside")
            self.assertEqual(list(self.agent.watch_events()), [])
            self.call("mkdir", path="nested")
            list(self.agent.watch_events())
            nested = self.call("watch", path="nested")["result"]["watch"]
            self.call("watch_ready", watch=nested)
            self.call("remove", path="nested")
            events = [entry["event"] for entry in self.agent.watch_events()]
            self.assertTrue(any(event["kind"] == "watch_stopped" and event["watch"] == nested for event in events))
            self.assertNotIn(nested, self.agent.watches)

    def test_child_names_and_exclusive_creation_use_linux_rules(self):
        for name in ("", ".", "..", "a/b", "bad\0name"):
            self.assertEqual(self.call("child_path", parent=".", name=name)["error"]["kind"], "invalid_request")
        child = self.call("child_path", parent=".", name="中文\\name.txt")["result"]["path"]
        self.assertEqual(child, os.path.join(self.directory.name, "中文\\name.txt"))
        self.assertIn("result", self.call("create_file", path=child))
        self.write(child, b"keep")
        self.assertEqual(self.call("create_file", path=child)["error"]["kind"], "conflict")
        self.assertEqual(base64.b64decode(self.call("read", path=child)["result"]["data"]), b"keep")
        os.symlink(child, os.path.join(self.directory.name, "link"))
        self.assertEqual(self.call("create_file", path="link")["error"]["kind"], "conflict")

    def test_relocate_reports_descendants_without_matching_sibling_prefixes(self):
        self.call("mkdir", path="old")
        self.call("mkdir", path="older")
        self.write("old/a", b"edited")
        self.write("older/a", b"separate")
        source = os.path.join(self.directory.name, "old/a")
        sibling = os.path.join(self.directory.name, "older/a")
        result = self.call("relocate", source="old", target="new", tracked=[source, sibling])["result"]
        self.assertEqual(result["entries"], [{"source": source, "target": os.path.join(self.directory.name, "new/a"), "name": "a"}])
        self.assertEqual(base64.b64decode(self.call("read", path="new/a")["result"]["data"]), b"edited")
        self.assertEqual(base64.b64decode(self.call("read", path="older/a")["result"]["data"]), b"separate")

    def test_relocate_refuses_open_identity_collision_before_mutation(self):
        self.call("mkdir", path="old")
        self.write("old/a", b"original")
        tracked = [os.path.join(self.directory.name, path) for path in ("old/a", "new/a")]
        self.assertEqual(self.call("relocate", source="old", target="new", tracked=tracked)["error"]["kind"], "conflict")
        self.assertTrue(os.path.exists(os.path.join(self.directory.name, "old/a")))
        self.assertFalse(os.path.exists(os.path.join(self.directory.name, "new")))

    def test_hangup_and_termination_clean_uncommitted_writes(self):
        for signum in (signal.SIGHUP, signal.SIGTERM):
            with self.subTest(signal=signum), subprocess.Popen(
                [sys.executable, "-u", os.path.join(os.path.dirname(__file__), "agent.py")],
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            ) as process:
                try:
                    requests = [
                        {"id": 1, "method": "mount", "params": {"path": self.directory.name}},
                        {"id": 2, "method": "write_begin", "params": {"root": 1, "path": "unfinished", "size": 10}},
                    ]
                    for request in requests:
                        process.stdin.write(json.dumps(request).encode() + b"\n")
                        process.stdin.flush()
                        self.assertIn("result", json.loads(process.stdout.readline()))
                    self.assertTrue(any(name.startswith(".loog-save-") for name in os.listdir(self.directory.name)))
                    process.send_signal(signum)
                    process.communicate(timeout=5)
                    self.assertEqual(process.returncode, 128 + signum)
                    self.assertEqual(os.listdir(self.directory.name), [])
                finally:
                    if process.poll() is None:
                        process.kill()
                        process.communicate(timeout=5)

    def test_existing_directory_creation_reports_conflict(self):
        self.assertIn("result", self.call("mkdir", path="existing"))
        self.assertEqual(self.call("mkdir", path="existing")["error"]["kind"], "conflict")

    def test_staged_save_keeps_original_until_commit_and_bounds_offsets(self):
        revision = self.write("staged", b"original")["result"]["revision"]
        chunk = b"abc" * 40000
        handle = self.call("write_begin", path="staged", size=len(chunk) * 3, expected_revision=revision)["result"]["write"]
        self.assertEqual(self.call("write_append", write=handle, offset=1, data="YQ==")["error"]["kind"], "invalid_request")
        for index in range(3):
            self.assertIn("result", self.call("write_append", write=handle, offset=index * len(chunk), data=base64.b64encode(chunk).decode()))
        self.assertEqual(base64.b64decode(self.call("read", path="staged")["result"]["data"]), b"original")
        committed = self.call("write_commit", write=handle)["result"]["revision"]
        self.assertNotEqual(committed, revision)
        with open(os.path.join(self.directory.name, "staged"), "rb") as stream:
            self.assertEqual(stream.read(), chunk * 3)
        self.assertFalse(self.agent.writes)
        self.assertFalse(any(name.startswith(".loog-save-") for name in os.listdir(self.directory.name)))

    def test_staged_conflict_abort_disconnect_and_expiry_remove_temporary_files(self):
        revision = self.write("staged", b"original")["result"]["revision"]
        handle = self.call("write_begin", path="staged", size=1, expected_revision=revision)["result"]["write"]
        self.call("write_append", write=handle, offset=0, data="YQ==")
        self.write("staged", b"external")
        self.assertEqual(self.call("write_commit", write=handle)["error"]["kind"], "conflict")
        self.assertFalse(self.agent.writes)
        handle = self.call("write_begin", path="staged", size=0)["result"]["write"]
        self.assertIn("result", self.call("write_abort", write=handle))
        handle = self.call("write_begin", path="staged", size=0)["result"]["write"]
        self.agent.writes[handle]["touched"] -= 121
        self.call("hello")
        self.assertFalse(self.agent.writes)
        self.call("write_begin", path="staged", size=0)
        self.agent.close()
        self.assertFalse(any(name.startswith(".loog-save-") for name in os.listdir(self.directory.name)))

    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="loog-agent-test-")
        self.addCleanup(self.directory.cleanup)
        self.agent = Agent()
        self.addCleanup(self.agent.close)
        self.root = self.agent.mount(self.directory.name)["root"]
        self.sequence = 0

    def call(self, method, **params):
        self.sequence += 1
        return self.agent.handle({"id": self.sequence, "method": method,
                                  "params": {"root": self.root, **params}})

    def write(self, name, content, **params):
        return self.call("write", path=name, data=base64.b64encode(content).decode(), **params)

    def test_bytes_encoding_and_paged_read(self):
        data = b"\xff\xfeA\x00\r\x00\n\x00"
        revision = self.write("中文 name.txt", data)["result"]["revision"]
        first = self.call("read", path="中文 name.txt", offset=0, limit=3)["result"]
        second = self.call("read", path="中文 name.txt", offset=3, limit=32)["result"]
        self.assertEqual(base64.b64decode(first["data"]) + base64.b64decode(second["data"]), data)
        self.assertFalse(first["eof"])
        self.assertTrue(second["eof"])
        self.assertEqual(self.call("stat", path="中文 name.txt")["result"]["revision"], revision)

    def test_non_utf8_names_fail_explicitly_without_lossy_identity_or_empty_success(self):
        name = os.fsencode(self.directory.name) + b"/non-utf8-\xff"
        with open(name, "wb") as stream:
            stream.write(b"preserved")
        response = self.call("list", path=".")
        self.assertEqual(response["error"]["kind"], "unsupported")
        self.assertIn("UTF-8", response["error"]["message"])
        self.assertNotIn("result", response)
        with open(name, "rb") as stream:
            self.assertEqual(stream.read(), b"preserved")

    def test_external_change_rejects_stale_save(self):
        revision = self.write("a", b"one")["result"]["revision"]
        self.write("a", b"external")
        result = self.write("a", b"stale", expected_revision=revision)
        self.assertEqual(result["error"]["kind"], "conflict")
        self.assertEqual(base64.b64decode(self.call("read", path="a")["result"]["data"]), b"external")

    def test_duplicate_mutation_is_idempotent_and_payload_reuse_is_rejected(self):
        request = {"id": 42, "method": "mkdir", "params": {"root": self.root, "path": "folder"}}
        first = self.agent.handle(request)
        self.assertIn("result", first)
        self.assertEqual(first, self.agent.handle(request))
        request["params"]["path"] = "different"
        self.assertEqual(self.agent.handle(request)["error"]["kind"], "conflict")
        self.assertFalse(os.path.exists(os.path.join(self.directory.name, "different")))

    def test_evicted_receipt_cannot_reexecute_mutation(self):
        for index in range(MAX_RECEIPTS + 1):
            self.call("mkdir", path=str(index))
        self.assertEqual(len(self.agent.receipts), MAX_RECEIPTS)
        result = self.agent.handle({"id": 1, "method": "mkdir", "params": {"root": self.root, "path": "changed"}})
        self.assertEqual(result["error"]["kind"], "unknown_outcome")
        self.assertFalse(os.path.exists(os.path.join(self.directory.name, "changed")))

    def test_workspace_escape_and_external_symlink_reads_are_refused(self):
        for path in ("../outside", "/etc/passwd"):
            self.assertEqual(self.call("read", path=path)["error"]["kind"], "permission_denied")
        os.symlink("/etc/passwd", os.path.join(self.directory.name, "link"))
        self.assertEqual(self.call("read", path="link")["error"]["kind"], "permission_denied")
        self.assertIn("result", self.call("remove", path="link"))

    def test_listing_has_stable_cursors_and_does_not_hash_file_contents(self):
        for name in ("a", "b", "c"):
            self.write(name, name.encode())
        self.agent.revision = lambda _: self.fail("directory listing hashed a file")
        first = self.call("list", path=".", limit=2)["result"]
        second = self.call("list", path=".", cursor=first["cursor"], limit=2)["result"]
        self.assertEqual([item["name"] for item in first["entries"] + second["entries"]], ["a", "b", "c"])
        self.assertIsNone(second["cursor"])

    def test_readonly_and_mounted_root_protection(self):
        self.write("a", b"original")
        target = os.path.join(self.directory.name, "a")
        os.chmod(target, 0o400)
        self.assertEqual(self.write("a", b"replace")["error"]["kind"], "permission_denied")
        os.chmod(target, 0o600)
        self.assertEqual(self.call("remove", path=self.directory.name)["error"]["kind"], "permission_denied")

    def test_rename_and_missing_are_distinct_from_success(self):
        self.write("a", b"a")
        self.write("b", b"b")
        self.assertEqual(self.call("rename", source="a", target="b")["error"]["kind"], "conflict")
        self.assertIn("result", self.call("rename", source="a", target="c"))
        self.assertEqual(self.call("read", path="a")["error"]["kind"], "not_found")

    def test_fifo_is_rejected_without_waiting_for_a_writer(self):
        os.mkfifo(os.path.join(self.directory.name, "pipe"))
        self.assertEqual(self.call("read", path="pipe")["error"]["kind"], "invalid_request")

    def test_internal_symlink_save_preserves_the_link(self):
        self.write("target", b"first")
        os.symlink("target", os.path.join(self.directory.name, "link"))
        self.assertIn("result", self.write("link", b"second"))
        self.assertTrue(os.path.islink(os.path.join(self.directory.name, "link")))
        self.assertEqual(base64.b64decode(self.call("read", path="target")["result"]["data"]), b"second")

    def test_directory_replaced_by_symlink_after_resolution_is_never_followed(self):
        os.mkdir(os.path.join(self.directory.name, "nested"))
        original = self.agent.path
        def swap(*args, **kwargs):
            result = original(*args, **kwargs)
            os.rmdir(os.path.join(self.directory.name, "nested"))
            os.symlink("/tmp", os.path.join(self.directory.name, "nested"))
            return result
        self.agent.path = swap
        self.assertIn("error", self.write("nested/escape.txt", b"no"))


if __name__ == "__main__":
    unittest.main()
