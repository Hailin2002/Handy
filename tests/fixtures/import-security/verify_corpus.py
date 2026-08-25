#!/usr/bin/env python3
"""Mechanical gate-checker for the import-security fixture corpus (G2/G3).

Proves, in order:
  1. REPRODUCIBLE   -- regenerating with generate_fixtures.py into a fresh temp
                       dir reproduces manifest.json byte-for-byte and every
                       corpus file byte-for-byte (deterministic generator).
  2. INTACT         -- every committed fixture's SHA-256 matches its
                       manifest.json entry.
  3. VERDICTS       -- reference_validator.py (executable form of spec §3–§8)
                       returns exactly the expected decision / primary_code /
                       warnings for every fixture.
  4. GENUINE CRYPTO -- the ZipCrypto fixture round-trips: decrypting member 1
                       with the documented password yields the chat plaintext,
                       matching CRC-32, and a correct ZipCrypto check byte --
                       i.e. real well-formed encryption, not a flipped flag.

Stdlib only. Run from this directory:  python verify_corpus.py
Exit code 0 = all gates hold; any failure prints details and exits 1.
"""
from __future__ import annotations

import hashlib
import json
import struct
import sys
import zlib
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import generate_fixtures as gen  # noqa: E402
import reference_validator as rv  # noqa: E402


def check_reproducible(manifest: dict) -> list[str]:
    errors: list[str] = []
    # Scratch dir lives next to this script (not the system temp area): keeps
    # the check self-contained and avoids cross-volume surprises.
    scratch = HERE / ".verify-tmp"
    if scratch.exists():
        import shutil
        shutil.rmtree(scratch)
    try:
        out = scratch
        out.mkdir(parents=True)
        gen.generate(out)
        regenerated_manifest = json.loads((out / "manifest.json").read_text("utf-8"))
        if regenerated_manifest != manifest:
            errors.append(
                "regenerated manifest.json differs from committed manifest.json "
                "(generator is not reproducible against the committed corpus)")
        for entry in manifest["fixtures"]:
            rel = entry["file"]
            committed = HERE / rel
            fresh = out / rel
            if not committed.exists():
                errors.append(f"missing committed fixture: {rel}")
                continue
            if not fresh.exists():
                errors.append(f"generator did not produce: {rel}")
                continue
            if committed.read_bytes() != fresh.read_bytes():
                errors.append(f"fixture not reproducible byte-for-byte: {rel}")
    finally:
        if scratch.exists():
            import shutil
            shutil.rmtree(scratch, ignore_errors=True)
    return errors


def check_intact(manifest: dict) -> list[str]:
    errors: list[str] = []
    seen = set()
    for entry in manifest["fixtures"]:
        path = HERE / entry["file"]
        seen.add(entry["file"])
        if not path.exists():
            errors.append(f"missing fixture: {entry['file']}")
            continue
        data = path.read_bytes()
        if len(data) != entry["bytes"]:
            errors.append(
                f"{entry['file']}: size {len(data)} != manifest {entry['bytes']}")
        digest = hashlib.sha256(data).hexdigest()
        if digest != entry["sha256"]:
            errors.append(f"{entry['file']}: sha256 mismatch")
        exp = entry["expected"]
        if exp["decision"] == "REJECT" and not exp["primary_code"]:
            errors.append(f"{entry['file']}: REJECT without primary_code")
        if exp["decision"] == "ACCEPT" and exp["primary_code"]:
            errors.append(f"{entry['file']}: ACCEPT must have null primary_code")
    # no stray files in corpus/
    for f in sorted((HERE / "corpus").iterdir()):
        if f.name == "__pycache__":
            continue
        rel = f"corpus/{f.name}"
        if rel not in seen:
            errors.append(f"stray corpus file without manifest entry: {rel}")
    return errors


def check_verdicts(manifest: dict) -> list[str]:
    errors: list[str] = []
    for entry in manifest["fixtures"]:
        path = HERE / entry["file"]
        result = rv.validate_file(path)
        exp = entry["expected"]
        if result.decision != exp["decision"]:
            errors.append(
                f"{entry['file']}: decision {result.decision} != expected "
                f"{exp['decision']} (validator said {result.primary_code})")
            continue
        if result.primary_code != exp["primary_code"]:
            errors.append(
                f"{entry['file']}: primary_code {result.primary_code} != expected "
                f"{exp['primary_code']}")
        got_warnings = sorted(result.warnings)
        want_warnings = sorted(exp["warnings"])
        if got_warnings != want_warnings:
            errors.append(
                f"{entry['file']}: warnings {got_warnings} != expected {want_warnings}")
    return errors


def _zipcrypto_keystream(password: bytes):
    """Mirror of generate_fixtures' cipher, fed with ciphertext for decryption."""
    k0, k1, k2 = 0x12345678, 0x23456789, 0x34567890

    def update(c: int) -> None:
        nonlocal k0, k1, k2
        k0 = zlib.crc32(bytes([c]), k0) & 0xFFFFFFFF
        k1 = (k1 + (k0 & 0xFF)) & 0xFFFFFFFF
        k1 = (k1 * 134775813 + 1) & 0xFFFFFFFF
        k2 = zlib.crc32(bytes([(k1 >> 24) & 0xFF]), k2) & 0xFFFFFFFF

    def dec_byte() -> int:
        t = (k2 | 2) & 0xFFFF
        return ((t * (t ^ 1)) >> 8) & 0xFF

    for c in password:
        update(c)

    def stream(chunk: bytes) -> bytes:
        out = bytearray()
        for c in chunk:
            p = c ^ dec_byte()
            out.append(p)
            update(p)
        return bytes(out)

    return stream


def check_genuine_encryption() -> list[str]:
    """attack_encrypted_zipcrypto.zip must be genuinely ZipCrypto'd."""
    errors: list[str] = []
    blob = (HERE / "corpus/attack_encrypted_zipcrypto.zip").read_bytes()
    eocd = blob.rfind(b"PK\x05\x06")
    (_cd_size, cd_offset) = struct.unpack_from("<II", blob, eocd + 12)
    # first (and only) central-directory record at cd_offset
    (_vmade, _vneed, flags, method, _t, _d, crc, csize, usize, nlen, elen,
     _clen, _disk, _iattr, _ext, lho) = struct.unpack_from("<HHHHHHIIIHHHHHII",
                                                           blob, cd_offset + 4)
    if not flags & 0x1:
        errors.append("encrypted fixture: general-purpose bit 0 not set in CD")
    if method != 0:
        errors.append(f"encrypted fixture: unexpected method {method}")
    lh_nlen, lh_elen = struct.unpack_from("<HH", blob, lho + 26)
    name = blob[lho + 30:lho + 30 + lh_nlen]
    data = blob[lho + 30 + lh_nlen + lh_elen:lho + 30 + lh_nlen + lh_elen + csize]
    password = b"correct-horse-battery"
    stream = _zipcrypto_keystream(password)
    plain = stream(data)
    header, content = plain[:12], plain[12:]
    if header[11] != (crc >> 24) & 0xFF:
        errors.append("encrypted fixture: ZipCrypto check byte does not match CRC")
    if len(content) != usize:
        errors.append(f"encrypted fixture: decrypted size {len(content)} != {usize}")
    if zlib.crc32(content) & 0xFFFFFFFF != crc:
        errors.append("encrypted fixture: decrypted plaintext CRC mismatch")
    if not content.startswith(b"WhatsApp Chat mit"):
        errors.append("encrypted fixture: decrypted content is not the chat export")
    if name != b"secret_chat.txt":
        errors.append(f"encrypted fixture: unexpected member name {name!r}")
    return errors


def main() -> int:
    manifest = json.loads((HERE / "manifest.json").read_text("utf-8"))
    limits = rv.LIMITS
    if manifest["limits"] != limits:
        print("FAIL: manifest limits diverge from reference validator LIMITS")
        return 1
    if manifest["limits"] != gen.LIMITS:
        print("FAIL: manifest limits diverge from generator LIMITS")
        return 1

    failures: list[tuple[str, list[str]]] = []

    errs = check_reproducible(manifest)
    failures.append(("REPRODUCIBLE", errs))
    errs = check_intact(manifest)
    failures.append(("INTACT", errs))
    errs = check_verdicts(manifest)
    failures.append(("VERDICTS", errs))
    failures.append(("GENUINE-CRYPTO", check_genuine_encryption()))

    n = len(manifest["fixtures"])
    ok = True
    for gate, errs in failures:
        status = "PASS" if not errs else "FAIL"
        if errs:
            ok = False
        print(f"[{status}] {gate}")
        for e in errs:
            print(f"       - {e}")
    print(f"\n{n} fixtures checked; "
          f"{'ALL GATES PASS' if ok else 'FAILURES PRESENT'}")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
