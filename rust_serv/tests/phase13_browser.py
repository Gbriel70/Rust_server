#!/usr/bin/env python3
"""Exercise the native browser WebSocket API through the shipped demo UI.

Requires Google Chrome and Python websocket-client. Uses local servers and an
isolated temporary Chrome profile. Build the debug binary before running.
"""
import json
import pathlib
import select
import subprocess
import sys
import tempfile
import time
import urllib.request

import websocket

ROOT = pathlib.Path(__file__).resolve().parents[1]
OUT = ROOT / "validation" / "phase13"


class DevTools:
    def __init__(self, url):
        self.socket = websocket.create_connection(url, timeout=10)
        self.sequence = 0
        self.events = []

    def call(self, method, params=None):
        self.sequence += 1
        ident = self.sequence
        self.socket.send(json.dumps({"id": ident, "method": method, "params": params or {}}))
        while True:
            response = json.loads(self.socket.recv())
            if response.get("method", "").startswith("Network.webSocket"):
                self.events.append(response)
            if response.get("id") == ident:
                if "error" in response:
                    raise RuntimeError(response["error"])
                return response.get("result", {})

    def evaluate(self, expression):
        result = self.call("Runtime.evaluate", {"expression": expression, "returnByValue": True})
        if "exceptionDetails" in result:
            raise RuntimeError(result["exceptionDetails"])
        return result["result"].get("value")

    def wait(self, expression):
        deadline = time.monotonic() + 10
        while not self.evaluate(expression):
            if time.monotonic() >= deadline:
                raise TimeoutError(expression)
            time.sleep(0.05)


def stop(process):
    process.terminate()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=5)


def exercise(browser, mode, extra):
    with (OUT / f"server-{mode}-{'-'.join(extra) or 'default'}.log").open("w") as log:
        server = subprocess.Popen(
            [str(ROOT / "target/debug/rust_serv"), "--root", str(ROOT / "public"),
             "--addr", "127.0.0.1:0", "--mode", mode, "--idle-timeout", "2", *extra],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=log, text=True,
        )
        try:
            if not select.select([server.stdout], [], [], 10)[0]:
                raise TimeoutError("server startup")
            announcement = server.stdout.readline()
            if not announcement.startswith("Listening on "):
                raise RuntimeError(announcement)
            address = announcement.split()[2]
            browser.events.clear()
            page_url = f"http://{address}/websocket.html"
            browser.call("Page.navigate", {"url": page_url})
            browser.wait(f"location.href === {json.dumps(page_url)} && document.readyState === 'complete' && Boolean(document.getElementById('connect'))")
            browser.evaluate("document.getElementById('connect').click()")
            browser.wait("document.getElementById('status').textContent === 'Conectado'")
            browser.evaluate("document.getElementById('message').value = 'Olá €'; document.getElementById('send').click(); document.getElementById('binary').click()")
            browser.wait("document.getElementById('log').textContent.includes('Recebido · Olá €') && document.getElementById('log').textContent.includes('Bytes recebidos · 0, 1, 2, 255')")
            browser.evaluate("document.getElementById('close').click()")
            browser.wait("document.getElementById('log').textContent.includes('Fechamento · 1000 · concluído')")
            handshakes = [event["params"]["response"] for event in browser.events
                          if event["method"] == "Network.webSocketHandshakeResponseReceived"]
            assert len(handshakes) == 1 and handshakes[0]["status"] == 101, handshakes
            result = {"mode": mode, "options": extra, "handshake": handshakes[0],
                      "log": browser.evaluate("document.getElementById('log').textContent")}
            server.stdin.write("\n")
            server.stdin.flush()
            assert server.wait(timeout=10) == 0
            return result
        finally:
            if server.poll() is None:
                stop(server)
            server.stdin.close()
            server.stdout.close()


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="rust-websocket-chrome-") as profile:
        with (OUT / "chrome.log").open("w") as log:
            chrome = subprocess.Popen(
                ["google-chrome", "--headless=new", "--no-sandbox", "--disable-gpu",
                 "--disable-background-networking", "--disable-component-update",
                 "--remote-allow-origins=*", "--remote-debugging-port=0",
                 f"--user-data-dir={profile}", "about:blank"], stdout=log, stderr=log,
            )
            browser = None
            try:
                active = pathlib.Path(profile) / "DevToolsActivePort"
                deadline = time.monotonic() + 15
                while not active.exists():
                    if chrome.poll() is not None or time.monotonic() >= deadline:
                        raise RuntimeError("Chrome startup failed; see validation/phase13/chrome.log")
                    time.sleep(0.1)
                port = int(active.read_text().splitlines()[0])
                with urllib.request.urlopen(f"http://127.0.0.1:{port}/json", timeout=5) as response:
                    pages = json.load(response)
                browser = DevTools(next(page["webSocketDebuggerUrl"] for page in pages if page["type"] == "page"))
                for method in ["Page.enable", "Runtime.enable", "Network.enable"]:
                    browser.call(method)
                cases = [("thread", []), ("pool", []), ("tokio", [])]
                if sys.platform.startswith("linux"):
                    cases += [("epoll", ["--trigger", "level"]), ("epoll", ["--trigger", "edge"])]
                results = [exercise(browser, mode, options) for mode, options in cases]
                (OUT / "browser.json").write_text(json.dumps(results, indent=2, ensure_ascii=False) + "\n")
                print(json.dumps(results, indent=2, ensure_ascii=False))
            finally:
                if browser:
                    try:
                        browser.call("Browser.close")
                    except websocket.WebSocketException:
                        pass
                    browser.socket.close()
                try:
                    chrome.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    stop(chrome)


if __name__ == "__main__":
    main()
