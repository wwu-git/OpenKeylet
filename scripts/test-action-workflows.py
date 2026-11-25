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
        self.rpc("initialize", {
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
        print("PASS: MCP selection, discovery purity, stable ID after rename/session switch, exact authenticated HTTP dispatch")
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
