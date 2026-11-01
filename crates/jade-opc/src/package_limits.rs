//! Resource budgets for OOXML ZIP input and decompressed parts.
//!
//! A `.docx` / `.xlsx` / `.pptx` is a ZIP, and a ZIP can claim — or actually
//! inflate to — far more than it weighs: a few kilobytes of deflate stream
//! expand to gigabytes (a "ZIP bomb"). Every package these crates open comes
//! from somewhere we do not control (an upload, a URL, an MCP tool argument), so
//! the container is the place to bound it once, for every format.
//!
//! This used to live in `jade-docx-reader` and only DOCX went through it; the
//! XLSX / PPTX path ([`crate::OpcPackage::open`]) inflated every entry without
//! a cap. The budgets are shared so the formats cannot drift apart again.

use std::io::{Cursor, Read};

/// Upper bounds applied while opening a package.
///
/// The defaults are browser-safe: the editors load packages inside a WebAssembly
/// heap, so they must fail with a message rather than abort on allocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageLoadLimits {
    /// Size of the compressed archive itself.
    pub max_archive_bytes: u64,
    /// Number of ZIP entries (directories included).
    pub max_entries: usize,
    /// Inflated size of any single part.
    pub max_part_bytes: u64,
    /// Inflated size of all parts together.
    pub max_total_bytes: u64,
}

impl Default for PackageLoadLimits {
    fn default() -> Self {
        Self {
            max_archive_bytes: 128 * 1024 * 1024,
            max_entries: 10_000,
            max_part_bytes: 64 * 1024 * 1024,
            max_total_bytes: 256 * 1024 * 1024,
        }
    }
}

/// A package exceeded one of its [`PackageLoadLimits`].
///
/// The message keeps the `package limit exceeded: <resource>` shape the DOCX
/// editor has always surfaced, because the browser tests and users match on it.
#[derive(Debug, thiserror::Error)]
#[error("package limit exceeded: {resource} ({actual} > {limit})")]
pub struct PackageLimitError {
    pub resource: &'static str,
    pub actual: u64,
    pub limit: u64,
}

fn check_limit(resource: &'static str, actual: u64, limit: u64) -> Result<(), PackageLimitError> {
    if actual > limit {
        Err(PackageLimitError {
            resource,
            actual,
            limit,
        })
    } else {
        Ok(())
    }
}

/// Preflight ZIP metadata without inflating parts. This is not XML validation.
/// The loader also checks actual output lengths when it decompresses each part.
pub fn validate_package(data: &[u8], limits: PackageLoadLimits) -> crate::Result<()> {
    open_checked_archive(data, limits).map(|_| ())
}

/// Open `data` as a ZIP after checking the archive size, the entry count and
/// the sizes the central directory declares.
///
/// The declared sizes are attacker-controlled, so passing here is necessary but
/// not sufficient: read each entry through [`read_part`], which bounds what the
/// decompressor actually produces.
pub fn open_checked_archive(
    data: &[u8],
    limits: PackageLoadLimits,
) -> crate::Result<zip::ZipArchive<Cursor<&[u8]>>> {
    check_limit("archive bytes", data.len() as u64, limits.max_archive_bytes)?;
    let mut archive = zip::ZipArchive::new(Cursor::new(data))?;
    check_limit("entries", archive.len() as u64, limits.max_entries as u64)?;
    let mut total = 0u64;
    for index in 0..archive.len() {
        let file = archive.by_index_raw(index)?;
        check_limit("part bytes", file.size(), limits.max_part_bytes)?;
        // Compare against remaining capacity before addition to avoid overflow.
        if file.size() > limits.max_total_bytes.saturating_sub(total) {
            return Err(PackageLimitError {
                resource: "expanded bytes",
                actual: total.saturating_add(file.size()),
                limit: limits.max_total_bytes,
            }
            .into());
        }
        total += file.size();
    }
    Ok(archive)
}

/// Count `size` more expanded bytes against the budgets, as
/// [`read_part`] counts what it inflates.
pub(crate) fn account(size: u64, limits: PackageLoadLimits, total: &mut u64) -> crate::Result<()> {
    check_limit("part bytes", size, limits.max_part_bytes)?;
    let remaining = limits.max_total_bytes.saturating_sub(*total);
    if size > remaining {
        return Err(PackageLimitError {
            resource: "expanded bytes",
            actual: total.saturating_add(size),
            limit: limits.max_total_bytes,
        }
        .into());
    }
    *total += size;
    Ok(())
}

/// Inflate one entry, reading at most one byte past what the budgets allow.
///
/// `total` is the running inflated size of the parts read so far; it is only
/// advanced when the part fits, so a failed read leaves it untouched.
pub fn read_part(
    reader: impl Read,
    limits: PackageLoadLimits,
    total: &mut u64,
) -> crate::Result<Vec<u8>> {
    let remaining = limits.max_total_bytes.saturating_sub(*total);
    let cap = limits.max_part_bytes.min(remaining);
    let mut contents = Vec::new();
    reader
        .take(cap.saturating_add(1))
        .read_to_end(&mut contents)?;
    check_limit("part bytes", contents.len() as u64, limits.max_part_bytes)?;
    if contents.len() as u64 > remaining {
        return Err(PackageLimitError {
            resource: "expanded bytes",
            actual: total.saturating_add(contents.len() as u64),
            limit: limits.max_total_bytes,
        }
        .into());
    }
    *total += contents.len() as u64;
    Ok(contents)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OpcError;

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
            let OpcError::Limit(limit) = error else {
                panic!("expected a limit error, got {error}");
            };
            assert_eq!(limit.resource, resource);
            assert_eq!(input.position(), expected_read);
            assert_eq!(total, used, "failed reads must not commit budget state");
        }
    }
}
