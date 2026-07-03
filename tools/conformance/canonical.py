"""Canonical JSON + hashing for the beaterOS conformance suite.

`final.md` (sections 13.11, 26) requires tamper-evident, hash-linked journals
and receipts. For those hashes to be *verifiable across languages* -- the Rust
`beater-os-core` crate today, a future TypeScript CLI or dashboard tomorrow --
every implementation must agree on one canonical byte layout for a value before
it is hashed.

This module pins that layout to a JSON Canonicalization Scheme (JCS, RFC 8785)
style encoding: object keys sorted by Unicode code point, compact separators,
UTF-8, no insignificant whitespace. It is intentionally dependency-free so the
gate runs anywhere Python 3.9+ is available.

NOTE FOR CROSS-IMPLEMENTATION CONVERGENCE: the Rust core currently hashes with
serde's *struct-declaration* field order, which is not canonical across
languages. This is tracked as an open coordination item in `AGENTS.md`; the
recommendation is for every implementation (including the Rust core) to adopt
this JCS layout so digests match byte-for-byte.
"""

from __future__ import annotations

import hashlib
import json
from typing import Any

# Matches `GENESIS_HASH` in crates/beater-os-core/src/hash.rs.
GENESIS_HASH = "0" * 64


def canonical_bytes(value: Any) -> bytes:
    """Return the canonical UTF-8 byte encoding of a JSON value (RFC 8785 style)."""
    return json.dumps(
        value,
        ensure_ascii=False,
        allow_nan=False,
        separators=(",", ":"),
        sort_keys=True,
    ).encode("utf-8")


def sha256_hex(value: Any) -> str:
    """SHA-256 (hex) over the canonical encoding of `value`."""
    return hashlib.sha256(canonical_bytes(value)).hexdigest()


def hash_preimage(record: dict[str, Any], omit_field: str) -> dict[str, Any]:
    """Return a copy of `record` without `omit_field` (the field being computed)."""
    return {k: v for k, v in record.items() if k != omit_field}
