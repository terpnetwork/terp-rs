#!/usr/bin/env python3
"""Loopback signer for the lab claim page.

Binds 127.0.0.1:19821. Broadcasts ProcessHeadstash with the container key
name from endpoints.json. Does not read or print a mnemonic, keyring file,
or environment secret.
"""

import json
import os
import subprocess
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

HOST = "127.0.0.1"
PORT = 19821
DIR = os.environ.get("HEADSTASH_E2E_DIR", "/tmp/headstash-e2e")
PAGE_ORIGIN = "http://127.0.0.1:19820"


def endpoints():
    with open(os.path.join(DIR, "endpoints.json"), encoding="utf-8") as handle:
        doc = json.load(handle)
    for key in ("lcd", "rpc"):
        value = str(doc.get(key) or "")
        if not value.startswith("http://127.0.0.1:"):
            raise SystemExit(f"refusing signer: {key} is not loopback")
    contract = str(doc.get("contract") or "")
    if not contract.startswith("terp1"):
        raise SystemExit("refusing signer: contract is not a terp bech32 address")
    chain_id = str(doc.get("chain_id") or "")
    container = str(doc.get("container") or "")
    home = str(doc.get("home") or "")
    if chain_id != "headstash-a-1" or not container or not home.startswith("/"):
        raise SystemExit("refusing signer: chain, container, or home is missing")
    return doc


DOC = endpoints()


class Handler(BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):
        sys.stderr.write("signer %s\n" % (fmt % args))

    def do_OPTIONS(self):
        # The page is on :19820 and this signer is on :19821, so Chromium
        # sends a preflight. 501 here is the "Failed to fetch" click failure.
        if self.path != "/broadcast":
            self._send(404, {"error": "not found"})
            return
        self.send_response(204)
        self._cors()
        self.send_header("access-control-allow-methods", "POST, OPTIONS")
        self.send_header("access-control-allow-headers", "content-type")
        self.send_header("content-length", "0")
        self.end_headers()

    def do_POST(self):
        if self.path != "/broadcast":
            self._send(404, {"error": "not found"})
            return
        length = int(self.headers.get("content-length") or "0")
        if length <= 0 or length > 2_000_000:
            self._send(400, {"error": "body size"})
            return
        try:
            payload = json.loads(self.rfile.read(length))
        except json.JSONDecodeError:
            self._send(400, {"error": "json"})
            return
        contract = str(payload.get("contract") or "")
        msg = payload.get("msg")
        if contract != DOC["contract"] or not isinstance(msg, dict) or "process_headstash" not in msg:
            self._send(400, {"error": "only ProcessHeadstash for the harness contract"})
            return
        result = broadcast(contract, msg)
        self._send(200 if "txhash" in result else 502, result)

    def _cors(self):
        self.send_header("access-control-allow-origin", PAGE_ORIGIN)
        self.send_header("vary", "origin")

    def _send(self, status, body):
        raw = json.dumps(body).encode()
        self.send_response(status)
        self._cors()
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)


def broadcast(contract, msg):
    home = DOC["home"]
    chain_id = DOC["chain_id"]
    container = DOC["container"]
    key = str(DOC.get("key_name") or "deployer")
    msg_text = json.dumps(msg, separators=(",", ":"))
    tx = [
        "terpd",
        "tx",
        "wasm",
        "execute",
        contract,
        msg_text,
        "--from",
        key,
        "--keyring-backend",
        "test",
        "--home",
        home,
        "--chain-id",
        chain_id,
        "--node",
        "tcp://127.0.0.1:26657",
        "--gas",
        "20000000",
        "--gas-prices",
        "0uterp",
        "--broadcast-mode",
        "sync",
        "--yes",
        "--output",
        "json",
    ]
    env = os.environ.copy()
    env["DOCKER_HOST"] = env.get(
        "DOCKER_HOST", "unix:///Users/returniflost/.docker/run/docker.sock"
    )
    run = subprocess.run(
        ["docker", "exec", container, *tx],
        capture_output=True,
        text=True,
        timeout=180,
        env=env,
        check=False,
    )
    txhash = ""
    try:
        txhash = json.loads(run.stdout or "{}").get("txhash") or ""
    except json.JSONDecodeError:
        txhash = ""
    queried = {}
    if txhash:
        query = subprocess.run(
            [
                "docker",
                "exec",
                container,
                "terpd",
                "query",
                "tx",
                txhash,
                "--home",
                home,
                "--output",
                "json",
            ],
            capture_output=True,
            text=True,
            timeout=60,
            env=env,
            check=False,
        )
        try:
            queried = json.loads(query.stdout or "{}")
        except json.JSONDecodeError:
            queried = {}
    raw_log = str(queried.get("raw_log") or "")
    if len(raw_log) > 400:
        raw_log = raw_log[:400]
    code = queried.get("code")
    if not txhash:
        err = (run.stderr or run.stdout or "no txhash").replace("\n", " ")
        return {"error": err[:400]}
    return {"txhash": txhash, "code": code, "raw_log": raw_log}


if __name__ == "__main__":
    server = ThreadingHTTPServer((HOST, PORT), Handler)
    sys.stderr.write(f"signer http://{HOST}:{PORT}/broadcast\n")
    server.serve_forever()
