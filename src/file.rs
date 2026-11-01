use crate::utils::Error;
use std::collections::HashSet;
use std::fs;
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

        // bsdtar writes a `./` root entry. It creates nothing, so there is nothing to check.
        if enclosed.as_os_str().is_empty() && file.is_dir() {
            continue;
        }

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
            file_paths.insert(enclosed);

            declared_uncompressed = declared_uncompressed.saturating_add(file.size());
            if declared_uncompressed > MAX_UNCOMPRESSED_BYTES {
                return Err(Error::from(
                    "zip rejected: uncompressed size limit exceeded",
                ));
            }
        }
    }

    // Checked after the loop so the result does not depend on entry order.
    for path in &seen_paths {
        if path.ancestors().skip(1).any(|a| file_paths.contains(a)) {
            return Err(Error::from(format!(
                "zip rejected: conflicting entry names ({})",
                path.display()
            )));
        }
    }

    // `with_size_limit` is left off: it rejects every archive that uses data descriptors.
    // The loop above enforces the same cap, and zip stops each entry at its declared size.
    let options = ExtractOptions::default()
        .enforce_header_consistency(true)
        .symlink_action(SymlinkExtractAction::NoExtract);

    let extracted = zip
        .extract_with_options(dest, options)
        .map_err(|e| Error::from(format!("zip extract error: {e}")));
    // Runs on failure too, because a mode 000 directory would defeat temp dir cleanup.
    let reset = reset_permissions(dest);
    extracted?;
    reset
}

/// Replaces modes taken from the archive, so an upload cannot set server permissions.
/// Uses the modes a plain file create gives under the usual 022 umask.
#[cfg(unix)]
fn reset_permissions(root: &Path) -> Result<(), Error> {
    use std::os::unix::fs::PermissionsExt;

    let fail = |path: &Path, e: std::io::Error| {
        Error::from(format!(
            "zip permission reset error at {}: {e}",
            path.display()
        ))
    };

    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let metadata = fs::symlink_metadata(&path).map_err(|e| fail(&path, e))?;
        if metadata.is_dir() {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
                .map_err(|e| fail(&path, e))?;
            for entry in fs::read_dir(&path).map_err(|e| fail(&path, e))? {
                pending.push(entry.map_err(|e| fail(&path, e))?.path());
            }
        } else if metadata.is_file() {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o644))
                .map_err(|e| fail(&path, e))?;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn reset_permissions(_root: &Path) -> Result<(), Error> {
    Ok(())
}

fn raw_name_is_absolute(name: &[u8]) -> bool {
    name.starts_with(b"/")
        || name.starts_with(b"\\")
        || (name.len() >= 2 && name[1] == b':' && name[0].is_ascii_alphabetic())
}

#[cfg(test)]
mod tests;
