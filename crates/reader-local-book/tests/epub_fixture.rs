//! Fixture-driven EPUB package-metadata stability tests.
//!
//! Loads a minimal OPF package file and pins the Core-owned metadata artifact.
//! The OPF is parsed by the std-only extractor; this test guards the DTO shape
//! and JSON round-trip so a future change to the XML walker is caught before
//! it can drift stored EPUB library snapshots.

use reader_local_book::{extract_epub_package_metadata, LocalBookEpubPackageMetadataRequest};

const OPF: &str = include_str!("fixtures/epub/minimal.opf");

#[test]
fn fixture_opf_extracts_stable_package_metadata() {
    let artifact = extract_epub_package_metadata(&LocalBookEpubPackageMetadataRequest {
        opf_xml: OPF.into(),
    })
    .expect("valid OPF must extract");

    assert!(!artifact.fail_closed);
    assert_eq!(
        artifact.metadata_identifier.as_deref(),
        Some("urn:uuid:fixture-epub-001")
    );
    assert_eq!(
        artifact.metadata_title.as_deref(),
        Some("Fixture EPUB Title")
    );
    assert_eq!(artifact.metadata_author.as_deref(), Some("Fixture Author"));
    assert_eq!(artifact.metadata_language.as_deref(), Some("zh-CN"));
    assert_eq!(
        artifact.package_unique_identifier_id.as_deref(),
        Some("bookid")
    );
    assert!(artifact.diagnostics_summary.is_empty());

    // Stable DTO output: serialize → deserialize must be identity.
    let json = serde_json::to_string(&artifact).unwrap();
    let back =
        serde_json::from_str::<reader_local_book::LocalBookEpubPackageMetadataArtifact>(&json)
            .unwrap();
    assert_eq!(back, artifact);
    assert!(json.contains(r#""metadataIdentifier":"urn:uuid:fixture-epub-001""#));
}

#[test]
fn fixture_opf_without_identifier_fails_closed_with_diagnostic() {
    let opf = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="bookid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>No Identifier Here</dc:title>
    <dc:language>en</dc:language>
  </metadata>
</package>"#;

    let artifact = extract_epub_package_metadata(&LocalBookEpubPackageMetadataRequest {
        opf_xml: opf.into(),
    })
    .expect("missing identifier is a fail-closed artifact, not an error");

    assert!(artifact.fail_closed);
    assert!(artifact.metadata_identifier.is_none());
    assert!(artifact
        .diagnostics_summary
        .iter()
        .any(|d| d.contains("missing_metadata_identifier")));
}
