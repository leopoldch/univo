"""HTTP GET requests for model downloads, on the standard library.

Rust plans and verifies every download (ADR 0008); this module only moves
bytes. It uses the system's proxy settings and certificates, sends nothing
but the URL and a User-Agent, and is imported only when a download starts.
"""

import http.client
import os
import time
import urllib.error
import urllib.request
from collections.abc import Callable
from typing import BinaryIO, TypeVar

from univo._univo import __version__

CHUNK_BYTES = 1 << 20

T = TypeVar("T")


class TransportError(Exception):
    """A failed request; read by Rust through ``status`` and ``reason``."""

    def __init__(self, reason: str, status: int | None = None, *, retryable: bool) -> None:
        super().__init__(reason)
        self.reason = reason
        self.status = status
        self.retryable = retryable


class UrllibTransport:
    """Follows redirects, applies ``timeout`` to every socket operation and
    makes at most ``attempts`` attempts, waiting ``backoff`` seconds, then
    twice as long, between them. Only connection failures, timeouts, HTTP
    429 and HTTP 5xx are retried."""

    def __init__(self, *, timeout: float = 30.0, attempts: int = 3, backoff: float = 1.0) -> None:
        self.timeout = timeout
        self.attempts = attempts
        self.backoff = backoff

    def fetch(self, url: str, destination: os.PathLike[str], max_bytes: int) -> None:
        def attempt() -> None:
            with open(destination, "wb") as output:
                self._get(url, max_bytes, output.write)

        self._retry(attempt)

    def fetch_text(self, url: str, max_bytes: int) -> str:
        def attempt() -> bytes:
            body = bytearray()
            self._get(url, max_bytes, body.extend)
            return bytes(body)

        body = self._retry(attempt)
        try:
            return body.decode("utf-8")
        except UnicodeDecodeError:
            raise TransportError("the response is not UTF-8 text", retryable=False) from None

    def keep_waiting(self) -> bool:
        return True

    def _retry(self, attempt: Callable[[], T]) -> T:
        delay = self.backoff
        for remaining in range(self.attempts - 1, -1, -1):
            try:
                return attempt()
            except TransportError as error:
                if not error.retryable or remaining == 0:
                    raise
            time.sleep(delay)
            delay *= 2
        raise AssertionError("unreachable")

    def _get(self, url: str, max_bytes: int, write: Callable[[bytes], object]) -> None:
        request = urllib.request.Request(url, headers={"User-Agent": f"univo/{__version__}"})
        try:
            with urllib.request.urlopen(request, timeout=self.timeout) as response:
                received = _copy(response, max_bytes, write)
                announced = response.headers.get("Content-Length")
                if announced is not None and announced.isdigit() and received < int(announced):
                    # The connection closed early without an error.
                    raise TransportError(
                        f"the response ended after {received} of {announced} bytes",
                        retryable=True,
                    )
        except urllib.error.HTTPError as error:
            raise TransportError(
                f"HTTP {error.code} {error.reason}",
                error.code,
                retryable=error.code == 429 or error.code >= 500,
            ) from None
        except urllib.error.URLError as error:
            raise TransportError(f"connection failed: {error.reason}", retryable=True) from None
        except TimeoutError:
            raise TransportError(f"no response for {self.timeout} s", retryable=True) from None
        except (ConnectionError, http.client.HTTPException) as error:
            raise TransportError(f"connection lost: {error!r}", retryable=True) from None


def _copy(response: BinaryIO, max_bytes: int, write: Callable[[bytes], object]) -> int:
    """Copies the body and returns its length in bytes."""
    received = 0
    while chunk := response.read(CHUNK_BYTES):
        received += len(chunk)
        if received > max_bytes:
            raise TransportError(f"the response exceeds {max_bytes} bytes", retryable=False)
        write(chunk)
    return received
