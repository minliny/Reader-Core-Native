//! Canonical-hash stability tests for [`reader_local_book::local_book_stable_checksum`].
//!
//! The checksum is the stable identity used for local-book book ids, chapter
//! ids, and resource ids. Its output is part of the serialized DTO surface, so
//! any drift in algorithm or framing would silently change every stored id.
//! These tests pin known input → output vectors so a future change to the
//! hashing function is caught here rather than discovered after a snapshot
//! round-trip changes shape.
//!
//! Vectors are standard FNV-1a 64-bit over the `|`-joined parts, emitted as
//! `fnv1a64:<16 hex lowercase>`.

use reader_local_book::local_book_stable_checksum;

#[test]
fn empty_parts_yield_offset_basis() {
    // FNV-1a 64-bit offset basis, the canonical empty-input result.
    assert_eq!(local_book_stable_checksum(&[]), "fnv1a64:cbf29ce484222325");
}

#[test]
fn single_part_pins_known_vector() {
    assert_eq!(
        local_book_stable_checksum(&["abc"]),
        "fnv1a64:e71fa2190541574b"
    );
    assert_eq!(
        local_book_stable_checksum(&["local-book"]),
        "fnv1a64:927f58330f0d50da"
    );
}

#[test]
fn joined_parts_use_pipe_separator() {
    // `["a","b","c"]` hashes the joined `a|b|c`, not the concatenation `abc`.
    assert_ne!(
        local_book_stable_checksum(&["a", "b", "c"]),
        local_book_stable_checksum(&["abc"])
    );
    assert_eq!(
        local_book_stable_checksum(&["a", "b", "c"]),
        "fnv1a64:e689323f6a5f21c7"
    );
}

#[test]
fn multibyte_utf8_hashes_over_bytes() {
    // CJK chapter headings must hash over their UTF-8 byte sequence, not a
    // codepoint or percent-escaped form.
    assert_eq!(
        local_book_stable_checksum(&["第1章"]),
        "fnv1a64:a626b3e5ac78f4d5"
    );
}

#[test]
fn checksum_is_deterministic_across_calls() {
    let a = local_book_stable_checksum(&["epub-resource", "book-1", "ch1", "OPS/ch1.xhtml"]);
    let b = local_book_stable_checksum(&["epub-resource", "book-1", "ch1", "OPS/ch1.xhtml"]);
    assert_eq!(a, b);
    assert!(a.starts_with("fnv1a64:"));
    assert_eq!(a.len(), "fnv1a64:".len() + 16);
}
