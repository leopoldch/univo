"""A local stand-in for the Hugging Face hub, serving the tiny test model."""

import hashlib
import http.server
import json
import pathlib
import socketserver
import threading
from collections.abc import Iterator
from contextlib import contextmanager
from dataclasses import dataclass, field

REPOSITORY_ROOT = pathlib.Path(__file__).resolve().parents[2]
TINY_MODEL = REPOSITORY_ROOT / "tests" / "fixtures" / "models" / "tiny"
REPOSITORY = "univo-test/tiny"
COMMIT = "0123456789abcdef0123456789abcdef01234567"
FILES = ("config.json", "model.safetensors", "tokenizer.json")


def metadata(directory: pathlib.Path, commit: str) -> bytes:
    """Hub metadata: weights as LFS with a SHA-256, JSON files as Git blobs."""
    siblings = []
    for name in FILES:
        content = (directory / name).read_bytes()
        entry: dict[str, object] = {"rfilename": name, "size": len(content)}
        if name.endswith(".safetensors"):
            entry["lfs"] = {"sha256": hashlib.sha256(content).hexdigest(), "size": len(content)}
        else:
            blob = hashlib.sha1(f"blob {len(content)}\0".encode() + content, usedforsecurity=False)
            entry["blobId"] = blob.hexdigest()
        siblings.append(entry)
    return json.dumps({"sha": commit, "siblings": siblings}).encode()


@dataclass
class Hub:
    """Requests seen and failures to inject, by request path suffix."""

    url: str = ""
    repository: str = REPOSITORY
    commit: str = COMMIT
    directory: pathlib.Path = TINY_MODEL
    requests: list[str] = field(default_factory=list)
    user_agents: set[str] = field(default_factory=set)
    statuses: dict[str, list[int]] = field(default_factory=dict)
    """Statuses answered, in order, before serving normally."""
    corrupt: set[str] = field(default_factory=set)
    oversized: set[str] = field(default_factory=set)

    def answer(self, path: str) -> tuple[int, bytes, dict[str, str]]:
        for suffix, statuses in self.statuses.items():
            if path.endswith(suffix) and statuses:
                return statuses.pop(0), b"", {}
        if path.startswith(f"/api/models/{self.repository}/revision/"):
            return 200, metadata(self.directory, self.commit), {}
        prefix = f"/{self.repository}/resolve/{self.commit}/"
        if path.startswith(prefix) and path.endswith(".safetensors"):
            # Large files are served from a CDN through a redirect.
            return 302, b"", {"Location": f"/cdn/{path.removeprefix(prefix)}"}
        name = path.removeprefix(prefix).removeprefix("/cdn/")
        if name not in FILES or not (path.startswith((prefix, "/cdn/"))):
            return 404, b"", {}
        content = bytearray((self.directory / name).read_bytes())
        if name in self.corrupt:
            content[-1] ^= 1
        if name in self.oversized:
            content += b" " * 64
        return 200, bytes(content), {}


class _Server(http.server.ThreadingHTTPServer):
    def server_bind(self) -> None:
        # HTTPServer.server_bind resolves the host name, which can take
        # seconds; the tests only need the address.
        socketserver.TCPServer.server_bind(self)
        host, port = self.server_address[:2]
        self.server_name, self.server_port = str(host), int(port)


@contextmanager
def serve(hub: Hub) -> Iterator[Hub]:
    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self) -> None:
            hub.requests.append(self.path)
            hub.user_agents.add(self.headers.get("User-Agent", ""))
            status, body, headers = hub.answer(self.path.split("?")[0])
            self.send_response(status)
            for key, value in headers.items():
                self.send_header(key, value)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, format: str, *args: object) -> None:
            pass

    server = _Server(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    hub.url = f"http://127.0.0.1:{server.server_address[1]}"
    try:
        yield hub
    finally:
        server.shutdown()
        server.server_close()
