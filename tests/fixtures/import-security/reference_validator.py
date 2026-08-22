#!/usr/bin/env python3
"""Dependency-free reference implementation of the Phase 9 import-security spec.

Implements sections 3-8 of
docs/reviews/import-security-spec/import-security-spec.md as an executable,
stdlib-only validator so that every fixture in corpus/ can be checked against
its manifest.json verdict (gate G3) before Phase 9 lands the Rust production
validator. This file is TEST INFRASTRUCTURE, not production code (gate G4).

Verdict model (spec section 2): exactly one outcome per input --
    {"decision": "ACCEPT"| "REJECT", "primary_code": str|null,
     "violations": [{"code": ..., "member": ...}], "warnings": [...],
     "stats": {...}}

Global precedence (concretizes spec sections 2, 4.3, 5.3, 6.3): the primary
code is the highest-priority violation present, in this order:
    Phase A: E_CORRUPT_CONTAINER, E_CORRUPT_METADATA
    mode:    E_ENCRYPTED
    links:   E_SYMLINK, E_SPECIAL_MEMBER
    paths:   E_TRAVERSAL, E_ABSOLUTE_PATH, E_RESERVED_NAME, E_INVALID_NAME
    limits:  E_TOO_MANY_ENTRIES, E_SINGLE_FILE_TOO_LARGE, E_RATIO_BOMB,
             E_TOTAL_SIZE_EXCEEDED
    Phase C (stream order): E_HEADER_MISMATCH, E_TRUNCATED, E_SIZE_MISMATCH,
             E_CRC_MISMATCH

Routing: a *.zip input goes through the archive validator; anything else goes
through the bare-text (section 8) validator. A .txt renamed from a ZIP is
caught by R-TXT-3 binary-magic rejection, and a corrupt ZIP still routes as
ZIP (its damage is described by the Phase-A codes instead).

CLI:  python reference_validator.py <file> [--quiet]
"""
from __future__ import annotations

import argparse
import json
import re
import struct
import sys
import zlib
from dataclasses import dataclass, field
from pathlib import Path

# ---------------------------------------------------------------------------
# Limits -- MUST mirror section 3 of import-security-spec.md. verify_corpus.py
# asserts that this mirror, generate_fixtures.LIMITS and manifest.json agree.
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

S_IFMT, S_IFREG, S_IFDIR, S_IFLNK = 0o170000, 0o100000, 0o040000, 0o120000

PRECEDENCE = [
    "E_CORRUPT_CONTAINER",
    "E_CORRUPT_METADATA",
    "E_ENCRYPTED",
    "E_SYMLINK",
    "E_SPECIAL_MEMBER",
    "E_TRAVERSAL",
    "E_ABSOLUTE_PATH",
    "E_RESERVED_NAME",
    "E_INVALID_NAME",
    "E_TOO_MANY_ENTRIES",
    "E_SINGLE_FILE_TOO_LARGE",
    "E_RATIO_BOMB",
    "E_TOTAL_SIZE_EXCEEDED",
    "E_HEADER_MISMATCH",
    "E_TRUNCATED",
    "E_SIZE_MISMATCH",
    "E_CRC_MISMATCH",
]

RESERVED_BASES = (
    {"CON", "PRN", "AUX", "NUL"}
    | {f"COM{i}" for i in range(1, 10)}
    | {f"LPT{i}" for i in range(1, 10)}
)

DRIVE_RE = re.compile(r"^[A-Za-z]:")

ARCHIVE_SUFFIXES = (".zip", ".gz", ".7z", ".tar", ".rar", ".bz2", ".xz")

TEXT_MAGIC_PREFIXES = (
    b"PK\x03\x04", b"%PDF", b"\x89PNG", b"\xff\xd8\xff",
    b"MThd", b"RIFF", b"OggS", b"\x1f\x8b",
)

# German-style WhatsApp export line: "01.03.2024, 09:15 - Name: text"
PARSEABLE_LINE_RE = re.compile(
    r"^\s*\d{1,2}[./]\d{1,2}[./]\d{2,4},?\s+\d{1,2}:\d{2}\s+-\s", re.M)


@dataclass
class Violation:
    code: str
    member: str


@dataclass
class Result:
    decision: str
    primary_code: str | None
    violations: list[Violation] = field(default_factory=list)
    warnings: list[str] = field(default_factory=list)
    stats: dict = field(default_factory=dict)

    def to_dict(self) -> dict:
        return {
            "decision": self.decision,
            "primary_code": self.primary_code,
            "violations": [{"code": v.code, "member": v.member} for v in self.violations],
            "warnings": self.warnings,
            "stats": self.stats,
        }


def _finish(violations: list[Violation], warnings: list[str], stats: dict) -> Result:
    """Collapse the violation list into the single deterministic verdict."""
    if not violations:
        return Result("ACCEPT", None, [], warnings, stats)
    primary = min(
        violations, key=lambda v: PRECEDENCE.index(v.code)
        if v.code in PRECEDENCE else len(PRECEDENCE))
    return Result("REJECT", primary.code, violations, warnings, stats)


# ---------------------------------------------------------------------------
# ZIP path (sections 4-7)
# ---------------------------------------------------------------------------
@dataclass
class Entry:
    name_bytes: bytes
    flags: int
    method: int
    crc: int
    csize: int
    usize: int
    ext_attr: int
    offset: int
    # filled by phase B:
    name: str = ""
    is_dir: bool = False


def _find_eocd(blob: bytes) -> int | None:
    """Return offset of the End-Of-Central-Directory record, or None.

    Tolerates a comment of up to 64 KiB (spec R-INTEG-1); anything else is a
    missing/unparseable EOCD."""
    window = 22 + 65535
    start = max(0, len(blob) - window)
    idx = blob.rfind(b"PK\x05\x06", start)
    return idx if idx >= 0 else None


def _parse_zip64_extra(extra: bytes, usize: int, csize: int, offset: int):
    """Substitute ZIP64 fields per appnote order (original size, compressed
    size, relative header offset). Returns (usize, csize, offset, ok)."""
    pos = 0
    while pos + 4 <= len(extra):
        cid, clen = struct.unpack_from("<HH", extra, pos)
        pos += 4
        if cid == 0x0001:
            body = extra[pos:pos + clen]
            # positional: take one u64 for each 0xFFFFFFFF sentinel, in order
            vals = []
            bpos = 0
            for cur in (usize, csize, offset):
                if cur == 0xFFFFFFFF:
                    if bpos + 8 > len(body):
                        return usize, csize, offset, False
                    vals.append(struct.unpack_from("<Q", body, bpos)[0])
                    bpos += 8
                else:
                    vals.append(cur)
            return vals[0], vals[1], vals[2], True
        pos += clen
    return usize, csize, offset, True


def _parse_central_directory(blob: bytes, cd_offset: int, cd_size: int, count: int):
    """Returns (entries, None) or (None, error_code)."""
    entries = []
    pos = cd_offset
    end = cd_offset + cd_size
    if end > len(blob) or cd_offset < 0:
        return None, "E_CORRUPT_CONTAINER"
    for _ in range(count):
        if pos + 46 > len(blob) or blob[pos:pos + 4] != b"PK\x01\x02":
            return None, "E_CORRUPT_CONTAINER"
        (flags, method, _t, _d, crc, csize, usize, nlen, elen, _clen,
         _disk, _iattr, ext_attr, offset) = struct.unpack_from(
            "<HHHHIIIHHHHHII", blob, pos + 8)
        name_bytes = blob[pos + 46:pos + 46 + nlen]
        extra = blob[pos + 46 + nlen:pos + 46 + nlen + elen]
        if len(name_bytes) != nlen or len(extra) != elen:
            return None, "E_CORRUPT_CONTAINER"
        usize, csize, offset, ok = _parse_zip64_extra(extra, usize, csize, offset)
        if not ok:
            return None, "E_CORRUPT_METADATA"
        entries.append(Entry(name_bytes, flags, method, crc, csize, usize,
                             ext_attr, offset))
        pos += 46 + nlen + elen
    return entries, None


def _decode_name(entry: Entry, warnings: list[str]) -> tuple[str, str] | None:
    """Section 4.1. Returns (decoded name, encoding) or None on metadata
    corruption (strict-UTF-8 failure when the UTF-8 flag is set)."""
    if entry.flags & 0x800:
        try:
            return entry.name_bytes.decode("utf-8"), "utf8"
        except UnicodeDecodeError:
            return None
    name = entry.name_bytes.decode("cp437")
    if any(ord(c) > 127 for c in name):
        warnings.append("name_decoded_cp437")
    return name, "cp437"


def _path_violations(name: str) -> list[str]:
    """Sections 4.2 R-PATH-1..8 on the normalized name."""
    codes: list[str] = []
    if any(ord(c) < 0x20 for c in name):
        codes.append("E_INVALID_NAME")
    norm = name.replace("\\", "/")
    comps = norm.split("/")
    if norm.startswith("/"):
        codes.append("E_ABSOLUTE_PATH")
    kept = []
    for comp in comps:
        if comp == "..":
            codes.append("E_TRAVERSAL")
        elif comp == ".":
            continue
        else:
            kept.append(comp)
    if norm and not kept and "E_ABSOLUTE_PATH" not in codes:
        # name reduced to nothing (e.g. "././") -- nothing materializable but
        # hostile-shaped; treat as traversal (R-PATH-2 tail).
        codes.append("E_TRAVERSAL")
    for comp in kept:
        # R-PATH-4: drive-letter prefixes (C:\, C:relative) AND any other
        # colon -- the latter is NTFS alternate-data-stream syntax
        # ("file.txt:stream") and is rejected outright.
        if DRIVE_RE.match(comp) or ":" in comp:
            codes.append("E_ABSOLUTE_PATH")
        base = comp.split(".", 1)[0].upper()
        if base in RESERVED_BASES:
            codes.append("E_RESERVED_NAME")
        if comp != comp.rstrip(". "):
            codes.append("E_RESERVED_NAME")
        if len(comp.encode("utf-8")) > LIMITS["MAX_COMPONENT_BYTES"]:
            codes.append("E_INVALID_NAME")
    if len(norm.encode("utf-8")) > LIMITS["MAX_NAME_BYTES"]:
        codes.append("E_INVALID_NAME")
    # de-duplicate preserving first occurrence
    seen: set[str] = set()
    return [c for c in codes if not (c in seen or seen.add(c))]


def validate_zip(blob: bytes) -> Result:
    violations: list[Violation] = []
    warnings: list[str] = []

    # ---- Phase A: container integrity -------------------------------------
    eocd_pos = _find_eocd(blob)
    if eocd_pos is None:
        return _finish([Violation("E_CORRUPT_CONTAINER", "<archive>")], warnings,
                       {"entries_scanned": 0})
    (_disk_no, _cd_disk, _this_entries, _total_entries, cd_size, cd_offset,
     _comment_len) = struct.unpack_from("<HHHHIIH", blob, eocd_pos + 4)
    if cd_offset + cd_size > eocd_pos:
        return _finish([Violation("E_CORRUPT_CONTAINER", "<central-directory>")],
                       warnings, {"entries_scanned": 0})
    entries, err = _parse_central_directory(blob, cd_offset, cd_size, _total_entries)
    if err is not None:
        return _finish([Violation(err, "<central-directory>")], warnings,
                       {"entries_scanned": 0})
    stats = {"entries_scanned": len(entries)}

    # ---- Phase B: metadata pre-scan ---------------------------------------
    # R-LIMIT-1 (entry count)
    if len(entries) > LIMITS["MAX_ENTRIES"]:
        violations.append(Violation("E_TOO_MANY_ENTRIES", f"<{len(entries)} entries>"))

    bytes_declared = 0
    for entry in entries:
        display = repr(entry.name_bytes)[:80]

        # R-INTEG-3: encryption flag (checked before anything else per
        # section 4.3 rationale: mode bits decide materialization)
        if entry.flags & 0x1:
            violations.append(Violation("E_ENCRYPTED", display))

        # R-MODE-1/2/3: Unix type bits
        mode_high = (entry.ext_attr >> 16) & 0xFFFF
        if mode_high:
            ftype = mode_high & S_IFMT
            if ftype == S_IFLNK:
                violations.append(Violation("E_SYMLINK", display))
            elif ftype not in (S_IFREG, S_IFDIR):
                violations.append(Violation("E_SPECIAL_MEMBER", display))

        # R-ENC-1/2 + R-PATH-*: decode and normalize the name
        decoded = _decode_name(entry, warnings)
        if decoded is None:
            violations.append(Violation("E_CORRUPT_METADATA", display))
            continue
        entry.name, _enc = decoded
        entry.is_dir = entry.name.endswith("/")
        violations.extend(
            Violation(code, entry.name) for code in _path_violations(entry.name))

        # R-LIMIT-2/4: per-entry declared size and ratio (directories carry
        # usize 0 and are naturally exempt)
        if entry.usize > LIMITS["MAX_FILE_UNCOMPRESSED"]:
            violations.append(Violation("E_SINGLE_FILE_TOO_LARGE", entry.name))
        if (not entry.is_dir and entry.csize >= LIMITS["RATIO_EXEMPT_BELOW"]
                and entry.csize > 0
                and entry.usize / entry.csize > LIMITS["MAX_COMPRESSION_RATIO"]):
            violations.append(Violation("E_RATIO_BOMB", entry.name))

        bytes_declared += entry.usize

        # Opaque nested archives (R-LIMIT-6): noted, never recursed into.
        if entry.name.lower().endswith(ARCHIVE_SUFFIXES):
            warnings.append("inner_archives_are_opaque")

    # R-LIMIT-3: total declared size
    if bytes_declared > LIMITS["MAX_TOTAL_UNCOMPRESSED"]:
        violations.append(Violation("E_TOTAL_SIZE_EXCEEDED", "<archive>"))
    stats["bytes_declared"] = bytes_declared

    if violations:
        return _finish(violations, warnings, stats)

    # ---- Phase C: streamed extraction with incremental caps ---------------
    running = 0
    for entry in entries:
        off = entry.offset
        if off + 30 > len(blob) or blob[off:off + 4] != b"PK\x03\x04":
            violations.append(Violation("E_CORRUPT_CONTAINER", entry.name))
            break
        # local header: sig(4) ver(2) | flags(2) method(2) time(2)+date(2)
        # -> read as one discarded I | crc(4) csize(4) usize(4) nlen(2) elen(2)
        lflags, lmethod, _ltd, lcrc, lcsize, lusize, lnlen, lelen = (
            struct.unpack_from("<HHIIIIHH", blob, off + 6))
        lname = blob[off + 30:off + 30 + lnlen]
        # R-INTEG-4: local header must agree with the central directory
        if (lname != entry.name_bytes or lmethod != entry.method
                or lcrc != entry.crc or lcsize != entry.csize or lusize != entry.usize):
            violations.append(Violation("E_HEADER_MISMATCH", entry.name))
            break
        data_start = off + 30 + lnlen + lelen
        data_end = data_start + entry.csize
        if data_end > len(blob):
            violations.append(Violation("E_TRUNCATED", entry.name))
            break
        payload = blob[data_start:data_end]

        # inflate with caps enforced incrementally (R-LIMIT-5a)
        produced = bytearray()
        truncated = False
        if entry.method == 0:
            # stored: the bytes are the content; a length mismatch is caught
            # by the exact-size check below
            produced = bytearray(payload)
        elif entry.method == 8:
            d = zlib.decompressobj(-15)
            try:
                produced = bytearray(d.decompress(payload, entry.usize + 1))
            except zlib.error:
                violations.append(Violation("E_CORRUPT_METADATA", entry.name))
                break
            if not d.eof and len(produced) <= entry.usize:
                truncated = True
        else:
            violations.append(Violation("E_CORRUPT_METADATA", entry.name))
            break

        if len(produced) > entry.usize:
            violations.append(Violation("E_SIZE_MISMATCH", entry.name))
            break
        if running + len(produced) > LIMITS["MAX_TOTAL_UNCOMPRESSED"]:
            violations.append(Violation("E_TOTAL_SIZE_EXCEEDED", entry.name))
            break
        if truncated:
            violations.append(Violation("E_TRUNCATED", entry.name))
            break
        # R-INTEG-6/R-LIMIT-5b/R-INTEG-5 (stream order)
        if len(produced) != entry.usize:
            violations.append(Violation("E_SIZE_MISMATCH", entry.name))
            break
        if zlib.crc32(bytes(produced)) & 0xFFFFFFFF != entry.crc:
            violations.append(Violation("E_CRC_MISMATCH", entry.name))
            break
        running += len(produced)

    stats["bytes_streamed"] = running
    return _finish(violations, warnings, stats)


# ---------------------------------------------------------------------------
# Bare-text path (section 8)
# ---------------------------------------------------------------------------
def validate_txt(blob: bytes) -> Result:
    stats = {"bytes": len(blob)}
    if len(blob) > LIMITS["MAX_FILE_UNCOMPRESSED"]:  # R-TXT-1
        return _finish([Violation("E_SINGLE_FILE_TOO_LARGE", "<file>")], [], stats)
    if len(blob) == 0:  # R-TXT-2
        return _finish([Violation("E_EMPTY", "<file>")], [], stats)
    head = blob[:4096]
    has_utf16_bom = blob.startswith((b"\xff\xfe", b"\xfe\xff"))
    # R-TXT-3: a NUL byte means binary -- except under a UTF-16 BOM, whose
    # encoding legitimately interleaves zero bytes (those go to R-TXT-4).
    if (b"\x00" in head and not has_utf16_bom) or any(
            blob.startswith(m) for m in TEXT_MAGIC_PREFIXES):
        return _finish([Violation("E_NOT_TEXT", "<file>")], [], stats)  # R-TXT-3
    if blob.startswith(b"\xef\xbb\xbf"):  # R-TXT-4
        codec = "utf-8-sig"
    elif blob.startswith(b"\xff\xfe"):
        codec = "utf-16-le"
        blob2 = blob[2:]
    elif blob.startswith(b"\xfe\xff"):
        codec = "utf-16-be"
        blob2 = blob[2:]
    else:
        codec = "utf-8"
        blob2 = blob
    try:
        text = blob2.decode(codec) if codec != "utf-8-sig" else blob.decode(codec)
    except UnicodeDecodeError:
        return _finish([Violation("E_TEXT_DECODING", "<file>")], [], stats)
    warnings = []
    if not PARSEABLE_LINE_RE.search(text):  # R-TXT-5
        warnings.append("unparsable_lines=all")
    return _finish([], warnings, stats)


# ---------------------------------------------------------------------------
# Public API
# ---------------------------------------------------------------------------
def validate_file(path: Path) -> Result:
    blob = path.read_bytes()
    if path.suffix.lower() == ".zip":
        return validate_zip(blob)
    return validate_txt(blob)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("file", type=Path)
    ap.add_argument("--compact", action="store_true", help="one-line verdict")
    args = ap.parse_args()
    result = validate_file(args.file)
    if args.compact:
        print(f"{args.file.name}: {result.decision} "
              f"{result.primary_code or '-'} "
              f"[{', '.join(result.warnings) or '-'}]")
    else:
        print(json.dumps(result.to_dict(), indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
