"""``python -m univo.models``: download or inspect embedding models."""

import argparse
import json
import sys

from univo.errors import UnivoError
from univo.models import ModelInfo, download, inspect


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        prog="python -m univo.models",
        description="Download or inspect Univo embedding models.",
    )
    commands = parser.add_subparsers(dest="command", required=True)
    fetch = commands.add_parser(
        "download", help="download a model into the cache, verify and load it"
    )
    look = commands.add_parser(
        "inspect", help="describe a locally available model, without network access"
    )
    for command in (fetch, look):
        command.add_argument(
            "model",
            nargs="?",
            default="default",
            help='"default" (the default), "org/name[@revision]" or a local directory',
        )
        command.add_argument("--cache-dir", help="cache directory (default: UNIVO_CACHE_DIR)")
        command.add_argument("--json", action="store_true", help="print the result as JSON")
    fetch.add_argument("--hub-url", help="Hugging Face compatible mirror (default: HF_ENDPOINT)")
    arguments = parser.parse_args(argv)

    try:
        if arguments.command == "download":
            info = download(
                arguments.model, hub_url=arguments.hub_url, cache_dir=arguments.cache_dir
            )
        else:
            info = inspect(arguments.model, cache_dir=arguments.cache_dir)
    except UnivoError as error:
        print(error, file=sys.stderr)
        return 1
    print(_json(info) if arguments.json else _text(info))
    return 0


def _text(info: ModelInfo) -> str:
    lines = [
        f"model      {info.name}",
        f"source     {info.source}",
        f"path       {info.path}",
        f"dimension  {info.dimension}",
        f"backend    {info.backend}",
    ]
    lines += [f"file       {file.name}  {file.size} bytes  {file.digest}" for file in info.files]
    return "\n".join(lines)


def _json(info: ModelInfo) -> str:
    fields = info._asdict()
    fields["path"] = str(info.path)
    fields["files"] = [file._asdict() for file in info.files]
    return json.dumps(fields, indent=2)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)
