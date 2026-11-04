#!/usr/bin/env python3
"""Exercise the generated configuration on an isolated core and local HTTP fixtures.

Usage: python3 script/check_modes.py --fixtures /tmp/smartclash-mode-validation
The opt-in TrafficModeTests upstream test writes the converted provider fixtures.
No system proxy, TUN, installed application, or production controller is changed.
"""
import argparse
import copy
import http.server
import json
import pathlib
import socket
import subprocess
import tempfile
import threading
import time
import urllib.request


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_CONNECT(self):
        self.send_response(200)
        self.end_headers()
        self.handle_one_request()

    def do_GET(self):
        if hasattr(self.server, "payloads"):
            body = self.server.payloads.get(self.path.rsplit("/", 1)[-1])
            if body is None:
                self.send_error(404)
                return
        else:
            body = self.server.marker.encode()
        self.send_response(200)
        self.send_header("Content-Length", str(1_000_000 if self.path == "/hold" else len(body)))
        self.end_headers()
        try:
            self.wfile.write(body)
            self.wfile.flush()
            if self.path == "/hold":
                for _ in range(50):
                    time.sleep(0.1)
                    self.wfile.write(b".")
                    self.wfile.flush()
        except (BrokenPipeError, ConnectionResetError):
            pass


def server(**attributes):
    result = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    for key, value in attributes.items():
        setattr(result, key, value)
    threading.Thread(target=result.serve_forever, daemon=True).start()
    return result


def unused_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixtures", type=pathlib.Path, required=True)
    args = parser.parse_args()
    root = pathlib.Path(__file__).resolve().parent.parent
    payloads = {p.name: p.read_bytes() for p in args.fixtures.glob("sr-*.yaml")}
    assert len(payloads) == 6, "Run the upstream fixture conversion test first"
    for filename, domain in [("sr-proxy-domain.yaml", "proxy-mode.test"), ("sr-reject-domain.yaml", "reject-mode.test")]:
        payloads[filename] += f'  - "{domain}"\n'.encode()
    payloads["sr-proxy-domain.yaml"] += b'  - "+.suffix-mode.test"\n'
    payloads["sr-proxy-classical.yaml"] += b'  - "DOMAIN-KEYWORD,keyword-mode"\n  - "IP-CIDR,203.0.113.0/24,no-resolve"\n'
    rules = server(payloads=payloads)
    target = server(marker="DIRECT")
    proxy_a, proxy_b = server(marker="PROXY-A"), server(marker="PROXY-B")
    ports = [unused_port() for _ in range(4)]
    proxy_port, probe_port, control_port, dns_port = ports
    config = json.loads((args.fixtures / "generated-config.json").read_bytes())
    config.update({"mixed-port": proxy_port, "external-controller": f"127.0.0.1:{control_port}", "secret": "isolated-test",
                   "hosts": {"proxy-mode.test": "203.0.113.10", "reject-mode.test": "203.0.113.11", "hot-reject.test": "203.0.113.12"}})
    config["dns"]["listen"] = f"127.0.0.1:{dns_port}"
    config["listeners"][0]["port"] = probe_port
    config["proxies"] = [{"name": name, "type": "http", "server": "127.0.0.1", "port": instance.server_port}
                         for name, instance in [("A", proxy_a), ("B", proxy_b)]]
    for group in config["proxy-groups"]:
        if group["name"] in ["SMART", "PROBE"]:
            group["proxies"] = ["A", "B"]
    for name, provider in config["rule-providers"].items():
        provider["url"] = f"http://127.0.0.1:{rules.server_port}/{name}.yaml"
        assert provider["interval"] == 3600

    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def api(path, method="GET", body=None):
        request = urllib.request.Request(f"http://127.0.0.1:{control_port}{path}",
                                        data=None if body is None else json.dumps(body).encode(), method=method,
                                        headers={"Authorization": "Bearer isolated-test", "Content-Type": "application/json"})
        with opener.open(request, timeout=20) as response:
            data = response.read()
            return json.loads(data) if data else None

    def fetch(url, port=proxy_port):
        return subprocess.run(["/usr/bin/curl", "--fail", "--silent", "--show-error", "--max-time", "4", "--noproxy", "",
                               "--proxy", f"http://127.0.0.1:{port}", url], capture_output=True, text=True)

    with tempfile.TemporaryDirectory(prefix="smartclash-modes-") as directory:
        path = pathlib.Path(directory)
        (path / "config.json").write_text(json.dumps(config))
        core = None
        try:
            subprocess.run([str(root / "Resources/mihomo"), "-t", "-d", directory, "-f", str(path / "config.json")], check=True, capture_output=True)
            with (path / "core.log").open("w") as log:
                core = subprocess.Popen([str(root / "Resources/mihomo"), "-d", directory, "-f", str(path / "config.json")], stdout=log, stderr=log)
            for _ in range(100):
                try:
                    api("/version")
                    break
                except Exception:
                    assert core.poll() is None, (path / "core.log").read_text()
                    time.sleep(0.1)
            for name in config["rule-providers"]:
                api(f"/providers/rules/{name}", "PUT")
            providers = api("/providers/rules")["providers"]
            assert len(providers) == 6 and all(p["ruleCount"] > 0 for p in providers.values()), providers
            print("PASS: all six providers loaded, including the converted full upstream rules")
            url = f"http://127.0.0.1:{target.server_port}/"
            assert fetch(url).stdout == "DIRECT"
            assert fetch("http://proxy-mode.test/").stdout == "PROXY-A"
            assert fetch("http://child.suffix-mode.test/").stdout == "PROXY-A"
            assert fetch("http://some-keyword-mode.test/").stdout == "PROXY-A"
            assert fetch("http://203.0.113.42/").stdout == "PROXY-A"
            assert fetch("http://reject-mode.test/").returncode != 0
            print("PASS: rule mode routes private traffic directly, proxies listed domains, and rejects blocked domains")
            api("/configs", "PATCH", {"mode": "global"})
            assert api("/configs")["mode"] == "global"
            assert fetch(url).stdout == "PROXY-A"
            api("/proxies/SMART", "PUT", {"name": "B"})
            assert fetch(url).stdout == "PROXY-B"
            assert fetch("http://reject-mode.test/").stdout == "PROXY-B"
            assert fetch(url, probe_port).stdout == "PROXY-A"
            print("PASS: global mode follows SMART A → B, bypasses rules, and leaves PROBE independent")
            api("/configs", "PATCH", {"mode": "direct"})
            assert api("/configs")["mode"] == "direct"
            assert fetch(url).stdout == "DIRECT"
            print("PASS: direct mode bypasses the selected proxy")
            live = subprocess.Popen(["/usr/bin/curl", "--silent", "--max-time", "8", "--noproxy", "", "--proxy",
                                     f"http://127.0.0.1:{proxy_port}", url + "hold"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            try:
                held = []
                for _ in range(30):
                    held = [c["id"] for c in (api("/connections")["connections"] or []) if str(c["metadata"].get("destinationPort")) == str(target.server_port)]
                    if held:
                        break
                    time.sleep(0.05)
                assert held
                api("/connections", "DELETE")
                assert not set(held).intersection(c["id"] for c in (api("/connections")["connections"] or []))
                live.wait(timeout=3)
            finally:
                if live.poll() is None:
                    live.terminate()
                    live.wait(timeout=3)
            print("PASS: closing connections removes sessions that retain an old mode")
            api("/configs", "PATCH", {"mode": "rule"})
            payloads["sr-reject-domain.yaml"] = b'payload:\n  - "hot-reject.test"\n'
            api("/providers/rules/sr-reject-domain", "PUT")
            assert fetch("http://hot-reject.test/").returncode != 0
            assert api("/configs")["mode"] == "rule"
            print("PASS: provider hot update takes effect without restarting the core or changing its mode")
            empty = copy.deepcopy(config)
            empty["mode"] = "direct"
            empty["proxies"] = []
            for group in empty["proxy-groups"]:
                if group["name"] in ["SMART", "PROBE"]:
                    group["proxies"] = ["DIRECT"]
            api("/configs?force=true", "PUT", {"payload": json.dumps(empty)})
            assert fetch(url).stdout == "DIRECT"
            print("PASS: direct mode works with zero subscription nodes")
        except Exception:
            print((path / "core.log").read_text()[-7000:] if (path / "core.log").exists() else "Core failed configuration validation")
            raise
        finally:
            if core is not None:
                core.terminate()
                core.wait(timeout=10)
            for instance in [rules, target, proxy_a, proxy_b]:
                instance.shutdown()
                instance.server_close()


if __name__ == "__main__":
    main()
