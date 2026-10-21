"""What `import univo` may load and do.

Each check runs in a fresh isolated interpreter, and first asserts that the
modules it looks for are not already loaded, so preloading cannot hide a use.
"""

from ._subprocess import run_isolated

FORBIDDEN_AT_IMPORT = (
    "asyncio",
    "http",
    "huggingface_hub",
    "httpx",
    "numpy",
    "onnxruntime",
    "pandas",
    "requests",
    "safetensors",
    "socket",
    "ssl",
    "tokenizers",
    "torch",
    "transformers",
    "urllib.request",
    "urllib3",
)

LOADED_MODULES = """
import json, sys
forbidden = {forbidden!r}
preloaded = sorted(name for name in forbidden if name in sys.modules)
import univo
import univo.models
loaded = sorted(name for name in forbidden if name in sys.modules)
print(json.dumps({{"preloaded": preloaded, "loaded": loaded}}))
"""

SOCKET_AUDIT_EVENTS = """
import json, sys
events = []
sys.addaudithook(
    lambda event, args: events.append(event) if event.startswith("socket.") else None
)
import univo
import univo.models
print(json.dumps(events))
"""


def test_importing_univo_and_its_models_loads_no_heavy_or_network_module() -> None:
    result = run_isolated(LOADED_MODULES.format(forbidden=FORBIDDEN_AT_IMPORT))

    assert result["preloaded"] == [], "the check is void: modules were already loaded"
    assert result["loaded"] == []


def test_import_opens_no_python_socket() -> None:
    """Covers Python-level sockets; tools/checks/check_native_dependencies.py covers native code."""
    assert run_isolated(SOCKET_AUDIT_EVENTS) == []


CONSTRUCTION = """
import json, sys
from univo import Finder, Schema, Text
before = set(sys.modules)
events = []
sys.addaudithook(lambda event, args: events.append(event) if event.startswith("socket.") else None)
finder = Finder(schema=Schema(fields={"text": Text(mode="semantic")}),
                model_path="/missing/univo/model", offline=True)
print(json.dumps({"loaded": finder.config()["model_loaded"],
                  "network": events,
                  "model_modules": sorted(name for name in set(sys.modules) - before
                                          if name.startswith("univo.models"))}))
"""


def test_finder_construction_neither_loads_model_nor_opens_socket() -> None:
    assert run_isolated(CONSTRUCTION) == {"loaded": False, "network": [], "model_modules": []}
