//! Resource budgets for DOCX ZIP input and decompressed parts.
//!
//! The budgets themselves live in [`jade_opc::package_limits`] so DOCX, XLSX and
//! PPTX share one definition. This module keeps the DOCX loader's error shape:
//! it reports `Box<dyn Error>` and callers downcast to [`PackageLimitError`].

use std::io::{Cursor, Read};

pub use jade_opc::package_limits::{PackageLimitError, PackageLoadLimits};

/// Unwrap the container's error so a limit violation is boxed as
/// [`PackageLimitError`] itself, as it was before the budgets moved.
fn boxed(error: jade_opc::OpcError) -> Box<dyn std::error::Error> {
    match error {
        jade_opc::OpcError::Limit(limit) => Box::new(limit),
        jade_opc::OpcError::Archive(error) => Box::new(error),
        jade_opc::OpcError::Io(error) => Box::new(error),
        other => Box::new(other),
    }
}

/// Preflight ZIP metadata without inflating parts. This is not XML validation.
/// The loader also checks actual output lengths when it decompresses each part.
pub fn validate_package(
    data: &[u8],
    limits: PackageLoadLimits,
) -> Result<(), Box<dyn std::error::Error>> {
    jade_opc::package_limits::validate_package(data, limits).map_err(boxed)
}

pub(crate) fn open_checked_archive(
    data: &[u8],
    limits: PackageLoadLimits,
) -> Result<zip::ZipArchive<Cursor<&[u8]>>, Box<dyn std::error::Error>> {
    jade_opc::package_limits::open_checked_archive(data, limits).map_err(boxed)
}

pub(crate) fn read_part(
    reader: impl Read,
    limits: PackageLoadLimits,
    total: &mut u64,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    jade_opc::package_limits::read_part(reader, limits, total).map_err(boxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OpenXmlPackage;
    use std::{
        future::Future,
        io::{Cursor, Write},
        task::{Context, Poll, Waker},
    };

    fn zip(parts: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in parts {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    fn load(
        bytes: &[u8],
        limits: PackageLoadLimits,
    ) -> Result<OpenXmlPackage, Box<dyn std::error::Error>> {
        let mut future = std::pin::pin!(OpenXmlPackage::load_with_limits(bytes, limits));
        match future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
        {
            Poll::Ready(result) => result,
            Poll::Pending => panic!("package loading must not need external I/O"),
        }
    }

    fn assert_limit(bytes: &[u8], limits: PackageLoadLimits, resource: &str) {
        let error = load(bytes, limits).unwrap_err();
        let limit = error
            .downcast_ref::<PackageLimitError>()
            .expect("typed limit error");
        assert_eq!(limit.resource, resource);
        assert!(limit.actual > limit.limit);
    }

    #[test]
    fn archive_budget_is_checked_before_zip_parsing() {
        assert_limit(
            &[0; 5],
            PackageLoadLimits {
                max_archive_bytes: 4,
                ..Default::default()
            },
            "archive bytes",
        );
    }

    #[test]
    fn entry_budget_rejects_many_small_parts() {
        let bytes = zip(&[("a.xml", b"a"), ("b.xml", b"b")]);
        assert_limit(
            &bytes,
            PackageLoadLimits {
                max_entries: 1,
                ..Default::default()
            },
            "entries",
        );
    }

    #[test]
    fn part_budget_rejects_compressed_expansion() {
        let bytes = zip(&[("word/document.xml", &[b'x'; 4096])]);
        assert_limit(
            &bytes,
            PackageLoadLimits {
                max_part_bytes: 4095,
                ..Default::default()
            },
            "part bytes",
        );
    }

    #[test]
    fn total_budget_applies_across_parts() {
        let bytes = zip(&[("a.xml", b"123"), ("b.xml", b"456")]);
        assert_limit(
            &bytes,
            PackageLoadLimits {
                max_part_bytes: 3,
                max_total_bytes: 5,
                ..Default::default()
            },
            "expanded bytes",
        );
    }

    #[test]
    fn exact_boundaries_preserve_all_parts_and_original_size() {
        let bytes = zip(&[("a.xml", b"123"), ("b.xml", b"456")]);
        let package = load(
            &bytes,
            PackageLoadLimits {
                max_archive_bytes: bytes.len() as u64,
                max_entries: 2,
                max_part_bytes: 3,
                max_total_bytes: 6,
            },
        )
        .unwrap();
        assert_eq!(package.files.len(), 2);
        assert_eq!(package.get("a.xml").unwrap(), b"123");
        assert_eq!(package.get("b.xml").unwrap(), b"456");
        assert_eq!(package.original_size_bytes, Some(bytes.len()));
    }

    #[test]
    fn actual_output_is_bounded_even_without_trustworthy_metadata() {
        for (part_limit, total_limit, used, resource, expected_read) in [
            (3, 100, 0, "part bytes", 4),
            (100, 5, 3, "expanded bytes", 3),
        ] {
            let mut input = Cursor::new(b"123456789");
            let mut total = used;
            let error = read_part(
                &mut input,
                PackageLoadLimits {
                    max_part_bytes: part_limit,
                    max_total_bytes: total_limit,
                    ..Default::default()
                },
                &mut total,
            )
            .unwrap_err();
            assert_eq!(
                error.downcast_ref::<PackageLimitError>().unwrap().resource,
                resource
            );
            assert_eq!(input.position(), expected_read);
            assert_eq!(total, used, "failed reads must not commit budget state");
        }
    }

    #[test]
    fn repository_docx_fixtures_fit_default_budgets() {
        let directory =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures");
        let mut checked = 0;
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|ext| ext == "docx")
                && !path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("._")
            {
                let bytes = std::fs::read(&path).unwrap();
                load(&bytes, PackageLoadLimits::default())
                    .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
                checked += 1;
            }
        }
        assert!(checked > 0);
    }
}
