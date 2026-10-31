use crate::utils::Error;
use std::collections::HashSet;
use std::io::Cursor;
use std::path::{Component, Path, PathBuf};
use zip::read::{ExtractOptions, SymlinkExtractAction};

/// Cheap zip-bomb guard for untrusted uploads such as `POST /api/jmix`.
const MAX_ZIP_ENTRIES: usize = 10_000;
/// Declared uncompressed size cap. Checked from central-directory sizes, not after inflate.
const MAX_UNCOMPRESSED_BYTES: u64 = 4 * 1024 * 1024 * 1024;

pub fn extract_zip(bytes: &[u8], dest: &Path) -> Result<(), Error> {
    let reader = Cursor::new(bytes);
    let mut zip =
        zip::ZipArchive::new(reader).map_err(|e| Error::from(format!("zip open error: {e}")))?;

    if zip.len() > MAX_ZIP_ENTRIES {
        return Err(Error::from(format!(
            "zip rejected: too many entries ({} > {})",
            zip.len(),
            MAX_ZIP_ENTRIES
        )));
    }

    if zip.has_duplicate_names() {
        return Err(Error::from("zip rejected: duplicate entry names"));
    }

    let mut seen_paths: HashSet<PathBuf> = HashSet::new();
    let mut file_paths: HashSet<PathBuf> = HashSet::new();
    let mut declared_uncompressed: u64 = 0;

    for i in 0..zip.len() {
        let file = zip
            .by_index(i)
            .map_err(|e| Error::from(format!("zip idx error: {e}")))?;

        if file.is_symlink() {
            return Err(Error::from("zip rejected: symlink entries are not allowed"));
        }

        if raw_name_is_absolute(file.name_raw()) {
            let raw = String::from_utf8_lossy(file.name_raw()).into_owned();
            return Err(Error::from(format!("zip rejected: unsafe path {raw:?}")));
        }

        let Some(enclosed) = file.enclosed_name() else {
            let raw = file
                .name()
                .map(|n| n.into_owned())
                .unwrap_or_else(|_| String::from("<invalid name>"));
            return Err(Error::from(format!("zip rejected: unsafe path {raw:?}")));
        };

        if enclosed.as_os_str().is_empty()
            || enclosed
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(Error::from(format!(
                "zip rejected: unsafe path {}",
                enclosed.display()
            )));
        }

        if !seen_paths.insert(enclosed.clone()) {
            return Err(Error::from(format!(
                "zip rejected: conflicting entry names ({})",
                enclosed.display()
            )));
        }

        if !file.is_dir() {
            let mut ancestor = enclosed.clone();
            while let Some(parent) = ancestor.parent() {
                if parent.as_os_str().is_empty() {
                    break;
                }
                if file_paths.contains(parent) {
                    return Err(Error::from(format!(
                        "zip rejected: conflicting entry names ({})",
                        enclosed.display()
                    )));
                }
                ancestor = parent.to_path_buf();
            }
            file_paths.insert(enclosed);

            declared_uncompressed = declared_uncompressed.saturating_add(file.size());
            if declared_uncompressed > MAX_UNCOMPRESSED_BYTES {
                return Err(Error::from(
                    "zip rejected: uncompressed size limit exceeded",
                ));
            }
        }
    }

    let options = ExtractOptions::default()
        .enforce_header_consistency(true)
        .with_size_limit(MAX_UNCOMPRESSED_BYTES)
        .symlink_action(SymlinkExtractAction::NoExtract);

    zip.extract_with_options(dest, options)
        .map_err(|e| Error::from(format!("zip extract error: {e}")))?;
    Ok(())
}

fn raw_name_is_absolute(name: &[u8]) -> bool {
    name.starts_with(b"/")
        || name.starts_with(b"\\")
        || (name.len() >= 2 && name[1] == b':' && name[0].is_ascii_alphabetic())
}

#[cfg(test)]
mod tests;
