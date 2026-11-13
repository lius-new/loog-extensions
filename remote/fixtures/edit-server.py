#!/usr/bin/env python3
"""LSP acceptance fixture: two buffer edits followed by a rejected bad range.

Runs only in the isolated test directory; never writes project files itself.
The final diagnostic proves both server requests received their actual outcomes.
"""
import json
import sys


def send(value):
    body = json.dumps(value, ensure_ascii=False).encode("utf-8")
    sys.stdout.buffer.write(b"Content-Length: " + str(len(body)).encode() + b"\r\n\r\n" + body)
    sys.stdout.buffer.flush()


def read():
    length = None
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        if line in (b"\r\n", b"\n"):
            break
        if line.lower().startswith(b"content-length:"):
            length = int(line.split(b":", 1)[1])
    if length is None or not 0 <= length <= 8 * 1024 * 1024:
        raise ValueError("invalid frame")
    body = sys.stdin.buffer.read(length)
    return json.loads(body)


def insertion(text, character=0):
    position = {"line": 0, "character": character}
    return {"range": {"start": position, "end": position}, "newText": text}


source = None
documents = {}
requested = False
applied = False
rejected = False
bad_sent = False
command_verified = False
forbidden_command = False
settings_initial = False
settings_changed = False
settings_response = False
while True:
    message = read()
    if message is None:
        break
    method = message.get("method")
    params = message.get("params") or {}
    if method == "initialize":
        settings_initial = params.get("initializationOptions") == {"acceptanceSettings": "project-中文"}
        send({"jsonrpc": "2.0", "id": message["id"], "result": {"capabilities": {
            "positionEncoding": "utf-8", "textDocumentSync": 1,
            "codeActionProvider": {"resolveProvider": True},
            "executeCommandProvider": {"commands": ["fixture.after", "fixture.must-not-run", "fixture.fail"]},
            "documentOnTypeFormattingProvider": {"firstTriggerCharacter": ";"}}}})
    elif method == "initialized":
        send({"jsonrpc": "2.0", "id": "project-settings", "method": "workspace/configuration",
              "params": {"items": [{"section": "acceptance"}]}})
    elif method == "workspace/didChangeConfiguration":
        settings_changed = params.get("settings") == {"acceptance": {"marker": "project-中文"}}
    elif method == "textDocument/codeAction":
        helper = source.rsplit("/", 1)[0] + "/helper.rs"
        if "source.fixAll" in params.get("context", {}).get("only", []):
            send({"jsonrpc": "2.0", "id": message["id"], "result": [{
                "title": "Save resource action", "kind": "source.fixAll", "edit": {"changes": {
                    source: [insertion("// save action\n")], helper: [insertion("// save helper\n")]}}}]})
            continue
        send({"jsonrpc": "2.0", "id": message["id"], "result": [
            {"title": "Apply resource action and command", "kind": "quickfix", "data": {"action": "apply"}},
            {"title": "Reject invalid action atomically", "kind": "quickfix", "edit": {"changes": {
                source: [insertion("MUST NOT APPLY")], helper: [insertion("BAD", 999999)]}},
                "command": {"command": "fixture.must-not-run", "arguments": []}},
            {"title": "Fail resource command", "command": "fixture.fail", "arguments": []}]})
    elif method == "codeAction/resolve":
        helper = source.rsplit("/", 1)[0] + "/helper.rs"
        send({"jsonrpc": "2.0", "id": message["id"], "result": {
            "title": "Apply resource action and command", "kind": "quickfix", "edit": {"changes": {
                source: [insertion("// action edit\n")], helper: [insertion("// action helper\n")]}},
            "command": {"command": "fixture.after", "arguments": [source]}}})
    elif method == "workspace/executeCommand":
        if params.get("command") == "fixture.after" and documents[source][1].startswith("// action edit\n"):
            send({"jsonrpc": "2.0", "id": "command-edit", "method": "workspace/applyEdit",
                  "params": {"label": "Command edit", "edit": {"changes": {
                      source: [insertion("// command edit\n")]}}}})
            # Deliberately finish the RPC before its edit receipt. The client
            # must not complete the native command until that edit has settled.
            send({"jsonrpc": "2.0", "id": message["id"], "result": None})
        else:
            forbidden_command |= params.get("command") == "fixture.must-not-run"
            send({"jsonrpc": "2.0", "id": message["id"], "error": {
                "code": -32001, "message": "fixture command failure"}})
    elif method == "textDocument/onTypeFormatting":
        edits = [insertion("// type formatting\n")] if params.get("ch") == ";" else []
        send({"jsonrpc": "2.0", "id": message["id"], "result": edits})
    elif method == "shutdown":
        send({"jsonrpc": "2.0", "id": message["id"], "result": None})
    elif method == "exit":
        break
    elif method in ("textDocument/didOpen", "textDocument/didChange"):
        doc = params["textDocument"]
        uri = doc["uri"]
        documents[uri] = (doc["version"], doc.get("text") if method.endswith("didOpen")
                          else params["contentChanges"][0]["text"])
        if source is None:
            source = uri
        if uri == source and not requested:
            requested = True
            helper = source.rsplit("/", 1)[0] + "/helper.rs"
            send({"jsonrpc": "2.0", "id": "apply-two", "method": "workspace/applyEdit",
                  "params": {"label": "Acceptance server edit", "edit": {"changes": {
                      source: [insertion("// server edit\n")],
                      helper: [insertion("// helper edit\n")]}}}})
    elif message.get("id") == "apply-two":
        applied = message.get("result", {}).get("applied") is True
    elif message.get("id") == "reject-range":
        rejected = message.get("result", {}).get("applied") is False
    elif message.get("id") == "command-edit":
        command_verified = message.get("result", {}).get("applied") is True
    elif message.get("id") == "project-settings":
        settings_response = message.get("result") == [{"marker": "project-中文"}]
    if applied and source in documents and documents[source][1].startswith("// server edit\n") and not bad_sent:
        bad_sent = True
        send({"jsonrpc": "2.0", "id": "reject-range", "method": "workspace/applyEdit",
              "params": {"label": "Acceptance invalid range", "edit": {"changes": {
                  source: [insertion("MUST NOT APPLY", 999999)]}}}})
    if applied and rejected:
        version, _ = documents[source]
        send({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {
            "uri": source, "version": version, "diagnostics": [{"range": {
                "start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
                "severity": 3, "message": "Project settings mismatch" if not (
                    settings_initial and settings_changed and settings_response) else "Forbidden command ran" if forbidden_command else (
                    "Action command receipt verified" if command_verified else "Both edit receipts verified")}]}})
