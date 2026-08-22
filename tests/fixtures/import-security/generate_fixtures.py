#!/usr/bin/env python3
"""Deterministic adversarial import-security fixture generator (Phase 9 prerequisite).

Builds the malicious/benign ZIP/TXT corpus specified in
docs/reviews/import-security-spec/import-security-spec.md and emits:

  corpus/<fixture>            the archive/text fixtures themselves
  manifest.json               machine-readable expected verdict per fixture (gate G3)
  MANIFEST.md                 human-readable table of the same

Determinism: fixed DOS timestamps, fixed ZipCrypto keystream header bytes, seeded
PRNG, raw-deflate level 6 -> identical bytes on every run for a given zlib version.
verify_corpus.py regenerates into a temp dir and asserts byte equality against the
committed corpus to prove reproducibility.

Stdlib only. Run:  python generate_fixtures.py [--out DIR]  (default: ./corpus next
to this script).
"""
from __future__ import annotations

import argparse
import hashlib
import json
import random
import struct
import zlib
from pathlib import Path

# ---------------------------------------------------------------------------
# Limits -- MUST mirror §3 of import-security-spec.md (single source of truth
# until Phase 9 lands them in Rust; verify_corpus.py asserts the mirror).
# ---------------------------------------------------------------------------
LIMITS = {
    "MAX_ENTRIES": 10_000,
    "MAX_FILE_UNCOMPRESSED": 256 * 1024 * 1024,
    "MAX_TOTAL_UNCOMPRESSED": 2 * 1024 * 1024 * 1024,
    "MAX_COMPRESSION_RATIO": 200,
    "RATIO_EXEMPT_BELOW": 4096,
    "MAX_NAME_BYTES": 1024,
    "MAX_COMPONENT_BYTES": 255,
}

DOS_TIME = 0  # 00:00:00
DOS_DATE = 0x21  # 1980-01-01

S_IFREG, S_IFDIR, S_IFLNK, S_IFCHR = 0o100000, 0o040000, 0o120000, 0o020000


# ---------------------------------------------------------------------------
# Low-level ZIP construction (full control over names/flags/modes; stdlib
# zipfile cannot emit CP437 names without the UTF-8 flag nor ZipCrypto).
# ---------------------------------------------------------------------------
class Member:
    def __init__(self, name_bytes, data=b"", *, method=8, flags=0, mode=S_IFREG | 0o644,
                 unix_attrs=True, encrypted=False, password=None):
        self.name_bytes = name_bytes
        self.data = data
        self.method = method
        self.flags = flags
        self.mode = mode
        self.unix_attrs = unix_attrs
        self.encrypted = encrypted
        self.password = password or b"x"


def _zipcrypto_keystream(password: bytes):
    k0, k1, k2 = 0x12345678, 0x23456789, 0x34567890

    def update(c):
        nonlocal k0, k1, k2
        k0 = zlib.crc32(bytes([c]), k0) & 0xFFFFFFFF
        k1 = (k1 + (k0 & 0xFF)) & 0xFFFFFFFF
        k1 = (k1 * 134775813 + 1) & 0xFFFFFFFF
        k2 = zlib.crc32(bytes([(k1 >> 24) & 0xFF]), k2) & 0xFFFFFFFF

    def dec_byte():
        t = (k2 | 2) & 0xFFFF
        return ((t * (t ^ 1)) >> 8) & 0xFF

    for c in password:
        update(c)

    def stream(chunk: bytes) -> bytes:
        out = bytearray()
        for p in chunk:
            d = dec_byte()
            c = p ^ d
            out.append(c)
            update(p)
        return bytes(out)

    return stream


def _encrypt_member(member: Member, rng: random.Random) -> tuple[bytes, int]:
    """ZipCrypto-encrypt stored plaintext; returns (cipher, crc_of_plaintext)."""
    crc = zlib.crc32(member.data) & 0xFFFFFFFF
    header = bytes(rng.randrange(256) for _ in range(11)) + bytes([(crc >> 24) & 0xFF])
    stream = _zipcrypto_keystream(member.password)
    cipher = stream(header + member.data)
    return cipher, crc


class ZipBuilder:
    """Assembles a ZIP from Members, recording offsets for surgical corruption."""

    def __init__(self):
        self.members: list[Member] = []
        self.rng = random.Random(0x50415253)  # fixed seed -> deterministic crypto headers

    def add(self, member: Member) -> None:
        self.members.append(member)

    def build(self) -> tuple[bytes, dict]:
        body = bytearray()
        records = []
        for m in self.members:
            crc = zlib.crc32(m.data) & 0xFFFFFFFF
            payload = m.data
            if m.encrypted:
                payload, crc = _encrypt_member(m, self.rng)
                m.flags |= 0x1
                m.method = 0  # stored: keeps the fixture hand-checkable
            elif m.method == 8:
                co = zlib.compressobj(6, zlib.DEFLATED, -15)
                payload = co.compress(m.data) + co.flush()
            nlen = len(m.name_bytes)
            offset = len(body)
            local = struct.pack(
                "<IHHHHHIIIHH", 0x04034B50, 20, m.flags, m.method, DOS_TIME, DOS_DATE,
                crc, len(payload), len(m.data), nlen, 0)
            body += local + m.name_bytes + payload
            records.append((m, crc, len(payload), offset))
        cd_offset = len(body)
        for m, crc, csize, offset in records:
            ext_attr = ((m.mode & 0xFFFF) << 16) if m.unix_attrs else 0
            cd = struct.pack(
                "<IHHHHHHIIIHHHHHII", 0x02014B50, (3 << 8) | 20, 20, m.flags, m.method,
                DOS_TIME, DOS_DATE, crc, csize, len(m.data), len(m.name_bytes), 0, 0,
                0, 0, ext_attr, offset)
            body += cd + m.name_bytes
            if not m.unix_attrs:
                pass
        eocd_offset = len(body)
        body += struct.pack("<IHHHHIIH", 0x06054B50, 0, 0, len(records),
                            len(records), eocd_offset - cd_offset, cd_offset, 0)
        meta = {
            "member_offsets": [r[3] for r in records],
            "member_name_offsets": [],  # filled below
            "cd_offset": cd_offset,
            "eocd_offset": eocd_offset,
        }
        # recompute local name-field offsets (30-byte fixed header after sig)
        name_offsets = []
        for (_m, _crc, _cs, off) in records:
            name_offsets.append(off + 30)
        meta["member_name_offsets"] = name_offsets
        return bytes(body), meta


# ---------------------------------------------------------------------------
# Payload builders
# ---------------------------------------------------------------------------
CHAT_LINES = [
    "01.03.2024, 09:15 - Alice Neumann: Hallo zusammen, Meeting startet in fünf Minuten.",
    "01.03.2024, 09:16 - Bob Schmidt: Bin dabei.",
    "01.03.2024, 09:31 - Alice Neumann: Protokoll folgt bis Freitag.",
]

FAKE_JPEG = b"\xff\xd8\xff\xe0" + b"\x00" * 64 + b"\xff\xd9"
FAKE_MP4 = b"\x00\x00\x00\x18ftypmp42" + b"\x00" * 128


def ratio_limited_data(total: int, seed: int, *, block_size: int = 8192,
                       rand_len: int = 64) -> bytes:
    """~105:1 bounded-compressible filler. Every 8 KiB block starts with 64
    SEEDED RANDOM bytes, so consecutive blocks are distinct: deflate's 32 KiB
    window cannot collapse later blocks onto earlier ones, which pins the
    sustained ratio safely below MAX_COMPRESSION_RATIO (200) while keeping the
    on-disk fixture small. (An earlier design repeated one identical 16 KiB
    period; LZ77 back-references crushed it far beyond 200:1 and mis-tripped
    the ratio rule instead of the size rules these fixtures must isolate.)"""
    rng = random.Random(seed)
    out = bytearray()
    full, tail = divmod(total, block_size)
    for _ in range(full):
        out += rng.randbytes(rand_len) + b"\x00" * (block_size - rand_len)
    if tail:
        r = rng.randbytes(min(rand_len, tail))
        out += r + b"\x00" * (tail - len(r))
    return bytes(out)


def chat_text() -> str:
    return "\r\n".join(["WhatsApp Chat mit Testgruppe", ""] + CHAT_LINES) + "\r\n"


# ---------------------------------------------------------------------------
# Individual fixtures
# ---------------------------------------------------------------------------
def fx_benign_minimal(b: ZipBuilder):
    b.add(Member(b"_chat.txt", chat_text().encode("utf-8"), flags=0x800))


def fx_benign_media(b: ZipBuilder):
    b.add(Member(b"_chat.txt", chat_text().encode("utf-8"), flags=0x800))
    b.add(Member(b"Media/", b"", method=0, mode=S_IFDIR | 0o755))
    b.add(Member(b"Media/IMG-20240301-WA0001.jpg", FAKE_JPEG, method=0))
    b.add(Member(b"Media/AUD-20240301-WA0002.opus", b"OggS" + b"\x00" * 32, method=0))


def fx_benign_unicode_utf8(b: ZipBuilder):
    name = "Medien/WhatsApp Bild 2024-03-01 um 12.00.30.jpeg".encode("utf-8")
    b.add(Member(b"_chat.txt", chat_text().encode("utf-8"), flags=0x800))
    b.add(Member(name, FAKE_JPEG, flags=0x800))


def fx_benign_cp437_legacy(b: ZipBuilder):
    # Raw CP437 bytes, UTF-8 flag NOT set (legacy Windows tooling style).
    name = "Grüße.txt".encode("cp437")
    b.add(Member(name, "Grüße aus dem Legacy-Export".encode("utf-8")))


def fx_benign_dotfile(b: ZipBuilder):
    b.add(Member(b"_chat.txt", chat_text().encode("utf-8"), flags=0x800))
    b.add(Member(b".wa_metadata.json", b'{"exported": true}', method=0))


def fx_benign_nested_archive_opaque(b: ZipBuilder):
    inner = ZipBuilder()
    inner.add(Member(b"bomb.txt", b"\x00" * (64 * 1024 * 1024)))
    inner_blob, _ = inner.build()
    b.add(Member(b"_chat.txt", chat_text().encode("utf-8"), flags=0x800))
    b.add(Member(b"Media/backup_export.zip", inner_blob))


def fx_traversal_basic(b: ZipBuilder):
    b.add(Member(b"_chat.txt", chat_text().encode("utf-8"), flags=0x800))
    b.add(Member(b"../evil.txt", b"pwned"))


def fx_traversal_deep_mixed(b: ZipBuilder):
    b.add(Member(b"a/b/../../../../../../etc/shadow", b"pwned"))
    b.add(Member(b"media/..\\..\\Windows\\evil.dll", b"pwned"))


def fx_traversal_backslash(b: ZipBuilder):
    b.add(Member(b"..\\..\\..\\evil.exe", b"pwned"))


def fx_absolute_unix(b: ZipBuilder):
    b.add(Member(b"/etc/passwd", b"pwned"))


def fx_absolute_windows(b: ZipBuilder):
    b.add(Member(b"C:\\Users\\victim\\AppData\\evil.dll", b"pwned"))


def fx_absolute_unc(b: ZipBuilder):
    b.add(Member(b"\\\\attacker\\share\\payload.exe", b"pwned"))


def fx_ads_colon(b: ZipBuilder):
    # Lone colon inside a component: NTFS alternate-data-stream syntax.
    b.add(Member(b"_chat.txt", chat_text().encode("utf-8"), flags=0x800))
    b.add(Member(b"notes.txt:hidden", b"pwned"))


def fx_symlink_escape(b: ZipBuilder):
    b.add(Member(b"_chat.txt", chat_text().encode("utf-8"), flags=0x800))
    b.add(Member(b"link_out", b"../../outside.txt", method=0, mode=S_IFLNK | 0o777))


def fx_symlink_internal(b: ZipBuilder):
    b.add(Member(b"_chat.txt", chat_text().encode("utf-8"), flags=0x800))
    b.add(Member(b"link_in", b"_chat.txt", method=0, mode=S_IFLNK | 0o777))


def fx_special_member(b: ZipBuilder):
    b.add(Member(b"_chat.txt", chat_text().encode("utf-8"), flags=0x800))
    b.add(Member(b"dev_zero", b"", method=0, mode=S_IFCHR | 0o644))


def fx_reserved_name(b: ZipBuilder):
    b.add(Member(b"COM1.txt", b"pwned"))


def fx_ratio_entry(b: ZipBuilder):
    # 200 MiB of zeros -> ~1080:1, under the single-file cap, over MAX_RATIO.
    b.add(Member(b"zeros.bin", b"\x00" * (200 * 1024 * 1024)))


def fx_single_file_oversize(b: ZipBuilder):
    # 300 MiB at ~105:1 (ratio rule NOT tripped) -> trips the single-file cap alone.
    data = ratio_limited_data(300 * 1024 * 1024, seed=101)
    b.add(Member(b"big_media.raw", data))


def fx_total_size(b: ZipBuilder):
    # 11 x 196 MiB = 2156 MiB > 2 GiB total cap; each entry < 256 MiB and
    # ~55:1 so ONLY the total-size rule fires.
    for i in range(11):
        data = ratio_limited_data(196 * 1024 * 1024, seed=200 + i)
        b.add(Member(f"media_{i:02d}.raw".encode(), data))


def fx_too_many_entries(b: ZipBuilder):
    for i in range(LIMITS["MAX_ENTRIES"] + 1):
        b.add(Member(f"f{i:05d}.txt".encode(), b"x", method=0))


def _encrypted_member(name: bytes, plaintext: bytes) -> Member:
    return Member(name, plaintext, encrypted=True, password=b"correct-horse-battery")


def fx_encrypted_zipcrypto(b: ZipBuilder):
    b.add(_encrypted_member(b"secret_chat.txt",
                            chat_text().encode("utf-8")))  # genuinely ZipCrypto'd


def fx_encrypted_partial(b: ZipBuilder):
    b.add(Member(b"_chat.txt", chat_text().encode("utf-8"), flags=0x800))  # readable bait
    b.add(_encrypted_member(b"Media/IMG-20240301-WA0001.jpg", FAKE_JPEG))


def _corrupt_truncated(blob: bytes, meta: dict) -> bytes:
    return blob[: meta["eocd_offset"] - 25]


def _corrupt_crc(blob: bytes, meta: dict) -> bytes:
    # First member is stored chat text; flip one byte inside its data region.
    off = meta["member_offsets"][0] + 30 + len(b"_chat.txt") + 10
    return blob[:off] + bytes([blob[off] ^ 0xFF]) + blob[off + 1:]


def _corrupt_central_directory(blob: bytes, meta: dict) -> bytes:
    off = meta["cd_offset"] + 2  # break the PK\x01\x02 signature
    return blob[:off] + bytes([blob[off] ^ 0xFF]) + blob[off + 1:]


def _corrupt_local_header(blob: bytes, meta: dict) -> bytes:
    off = meta["member_name_offsets"][1]  # garble 2nd member's local name
    return blob[:off] + bytes([blob[off] ^ 0xFF]) + blob[off + 1:]


def build_corrupt_crc_stored(_b=None) -> tuple[bytes, dict]:
    b = ZipBuilder()
    b.add(Member(b"_chat.txt", chat_text().encode("utf-8"), method=0))
    b.add(Member(b"note.txt", b"hello", method=0))
    blob, meta = b.build()
    return _corrupt_crc(blob, meta), meta


def build_corrupt_local_header(_b=None) -> tuple[bytes, dict]:
    b = ZipBuilder()
    b.add(Member(b"_chat.txt", chat_text().encode("utf-8"), method=0))
    b.add(Member(b"note.txt", b"hello", method=0))
    blob, meta = b.build()
    return _corrupt_local_header(blob, meta), meta


def build_corrupt_central_directory(_b=None) -> tuple[bytes, dict]:
    b = ZipBuilder()
    b.add(Member(b"_chat.txt", chat_text().encode("utf-8")))
    b.add(Member(b"note.txt", b"hello"))
    blob, meta = b.build()
    return _corrupt_central_directory(blob, meta), meta


def build_corrupt_truncated_eocd(_b=None) -> tuple[bytes, dict]:
    b = ZipBuilder()
    b.add(Member(b"_chat.txt", chat_text().encode("utf-8")))
    b.add(Member(b"note.txt", b"hello"))
    blob, meta = b.build()
    return _corrupt_truncated(blob, meta), meta


# ---------------------------------------------------------------------------
# TXT fixtures
# ---------------------------------------------------------------------------
TXT_FIXTURES = {
    "benign_chat_utf8_bom.txt": (b"\xef\xbb\xbf" + chat_text().encode("utf-8"), "ACCEPT", None, []),
    "benign_chat_utf16le.txt": (b"\xff\xfe" + chat_text().encode("utf-16-le"), "ACCEPT", None, []),
    "benign_chat_no_timestamps.txt": (b"free form notes without any parsable timestamps\nsecond line\n", "ACCEPT", None, ["unparsable_lines=all"]),
    "attack_txt_empty.txt": (b"", "REJECT", "E_EMPTY", []),
    "attack_txt_binary_masquerade.txt": (b"PK\x03\x04 looks like a zip renamed to .txt", "REJECT", "E_NOT_TEXT", []),
    "attack_txt_invalid_utf8.txt": (b"01.03.2024, 09:15 - \xff\xfe\xfa broken: bytes\n", "REJECT", "E_TEXT_DECODING", []),
}


# ---------------------------------------------------------------------------
# Corpus assembly
# ---------------------------------------------------------------------------
ZIP_FIXTURES = [
    ("benign_minimal.zip", fx_benign_minimal, "ACCEPT", None, []),
    ("benign_media.zip", fx_benign_media, "ACCEPT", None, []),
    ("benign_unicode_utf8.zip", fx_benign_unicode_utf8, "ACCEPT", None, []),
    ("benign_cp437_legacy.zip", fx_benign_cp437_legacy, "ACCEPT", None, ["name_decoded_cp437"]),
    ("benign_dotfile.zip", fx_benign_dotfile, "ACCEPT", None, []),
    ("benign_nested_archive_opaque.zip", fx_benign_nested_archive_opaque, "ACCEPT", None,
     ["inner_archives_are_opaque"]),
    ("attack_traversal_basic.zip", fx_traversal_basic, "REJECT", "E_TRAVERSAL", []),
    ("attack_traversal_deep_mixed.zip", fx_traversal_deep_mixed, "REJECT", "E_TRAVERSAL", []),
    ("attack_traversal_backslash.zip", fx_traversal_backslash, "REJECT", "E_TRAVERSAL", []),
    ("attack_absolute_unix.zip", fx_absolute_unix, "REJECT", "E_ABSOLUTE_PATH", []),
    ("attack_absolute_windows.zip", fx_absolute_windows, "REJECT", "E_ABSOLUTE_PATH", []),
    ("attack_absolute_unc.zip", fx_absolute_unc, "REJECT", "E_ABSOLUTE_PATH", []),
    ("attack_ads_colon.zip", fx_ads_colon, "REJECT", "E_ABSOLUTE_PATH", []),
    ("attack_symlink_escape.zip", fx_symlink_escape, "REJECT", "E_SYMLINK", []),
    ("attack_symlink_internal.zip", fx_symlink_internal, "REJECT", "E_SYMLINK", []),
    ("attack_special_member.zip", fx_special_member, "REJECT", "E_SPECIAL_MEMBER", []),
    ("attack_reserved_name.zip", fx_reserved_name, "REJECT", "E_RESERVED_NAME", []),
    ("attack_ratio_entry.zip", fx_ratio_entry, "REJECT", "E_RATIO_BOMB", []),
    ("attack_single_file_oversize.zip", fx_single_file_oversize, "REJECT", "E_SINGLE_FILE_TOO_LARGE", []),
    ("attack_total_size.zip", fx_total_size, "REJECT", "E_TOTAL_SIZE_EXCEEDED", []),
    ("attack_too_many_entries.zip", fx_too_many_entries, "REJECT", "E_TOO_MANY_ENTRIES", []),
    ("attack_encrypted_zipcrypto.zip", fx_encrypted_zipcrypto, "REJECT", "E_ENCRYPTED", []),
    ("attack_encrypted_partial.zip", fx_encrypted_partial, "REJECT", "E_ENCRYPTED", []),
    ("corrupt_truncated_eocd.zip", build_corrupt_truncated_eocd, "REJECT", "E_CORRUPT_CONTAINER", []),
    ("corrupt_central_directory.zip", build_corrupt_central_directory, "REJECT", "E_CORRUPT_CONTAINER", []),
    ("corrupt_crc_stored.zip", build_corrupt_crc_stored, "REJECT", "E_CRC_MISMATCH", []),
    ("corrupt_local_header.zip", build_corrupt_local_header, "REJECT", "E_HEADER_MISMATCH", []),
]

CLASS_OF = {
    "benign": "benign accept-path",
    "attack_traversal": "traversal",
    "attack_absolute": "absolute path",
    "attack_ads": "absolute path",
    "attack_symlink": "symlink",
    "attack_special": "symlink/special-member",
    "attack_reserved": "reserved name",
    "attack_ratio": "bomb ratio",
    "attack_single": "bomb ratio",
    "attack_total": "bomb ratio",
    "attack_too_many": "bomb ratio",
    "attack_encrypted": "encrypted",
    "corrupt": "corrupt",
    "attack_txt": "text rule",
    "benign_chat": "text rule",
}


def classify(name: str) -> str:
    for prefix, label in CLASS_OF.items():
        if name.startswith(prefix):
            return label
    return "uncategorized"


def generate(out_dir: Path) -> None:
    corpus_dir = out_dir / "corpus"
    corpus_dir.mkdir(parents=True, exist_ok=True)
    manifest = []

    for name, builder, decision, code, warnings in ZIP_FIXTURES:
        b = ZipBuilder()
        result = builder(b)
        blob = result[0] if isinstance(result, tuple) else b.build()[0]
        path = corpus_dir / name
        path.write_bytes(blob)
        manifest.append({
            "file": f"corpus/{name}",
            "class": classify(name),
            "expected": {"decision": decision, "primary_code": code, "warnings": warnings},
            "bytes": len(blob),
            "sha256": hashlib.sha256(blob).hexdigest(),
        })

    for name, (data, decision, code, warnings) in TXT_FIXTURES.items():
        path = corpus_dir / name
        path.write_bytes(data)
        manifest.append({
            "file": f"corpus/{name}",
            "class": classify(name),
            "expected": {"decision": decision, "primary_code": code, "warnings": warnings},
            "bytes": len(data),
            "sha256": hashlib.sha256(data).hexdigest(),
        })

    manifest_doc = {
        "schema": "handy.import-security-fixtures",
        "manifest_version": 1,
        "spec": "docs/reviews/import-security-spec/import-security-spec.md",
        "limits": LIMITS,
        "fixtures": sorted(manifest, key=lambda e: e["file"]),
    }
    (out_dir / "manifest.json").write_text(
        json.dumps(manifest_doc, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")

    rows = ["| Fixture | Class | Decision | Primary code | Warnings | Bytes |",
            "| --- | --- | --- | --- | --- | --- |"]
    for e in manifest_doc["fixtures"]:
        exp = e["expected"]
        rows.append(f"| `{e['file']}` | {e['class']} | {exp['decision']} | "
                    f"{exp['primary_code'] or '—'} | {', '.join(exp['warnings']) or '—'} | "
                    f"{e['bytes']:,} |")
    (out_dir / "MANIFEST.md").write_text(
        "# Import-Security Fixture Manifest\n\nGenerated by `generate_fixtures.py`; "
        "machine-readable source of truth: `manifest.json`.\n\n" + "\n".join(rows) + "\n",
        encoding="utf-8")


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", type=Path, default=Path(__file__).resolve().parent)
    args = ap.parse_args()
    generate(args.out)
    print(f"fixtures written under {args.out / 'corpus'}")
