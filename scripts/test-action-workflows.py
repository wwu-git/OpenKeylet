#!/usr/bin/env python3
"""Verify OpenKeylet's workflow contract over real MCP stdio with a disposable vault."""
import json
import os
import select
import subprocess
import sys
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


class Endpoint(BaseHTTPRequestHandler):
    def do_GET(self):
        token = self.headers.get("Authorization")
        self.server.requests.append(token)
        body = json.dumps({"authorizedAccount": token}).encode()
        status = 403 if self.path == "/denied" else 200
        content_type = "application/json"
        if self.path == "/text":
            body = b"fixture response"
            content_type = "text/plain"
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        if status == 403:
            self.send_header("X-Request-Id", "fixture-denial")
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_):
        pass


class Session:
    def __init__(self, binary, env):
        self.process = subprocess.Popen(
            [binary, "mcp", "stdio"], env=env, stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, text=True,
        )
        self.request_id = 0
        self.info = self.rpc("initialize", {
            "protocolVersion": "2025-03-26", "capabilities": {},
            "clientInfo": {"name": "action-workflow-test", "version": "1"},
        })
        self.send({"method": "notifications/initialized"})

    def send(self, message):
        self.process.stdin.write(json.dumps({"jsonrpc": "2.0", **message}) + "\n")
        self.process.stdin.flush()

    def rpc(self, method, params):
        self.request_id += 1
        self.send({"id": self.request_id, "method": method, "params": params})
        assert select.select([self.process.stdout], [], [], 15)[0], f"{method} timed out"
        response = json.loads(self.process.stdout.readline())
        assert response.get("id") == self.request_id and "result" in response, response
        return response["result"]

    def tool(self, tool_name, **arguments):
        return self.rpc("tools/call", {"name": tool_name, "arguments": arguments})

    def data(self, tool_name, **arguments):
        result = self.tool(tool_name, **arguments)
        assert not result.get("isError"), result
        return result["structuredContent"]

    def close(self):
        self.process.terminate()
        self.process.wait(timeout=10)


def verify(binary, directory, address, requests):
    env = dict(os.environ, OPENKEYLET_DATA_DIR=directory, NO_PROXY="127.0.0.1,localhost")
    for name in list(env):
        if name.lower() in {"http_proxy", "https_proxy", "all_proxy"}:
            del env[name]

    for args in [("folder", "add", "Workflow"), ("enable",)]:
        subprocess.run([binary, "permissions", *args], env=env, check=True,
                       capture_output=True, timeout=15)
    session = Session(binary, env)
    try:
        tools = session.rpc("tools/list", {})["tools"]
        action_tool = next(tool for tool in tools if tool["name"] == "action_capabilities")
        assert action_tool["inputSchema"]["type"] == "object"
        ssh_tool = next(tool for tool in tools if tool["name"] == "ssh_run")
        for guidance in [session.info["instructions"], ssh_tool["description"]]:
            assert "sudo -S -p '' -- ls" in guidance
            assert r"{{openkeylet:password}}\n" in guidance
            assert r"{{openkeylet:sudoPassword}}\n" in guidance
            assert "SSH password authentication" in guidance
            assert "SSH private-key authentication" in guidance
        print("PASS: MCP initialization and ssh_run describe both SSH sudo flows")

        def select_project(**selector):
            return session.data("action_capabilities", action="selectProject",
                                capability="http", **selector)

        def create(name):
            return session.data("items_create", name=name, folder="Workflow",
                                itemType="api", httpAuthType="bearer", modules=[
                {"kind": "url", "value": address},
                {"kind": "apiCredential", "value": f"fixture-token-{name}"},
            ])

        assert select_project()["status"] == "unavailable"
        first = create("Cloud One")
        unique = select_project()
        assert unique["status"] == "selected" and unique["project"]["id"] == first["id"]
        assert unique["project"]["selection"] == "unique"

        second = create("Cloud Two")
        ambiguous = select_project()
        assert ambiguous["status"] == "chooseProject"
        assert {item["id"] for item in ambiguous["candidates"]} == {first["id"], second["id"]}
        assert select_project(item="Cloud")["status"] == "chooseProject"
        assert select_project(item="Unknown")["status"] == "unavailable"
        explicit = select_project(item="Cloud One")
        assert explicit["project"]["id"] == first["id"]
        assert explicit["project"]["selection"] in {"explicit", "explicitName"}
        assert select_project()["status"] == "chooseProject"

        session.data("items_search", query="Cloud Two")
        active = select_project()
        assert active["project"]["id"] == second["id"] and active["project"]["selection"] == "session"

        requested_id = select_project(itemId=first["id"])["project"]["id"]
        session.data("items_update", id=first["id"], expectedRevision=first["revision"],
                     name="Renamed Cloud")
        before = len(requests)
        assert session.tool("http_send", itemId="71fa9bc2-1f81-4b67-ae88-2420c983b46e").get("isError")
        assert session.tool("http_send", itemId=requested_id, item="Cloud Two").get("isError")
        assert len(requests) == before
        response = session.data("http_send", itemId=requested_id)
        assert response["status"] == 200 and requests[-1] == "Bearer fixture-token-Cloud One"
        assert "fixture-token" not in json.dumps(response)

        planned = session.data("action_capabilities", action="httpRequest",
                               itemId=requested_id, method="GET")
        assert planned["status"] == "ready"
        assert planned["project"]["id"] == requested_id
        call = planned["nextCall"]
        assert call["tool"] == "http_send" and call["arguments"]["itemId"] == requested_id
        assert call["arguments"]["responseFormat"] == "structured"
        structured = session.tool(call["tool"], **call["arguments"])
        assert structured["structuredContent"]["status"] == 200
        assert isinstance(structured["structuredContent"]["data"], dict)
        assert "body" not in structured["structuredContent"]
        assert "fixture-token" not in json.dumps(structured)
        assert len(json.dumps(structured["content"])) < len(json.dumps(structured["structuredContent"]))

        text = session.data("http_send", itemId=requested_id, url="/text",
                            responseFormat="structured")
        assert text["body"] == "fixture response" and "data" not in text
        brief = session.data("http_send", itemId=requested_id, maxResponseBytes=2,
                             responseFormat="structured")
        assert brief["truncated"] and "body" in brief and "data" not in brief
        denied = session.tool("http_send", itemId=requested_id, url="/denied",
                              responseFormat="structured")["structuredContent"]
        assert denied["status"] == 403 and denied["error"]["code"] == "remoteHttpError"
        assert denied["error"]["requestId"] == "fixture-denial"
        assert "fixture-token" not in json.dumps(denied)

        before = len(requests)
        runtime_url = session.data("items_create", name="Runtime URL", folder="Workflow",
                                   itemType="custom", modules=[
            {"kind": "customSecret", "name": "Value", "value": "synthetic-runtime-token"},
        ])
        no_url = session.data("action_capabilities", action="httpRequest",
                              itemId=runtime_url["id"], method="GET")
        assert no_url["status"] == "needsInput" and no_url["missingInputs"][0]["name"] == "url"
        bad_reference = session.data("action_capabilities", action="httpRequest",
                                     itemId=requested_id, method="GET",
                                     query={"probe": "{{openkeylet:missing-value}}"})
        assert bad_reference["status"] == "needsInput" and "nextCall" not in bad_reference
        assert len(requests) == before

        escaped_token = 'fixture-escaped-"quoted"'
        escaped_project = session.data("items_create", name="Escaped JSON",
                                        folder="Workflow", itemType="api",
                                        httpAuthType="bearer", modules=[
            {"kind": "url", "value": address},
            {"kind": "apiCredential", "value": escaped_token},
        ])
        escaped = session.data("http_send", itemId=escaped_project["id"],
                               responseFormat="structured")
        assert requests[-1] == f"Bearer {escaped_token}"
        assert "fixture-escaped-" not in json.dumps(escaped)

        uuid_named = create(requested_id)
        assert select_project(item=requested_id)["project"]["id"] == uuid_named["id"]
        assert select_project(itemId=requested_id)["project"]["id"] == requested_id
        renamed = session.data("items_search", query="Renamed Cloud")["items"][0]
        session.data("items_update", id=requested_id, expectedRevision=renamed["revision"],
                     enabled=False)
        before = len(requests)
        assert select_project(itemId=requested_id)["status"] == "unavailable"
        assert session.tool("http_send", itemId=requested_id).get("isError")
        assert len(requests) == before
        print("PASS: MCP selection, stable project IDs, HTTP dispatch, structured data and error redaction")

        for name, service_url, operation, inputs in [
            ("GitHub", "https://api.github.com", "githubCommits",
             {"owner": "fixture-owner", "repo": "fixture-repo"}),
            ("Cloudflare", "https://api.cloudflare.com/client/v4", "cloudflareDnsRecords",
             {"zoneName": "fixture.invalid"}),
            ("DigitalOcean", "https://api.digitalocean.com/v2", "digitalOceanDroplets", {}),
            ("Hetzner", "https://api.hetzner.cloud/v1", "hetznerServers", {}),
        ]:
            project = session.data("items_create", name=name, folder="Workflow",
                                   itemType="api", httpAuthType="bearer", modules=[
                {"kind": "url", "value": service_url},
                {"kind": "apiCredential", "value": "synthetic-native-token"},
            ])
            session.data("items_search", query="Cloud Two")
            workflow = {"operation": operation, **inputs}
            preview = session.data("action_capabilities", action="nativeApi", workflow=workflow)
            assert preview["status"] == "ready" and preview["project"]["id"] == project["id"], preview
            call = preview["nextCall"]
            assert call["tool"] == "http_send" and call["arguments"]["itemId"] == project["id"]
            assert call["arguments"]["workflow"]["operation"] == operation
            assert "synthetic-native-token" not in json.dumps(preview)
            assert session.tool("http_send", **{**call["arguments"], "itemId": second["id"]}).get("isError")
            assert session.tool("http_send", **{**call["arguments"], "url": address}).get("isError")
            if operation == "cloudflareDnsRecords":
                assert preview["lookup"]["nextInput"] == "zoneId" and preview["lookup"]["collectAllPages"]
            if operation == "githubCommits":
                incomplete = session.data("action_capabilities", action="nativeApi",
                                          workflow={"operation": operation})
                assert incomplete["status"] == "needsInput" and incomplete["missingInputs"][0]["name"] == "owner"
        assert len(requests) == before
        print("PASS: native MCP planning preserves provider and project, detects missing input, rejects overridden targets")

        def plan(action, **arguments):
            return session.data("action_capabilities", action=action, **arguments)

        def follow(preview):
            assert preview["status"] == "ready", preview
            call = preview["nextCall"]
            return session.data(call["tool"], **call["arguments"])

        bindings = []
        for variable, value in [("FIXTURE_ALPHA", "alpha-value"), ("FIXTURE_BETA", "beta-value")]:
            project = session.data("items_create", name=variable, folder="Workflow",
                                   itemType="custom", modules=[
                {"kind": "customSecret", "name": "Value", "value": value},
            ])
            item = session.data("items_search", query=variable)["items"][0]
            bindings.append(variable + "={{openkeylet:" + project["id"] + "/" + item["modules"][0]["id"] + "}}")
        template = "\n".join(bindings)
        preview = plan("terminalRun", request={
            "command": 'test "$FIXTURE_ALPHA" = alpha-value && test "$FIXTURE_BETA" = beta-value && printf "%s" "$FIXTURE_ALPHA"',
            "envTemplate": template, "timeoutSeconds": 5,
        })
        result = follow(preview)
        assert result["exitCode"] == 0 and result["stdout"] == "[REDACTED]"
        invalid = plan("terminalRun", request={"command": "true", "envTemplate": "NAME=raw-value"})
        assert invalid["code"] == "invalidTemplate" and "nextCall" not in invalid

        missing_terminal = plan("terminalSession", request={"step": "read"})
        assert missing_terminal["code"] == "terminalUnavailable"
        terminal_id = follow(plan("terminalOpen", request={"program": "/bin/sh"}))["sessionId"]
        try:
            def terminal(step, **arguments):
                preview = plan("terminalSession", sessionId=terminal_id,
                               request={"step": step, **arguments})
                assert preview["target"]["sessionId"] == terminal_id
                return follow(preview)

            totp = session.data("items_create", name="Fixture 2FA", folder="Workflow",
                                itemType="custom", modules=[
                {"kind": "totp", "value": "JBSWY3DPEHPK3PXP"},
            ])
            unavailable = plan("credentialFill", itemId=totp["id"], target="browser")
            assert unavailable["code"] == "browserUnavailable" and "nextCall" not in unavailable
            terminal("write",
                     text=r"""printf '%s%s\n' 'fill-' 'ready'; IFS= read -r code; case "$code" in [0-9][0-9][0-9][0-9][0-9][0-9]) printf '%s%s\n' 'fill-' 'ok';; esac""",
                     submit=True)
            assert "fill-ready" in terminal("read", until="fill-ready", waitSeconds=5)["output"]
            result = follow(plan("credentialFill", itemId=totp["id"], module="totp",
                                 target="terminal", sessionId=terminal_id, submit=True))
            assert result["project"]["id"] == totp["id"] and result["target"] == "terminal"
            assert "fill-ok" in terminal("read", until="fill-ok", waitSeconds=5)["output"]
        finally:
            terminal("stop")
        assert plan("terminalSession", sessionId=terminal_id, request={"step": "read"})["code"] == "terminalUnavailable"

        write_preview = plan("vaultWrite", operation="update", itemId=second["id"])
        assert write_preview["target"]["writeTool"] == "items_update"
        assert write_preview["target"]["folder"] == "Workflow"
        assert write_preview["target"]["expectedRevision"] == second["revision"]
        assert follow(write_preview)["enabled"]
        denied = plan("vaultWrite", operation="create", folder="Unauthorized")
        assert denied["code"] == "writePermissionDenied" and "nextCall" not in denied
        subprocess.run([binary, "permissions", "disable"], env=env, check=True,
                       capture_output=True, timeout=15)
        denied = plan("vaultWrite", operation="update", itemId=second["id"])
        assert denied["code"] == "writePermissionDenied" and "nextCall" not in denied
        for workflow in ["envImport", "passwordVaultImport", "twoFactorImageImport",
                         "twoFactorBrowserImport", "githubBackupRestore"]:
            desktop = plan("desktop", workflow=workflow)
            assert desktop["status"] == "desktopAction" and desktop["desktop"]["steps"]
            assert "nextCall" not in desktop
        print("PASS: local env template, PTY lifecycle, TOTP fill, vault grants and desktop workflow steps")
    finally:
        session.close()


if __name__ == "__main__":
    binary = str(Path(sys.argv[1]).resolve())
    with ThreadingHTTPServer(("127.0.0.1", 0), Endpoint) as server:
        server.requests = []
        threading.Thread(target=server.serve_forever, daemon=True).start()
        try:
            with tempfile.TemporaryDirectory(prefix="openkeylet-workflow-") as directory:
                verify(binary, directory, f"http://127.0.0.1:{server.server_port}", server.requests)
        finally:
            server.shutdown()
