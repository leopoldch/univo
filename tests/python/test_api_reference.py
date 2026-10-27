"""docs/reference/api.md documents every public name, method, parameter and attribute."""

import dataclasses
import inspect
import re
from collections.abc import Callable
from pathlib import Path
from typing import Any

import pytest
import univo
import univo.models

REFERENCE = (Path(__file__).parents[2] / "docs" / "reference" / "api.md").read_text()
BLOCKS = re.findall(r"^```\w*\n(.*?)^```", REFERENCE, re.MULTILINE | re.DOTALL)
PROSE = re.sub(r"^```\w*\n.*?^```", "", REFERENCE, flags=re.MULTILINE | re.DOTALL)
INLINE = re.findall(r"`([^`\n]+)`", PROSE)
CODE = BLOCKS + INLINE
HEADINGS = set(re.findall(r"^#+ (.+)$", PROSE, re.MULTILINE))


def documented(name: str) -> bool:
    """`name` is a heading, or appears in inline code or a code block."""
    return name in HEADINGS or any(re.search(rf"\b{re.escape(name)}\b", code) for code in CODE)


def explained(name: str) -> bool:
    """`name` appears in the text, not only in a signature."""
    return any(re.search(rf"\b{re.escape(name)}\b", code) for code in INLINE)


def parameters(function: Callable[..., Any]) -> list[str]:
    return [name for name in inspect.signature(function).parameters if name not in ("self", "cls")]


def test_every_public_name_is_documented() -> None:
    missing = [name for name in univo.__all__ if not documented(name)]
    missing += [name for name in univo.models.__all__ if not documented(name)]
    assert not missing


@pytest.mark.parametrize("cls", [univo.Finder, univo.Schema, univo.Text, univo.Exact])
def test_every_constructor_parameter_is_documented(cls: type) -> None:
    assert not [name for name in parameters(cls) if not explained(name)]


def test_every_finder_method_and_parameter_is_documented() -> None:
    methods = [
        name for name, _ in inspect.getmembers(univo.Finder, callable) if not name.startswith("_")
    ]
    assert not [name for name in methods if name not in HEADINGS]
    missing = [
        f"{method}({name})"
        for method in methods
        for name in parameters(getattr(univo.Finder, method))
        if not explained(name)
    ]
    assert not missing


@pytest.mark.parametrize(
    "cls", [univo.Match, univo.Report, univo.Groups, univo.Group, univo.Decision]
)
def test_every_result_attribute_is_documented(cls: Any) -> None:
    assert not [field.name for field in dataclasses.fields(cls) if not explained(field.name)]


def test_every_error_attribute_is_documented() -> None:
    attributes = inspect.get_annotations(univo.UnivoError)
    assert not [name for name in attributes if not explained(name)]


def test_every_stats_key_is_documented() -> None:
    assert not [key for key in univo.Finder().stats() if not explained(key)]


def test_model_functions_and_fields_are_documented() -> None:
    names = parameters(univo.models.download) + parameters(univo.models.inspect)
    names += list(univo.models.ModelInfo._fields) + list(univo.models.ModelFile._fields)
    assert not [name for name in names if not explained(name)]
