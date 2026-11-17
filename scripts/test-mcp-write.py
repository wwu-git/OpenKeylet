#!/usr/bin/env python3
"""Isolated stdio/HTTP acceptance using synthetic values, never the owner's vault."""
import base64
import json
import os
from pathlib import Path
import select
import subprocess
import sys
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

binary = Path(sys.argv[1] if len(sys.argv) > 1 else "src-tauri/target/debug/openkeylet").resolve()
seen = []
class Endpoint(BaseHTTPRequestHandler):
    def do_GET(self):
        seen.append({name.lower():value for name,value in self.headers.items()})
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b'{"ok":true}')
    def log_message(self, *_):
        pass
server = ThreadingHTTPServer(("127.0.0.1", 0), Endpoint)
threading.Thread(target=server.serve_forever, daemon=True).start()
try:
    with tempfile.TemporaryDirectory(prefix="openkeylet-write-stdio-") as directory:
        env = dict(os.environ, OPENKEYLET_DATA_DIR=directory, NO_PROXY="127.0.0.1,localhost")
        for name in list(env):
            if name.lower() in ("http_proxy", "https_proxy", "all_proxy"):
                env.pop(name)
        process = subprocess.Popen([str(binary), "mcp", "stdio"], env=env, stdin=subprocess.PIPE,
                                   stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        counter = 0
        def rpc(method, params):
            global counter
            counter += 1
            process.stdin.write(json.dumps({"jsonrpc":"2.0","id":counter,"method":method,"params":params}) + "\n")
            process.stdin.flush()
            while True:
                if not select.select([process.stdout], [], [], 15)[0]:
                    raise AssertionError("MCP response timed out")
                line = process.stdout.readline()
                assert line, "MCP exited unexpectedly"
                response = json.loads(line)
                if response.get("id") == counter:
                    return response
        def tool(name, arguments):
            response = rpc("tools/call", {"name":name,"arguments":arguments})
            assert "error" not in response, f"Unexpected RPC error for {name}"
            return response["result"]
        def data(result):
            assert not result.get("isError"), "Tool unexpectedly rejected the operation"
            return result["structuredContent"]
        try:
            rpc("initialize", {"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"isolated-write-test","version":"1"}})
            process.stdin.write('{"jsonrpc":"2.0","method":"notifications/initialized"}\n')
            process.stdin.flush()
            names = {t["name"] for t in rpc("tools/list", {})["result"]["tools"]}
            assert {"vault_write_capabilities", "items_create", "items_update"} <= names
            assert not data(tool("vault_write_capabilities", {}))["enabled"]
            payload = {"name":"synthetic-api","folder":"Agent","itemType":"api","httpAuthType":"bearer",
                       "modules":[{"kind":"url","value":f"http://127.0.0.1:{server.server_port}/v1"},
                                  {"kind":"apiCredential","value":"synthetic-token"}]}
            assert tool("items_create", payload).get("isError")
            path = Path(directory) / "vault.json"
            def permissions(*args):
                result = subprocess.run([str(binary), "permissions", *args], env=env,
                                        capture_output=True, text=True, timeout=15, check=True)
                return json.loads(result.stdout)
            permissions("folder", "add", "Agent")
            assert permissions("enable")["effective"]
            assert tool("items_create", dict(payload, folder="Agent/child")).get("isError")
            created_result = tool("items_create", payload)
            created = data(created_result)
            assert "synthetic-token" not in json.dumps(created_result)
            listed = data(tool("items_search", {"query":"synthetic-api"}))["items"][0]
            credential = next(m for m in listed["modules"] if m["kind"] == "apiCredential")
            assert "value" not in credential
            data(tool("http_send", {"item":"synthetic-api","timeoutSeconds":5}))
            assert seen[-1]["authorization"] == "Bearer synthetic-token"
            patch = {"id":created["id"],"expectedRevision":created["revision"],
                     "modules":[{"id":credential["id"],"value":"replacement-token"}]}
            updated = data(tool("items_update", patch))
            assert updated["revision"] == created["revision"] + 1
            data(tool("http_send", {"item":"synthetic-api","timeoutSeconds":5}))
            assert seen[-1]["authorization"] == "Bearer replacement-token"
            for mode in ("apiKey", "basic"):
                extra = dict(payload, name="synthetic-" + mode, httpAuthType=mode, authHeader="X-API-Key")
                extra["modules"] = [payload["modules"][0], {"kind":"apiCredential","value":"synthetic-api-key"},
                                    {"kind":"username","value":"test-user"},{"kind":"password","value":"test-password"}]
                data(tool("items_create", extra))
                data(tool("http_send", {"item":extra["name"],"timeoutSeconds":5}))
                if mode == "apiKey":
                    assert seen[-1]["x-api-key"] == "synthetic-api-key"
                else:
                    expected = "Basic " + base64.b64encode(b"test-user:test-password").decode()
                    assert seen[-1]["authorization"] == expected
            assert tool("items_update", patch).get("isError")
            bad = {"id":created["id"],"expectedRevision":updated["revision"],"folder":"Personal"}
            before = path.read_bytes()
            assert tool("items_update", bad).get("isError")
            assert path.read_bytes() == before
            malformed = rpc("tools/call", {"name":"items_create","arguments":dict(payload, modules="NEVER-ECHO-THIS")})
            assert "NEVER-ECHO-THIS" not in json.dumps(malformed)
            assert not permissions("folder", "remove", "Agent")["effective"]
            assert tool("items_update", {"id":created["id"],"expectedRevision":updated["revision"],"description":"denied"}).get("isError")
            assert not data(tool("vault_write_capabilities", {}))["enabled"]
            unchanged = data(tool("items_search", {"query":"synthetic-api"}))["items"][0]
            assert unchanged["revision"] == updated["revision"]
            assert "replacement-token" not in path.read_text()
            print("PASS: real MCP stdio discovery, default deny, create/update, hidden search, HTTP Bearer/API Key/Basic use and token replacement, version conflict, move denial, rollback, sanitized schema errors, immediate revocation, encrypted persistence")
        finally:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
finally:
    server.shutdown()
    server.server_close()
