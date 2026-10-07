"""Generate Unicode oracles for the Rust normalizers from CPython.

CPython's unicodedata and str.casefold are an independent implementation of
NFC and full case folding. The fixtures pin the expected output of the text
and hostname normalizers defined in docs/duplicate-definitions.md:

- text_codepoints.tsv: every code point whose folded text form differs from
  itself;
- strings.jsonl: deterministic random strings mixing scripts, combining
  marks, ligatures, width variants and trailing dots.

Usage: python -I tools/build_unicode_fixtures.py
"""

import json
import pathlib
import random
import string
import sys
import unicodedata

OUTPUT = pathlib.Path(__file__).resolve().parents[1] / "tests" / "fixtures" / "unicode"
STRING_COUNT = 1500
SEED = 20261007


def _chars(*codes: int) -> str:
    return "".join(map(chr, codes))


POOLS = [
    "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 .-_",
    # Latin-1 accented letters and ligatures.
    _chars(0xE0, 0xE2, 0xE4, 0xE7, 0xE9, 0xE8, 0xEA, 0xEB, 0xEE, 0xEF, 0xF4, 0xF6, 0xF9, 0xFB),
    _chars(0xFC, 0xFF, 0xC0, 0xC9, 0x152, 0x153, 0xC6),
    # Combining marks, to exercise composition.
    _chars(0x300, 0x301, 0x302, 0x303, 0x308, 0x30A, 0x327, 0x323, 0x331),
    # Characters with special case folding: sharp s, dotted and dotless i,
    # long s, Latin ligatures.
    _chars(0xDF, 0x1E9E, 0x130, 0x131, 0x17F, 0xFB01, 0xFB02, 0xFB00, 0xFB03, 0xFB05),
    # Greek, including final sigma and accented vowels.
    _chars(0x391, 0x392, 0x393, 0x394, 0x3A3, 0x3C3, 0x3C2, 0x3A9, 0x3C9, 0x3AC, 0x390, 0x3B0),
    # Cyrillic.
    _chars(0x410, 0x411, 0x412, 0x413, 0x414, 0x415, 0x401, 0x416, 0x430, 0x431, 0x451, 0x42F),
    # Fullwidth Latin letters and digits.
    _chars(0xFF21, 0xFF22, 0xFF23, 0xFF41, 0xFF42, 0xFF43, 0xFF10, 0xFF11, 0xFF12),
    # Hangul jamo, which compose into syllables, and precomposed syllables.
    _chars(0x1100, 0x1161, 0x11A8, 0xAC00, 0xD55C),
    # CJK ideographs.
    _chars(0x6771, 0x4EAC, 0x96FB, 0x529B, 0x4E2D, 0x56FD, 0x94F6, 0x884C),
    # Zero-width, no-break and line separator characters.
    _chars(0x200B, 0x200D, 0xA0, 0x2028),
    # Emoji, including a regional indicator pair.
    _chars(0x1F355, 0x1F354, 0x1F1EB, 0x1F1F7),
]


def nfc(text: str) -> str:
    return unicodedata.normalize("NFC", text)


def fold_text(text: str) -> str:
    return nfc(nfc(text).casefold())


ASCII_LOWERCASE = str.maketrans(string.ascii_uppercase, string.ascii_lowercase)


def hostname(text: str) -> str | None:
    """None for an invalid hostname: more than one trailing dot."""
    if text.endswith(".."):
        return None
    return nfc(nfc(text).translate(ASCII_LOWERCASE)).removesuffix(".")


# Lowering an ASCII base before a combining mark can enable a composition:
# T + U+0308 becomes U+1E97. E + U+0301 composes to a non-ASCII letter first
# and is therefore never lowered.
EDGE_CASES = [
    _chars(0x54, 0x308, 0x2E, 0x64, 0x65),
    _chars(0x45, 0x301, 0x2E, 0x44, 0x45),
    _chars(0x1E97),
    _chars(0x66, 0x61, 0xDF, 0x2E, 0x64, 0x65),
    "Stripe.COM.",
    "a..",
    "a.",
    ".",
]


def assigned_bmp(rng: random.Random) -> str:
    while True:
        character = chr(rng.randrange(0x20, 0x10000))
        if unicodedata.category(character) not in {"Cn", "Cs", "Co"}:
            return character


def random_string(rng: random.Random) -> str:
    characters = []
    for _ in range(rng.randrange(0, 16)):
        if rng.random() < 0.15:
            characters.append(assigned_bmp(rng))
        else:
            characters.append(rng.choice(rng.choice(POOLS)))
    text = "".join(characters)
    return text + "." if rng.random() < 0.1 else text


def codepoints(text: str) -> str:
    return " ".join(f"{ord(character):X}" for character in text)


def main() -> int:
    OUTPUT.mkdir(parents=True, exist_ok=True)
    header = f"# CPython {sys.version.split()[0]}, Unicode {unicodedata.unidata_version}\n"

    lines = []
    for code in range(0x110000):
        if 0xD800 <= code <= 0xDFFF:
            continue
        character = chr(code)
        folded = fold_text(character)
        if folded != character:
            lines.append(f"{code:X}\t{codepoints(folded)}\n")
    (OUTPUT / "text_codepoints.tsv").write_text(header + "".join(lines), encoding="utf-8")

    rng = random.Random(SEED)
    records = []
    for text in EDGE_CASES + [random_string(rng) for _ in range(STRING_COUNT)]:
        record: dict[str, str | None] = {
            "input": text,
            "text": fold_text(text),
            "text_no_fold": nfc(text),
        }
        record["hostname"] = hostname(text)
        records.append(json.dumps(record, ensure_ascii=False) + "\n")
    (OUTPUT / "strings.jsonl").write_text("".join(records), encoding="utf-8")
    print(f"{len(lines)} code points, {len(records)} strings; {header.strip()}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
