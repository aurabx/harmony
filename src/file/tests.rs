use super::*;
use std::fs;
use std::io::{Cursor, Write};
use std::path::Path;
use tempfile::TempDir;
use zip::write::{SimpleFileOptions, ZipWriter};

fn create_test_zip() -> Vec<u8> {
    let mut buffer = Vec::new();
    {
        let mut zip = ZipWriter::new(Cursor::new(&mut buffer));
        let options = SimpleFileOptions::default();

        zip.start_file("test.txt", options).unwrap();
        zip.write_all(b"Hello, World!").unwrap();

        zip.start_file("subdir/nested.txt", options).unwrap();
        zip.write_all(b"Nested content").unwrap();

        zip.add_directory("empty_dir/", options).unwrap();

        zip.finish().unwrap();
    }
    buffer
}

fn stored_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buffer = Vec::new();
    {
        let mut zip = ZipWriter::new(Cursor::new(&mut buffer));
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (name, data) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
    }
    buffer
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = if crc & 1 == 1 { 0xedb8_8320 } else { 0 };
            crc = (crc >> 1) ^ mask;
        }
    }
    !crc
}

fn push_u16(buf: &mut Vec<u8>, v: u16) {
    buf.extend_from_slice(&v.to_le_bytes());
}
fn push_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn append_local(buf: &mut Vec<u8>, name: &str, data: &[u8]) {
    let crc = crc32(data);
    let name_bytes = name.as_bytes();
    buf.extend_from_slice(&[0x50, 0x4b, 0x03, 0x04]);
    push_u16(buf, 20);
    push_u16(buf, 0);
    push_u16(buf, 0);
    push_u16(buf, 0);
    push_u16(buf, 0);
    push_u32(buf, crc);
    push_u32(buf, data.len() as u32);
    push_u32(buf, data.len() as u32);
    push_u16(buf, name_bytes.len() as u16);
    push_u16(buf, 0);
    buf.extend_from_slice(name_bytes);
    buf.extend_from_slice(data);
}

fn append_central(buf: &mut Vec<u8>, name: &str, data: &[u8], local_offset: u32) {
    let crc = crc32(data);
    let name_bytes = name.as_bytes();
    buf.extend_from_slice(&[0x50, 0x4b, 0x01, 0x02]);
    push_u16(buf, 20);
    push_u16(buf, 20);
    push_u16(buf, 0);
    push_u16(buf, 0);
    push_u16(buf, 0);
    push_u16(buf, 0);
    push_u32(buf, crc);
    push_u32(buf, data.len() as u32);
    push_u32(buf, data.len() as u32);
    push_u16(buf, name_bytes.len() as u16);
    push_u16(buf, 0);
    push_u16(buf, 0);
    push_u16(buf, 0);
    push_u16(buf, 0);
    push_u32(buf, 0);
    push_u32(buf, local_offset);
    buf.extend_from_slice(name_bytes);
}

/// Stored ZIP that keeps entry names verbatim. ZipWriter sanitizes some names.
fn raw_stored_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut offsets = Vec::new();
    for (name, data) in entries {
        offsets.push(buf.len() as u32);
        append_local(&mut buf, name, data);
    }
    let cd_start = buf.len() as u32;
    for ((name, data), offset) in entries.iter().zip(offsets) {
        append_central(&mut buf, name, data, offset);
    }
    let cd_size = buf.len() as u32 - cd_start;
    let count = entries.len() as u16;
    buf.extend_from_slice(&[0x50, 0x4b, 0x05, 0x06]);
    push_u16(&mut buf, 0);
    push_u16(&mut buf, 0);
    push_u16(&mut buf, count);
    push_u16(&mut buf, count);
    push_u32(&mut buf, cd_size);
    push_u32(&mut buf, cd_start);
    push_u16(&mut buf, 0);
    buf
}

/// Build a stored ZIP with two central-directory entries that share one name.
fn zip_with_duplicate_names(name: &str, first: &[u8], second: &[u8]) -> Vec<u8> {
    raw_stored_zip(&[(name, first), (name, second)])
}

/// Same-length LFH/CDFH filename mismatch (zip cloaking).
fn zip_with_header_mismatch(lfh_name: &str, cdh_name: &str, data: &[u8]) -> Vec<u8> {
    assert_eq!(lfh_name.len(), cdh_name.len());
    let mut zip = stored_zip(&[(lfh_name, data)]);
    let cdh = zip
        .windows(4)
        .position(|s| s == [0x50, 0x4b, 0x01, 0x02])
        .expect("central directory");
    let name_len = u16::from_le_bytes([zip[cdh + 28], zip[cdh + 29]]) as usize;
    assert_eq!(name_len, cdh_name.len());
    zip[cdh + 46..cdh + 46 + name_len].copy_from_slice(cdh_name.as_bytes());
    zip
}

fn dest_has_escaped(dest: &Path, marker: &str) -> bool {
    dest.parent()
        .map(|parent| parent.join(marker).exists())
        .unwrap_or(false)
}

#[test]
fn test_extract_zip_basic() {
    let zip_data = create_test_zip();
    let temp_dir = TempDir::new().unwrap();

    let result = extract_zip(&zip_data, temp_dir.path());
    assert!(result.is_ok(), "Failed to extract zip: {:?}", result.err());

    let test_file = temp_dir.path().join("test.txt");
    assert!(test_file.exists(), "test.txt should exist");

    let content = fs::read_to_string(&test_file).unwrap();
    assert_eq!(content, "Hello, World!");
}

#[test]
fn test_extract_zip_nested_directory() {
    let zip_data = create_test_zip();
    let temp_dir = TempDir::new().unwrap();

    extract_zip(&zip_data, temp_dir.path()).unwrap();

    let nested_file = temp_dir.path().join("subdir").join("nested.txt");
    assert!(nested_file.exists(), "subdir/nested.txt should exist");

    let content = fs::read_to_string(&nested_file).unwrap();
    assert_eq!(content, "Nested content");
}

#[test]
fn test_extract_zip_empty_directory() {
    let zip_data = create_test_zip();
    let temp_dir = TempDir::new().unwrap();

    extract_zip(&zip_data, temp_dir.path()).unwrap();

    let empty_dir = temp_dir.path().join("empty_dir");
    assert!(empty_dir.exists(), "empty_dir should exist");
    assert!(empty_dir.is_dir(), "empty_dir should be a directory");
}

#[test]
fn test_extract_zip_invalid_data() {
    let temp_dir = TempDir::new().unwrap();
    let invalid_zip = b"not a valid zip file";

    let result = extract_zip(invalid_zip, temp_dir.path());
    assert!(result.is_err(), "Should fail with invalid zip data");
}

#[test]
fn test_extract_zip_empty_data() {
    let temp_dir = TempDir::new().unwrap();
    let empty_data: &[u8] = &[];

    let result = extract_zip(empty_data, temp_dir.path());
    assert!(result.is_err(), "Should fail with empty data");
}

#[test]
fn test_extract_zip_preserves_structure() {
    let zip_data = create_test_zip();
    let temp_dir = TempDir::new().unwrap();

    extract_zip(&zip_data, temp_dir.path()).unwrap();

    assert!(temp_dir.path().join("test.txt").exists());
    assert!(temp_dir.path().join("subdir").is_dir());
    assert!(temp_dir.path().join("subdir").join("nested.txt").exists());
    assert!(temp_dir.path().join("empty_dir").is_dir());
}

#[test]
fn test_extract_zip_multiple_levels() {
    let buffer = stored_zip(&[("level1/level2/level3/deep.txt", b"Deep content")]);

    let temp_dir = TempDir::new().unwrap();
    let result = extract_zip(&buffer, temp_dir.path());
    assert!(result.is_ok());

    let deep_file = temp_dir
        .path()
        .join("level1")
        .join("level2")
        .join("level3")
        .join("deep.txt");
    assert!(deep_file.exists());

    let content = fs::read_to_string(&deep_file).unwrap();
    assert_eq!(content, "Deep content");
}

#[test]
fn test_extract_zip_overwrites_existing() {
    let temp_dir = TempDir::new().unwrap();

    let existing_file = temp_dir.path().join("test.txt");
    fs::write(&existing_file, b"Old content").unwrap();

    let zip_data = create_test_zip();
    let result = extract_zip(&zip_data, temp_dir.path());
    assert!(result.is_ok());

    let content = fs::read_to_string(&existing_file).unwrap();
    assert_eq!(content, "Hello, World!");
}

#[test]
fn test_extract_zip_empty_archive() {
    let mut buffer = Vec::new();
    {
        let zip = ZipWriter::new(Cursor::new(&mut buffer));
        zip.finish().unwrap();
    }

    let temp_dir = TempDir::new().unwrap();
    let result = extract_zip(&buffer, temp_dir.path());
    assert!(result.is_ok(), "Empty zip should extract successfully");
}

#[test]
fn test_extract_zip_rejects_path_traversal() {
    let zip_data = stored_zip(&[("../evil.txt", b"escaped")]);
    let parent = TempDir::new().unwrap();
    let dest = parent.path().join("dest");
    fs::create_dir_all(&dest).unwrap();

    let result = extract_zip(&zip_data, &dest);
    assert!(result.is_err(), "path traversal must be rejected");
    assert!(
        !dest_has_escaped(&dest, "evil.txt"),
        "extractor must not write outside dest"
    );
    assert!(!dest.join("evil.txt").exists());
}

#[test]
fn test_extract_zip_rejects_absolute_path() {
    let zip_data = raw_stored_zip(&[("/tmp/harmony-zip-escape.txt", b"escaped")]);
    let temp_dir = TempDir::new().unwrap();

    let result = extract_zip(&zip_data, temp_dir.path());
    assert!(result.is_err(), "absolute path must be rejected");
    assert!(!Path::new("/tmp/harmony-zip-escape.txt").exists());
    assert!(!temp_dir.path().join("tmp/harmony-zip-escape.txt").exists());
}

#[test]
fn test_extract_zip_rejects_duplicate_entry_names() {
    let zip_data = zip_with_duplicate_names("safe.txt", b"first", b"second");
    let temp_dir = TempDir::new().unwrap();

    let result = extract_zip(&zip_data, temp_dir.path());
    assert!(result.is_err(), "duplicate names must be rejected");
    let err = result.err().unwrap().to_string();
    assert!(
        err.contains("duplicate") || err.contains("conflicting"),
        "unexpected error: {err}"
    );
}

#[test]
fn test_extract_zip_rejects_conflicting_file_and_child() {
    let zip_data = stored_zip(&[("foo", b"file"), ("foo/bar", b"child")]);
    let temp_dir = TempDir::new().unwrap();

    let result = extract_zip(&zip_data, temp_dir.path());
    assert!(result.is_err(), "file/child name conflict must be rejected");
}

#[test]
fn test_extract_zip_rejects_header_inconsistency() {
    let zip_data = zip_with_header_mismatch("doc_report.pdf", "cron_worker.sh", b"cloaked");
    let temp_dir = TempDir::new().unwrap();

    let result = extract_zip(&zip_data, temp_dir.path());
    assert!(
        result.is_err(),
        "LFH/CDFH filename mismatch must be rejected"
    );
    assert!(!temp_dir.path().join("doc_report.pdf").exists());
    assert!(!temp_dir.path().join("cron_worker.sh").exists());
}

#[test]
fn test_extract_zip_rejects_symlink() {
    let mut buffer = Vec::new();
    {
        let mut zip = ZipWriter::new(Cursor::new(&mut buffer));
        zip.add_symlink("link", "../outside", SimpleFileOptions::default())
            .unwrap();
        zip.finish().unwrap();
    }

    let parent = TempDir::new().unwrap();
    let dest = parent.path().join("dest");
    fs::create_dir_all(&dest).unwrap();

    let result = extract_zip(&buffer, &dest);
    assert!(result.is_err(), "symlink entries must be rejected");
    assert!(!dest.join("link").exists());
}

/// Stored ZIP with general-purpose flag bit 3 set, as streaming writers produce.
fn data_descriptor_zip(name: &str, data: &[u8]) -> Vec<u8> {
    let crc = crc32(data);
    let size = data.len() as u32;
    let mut buf = Vec::new();

    buf.extend_from_slice(&[0x50, 0x4b, 0x03, 0x04]);
    push_u16(&mut buf, 20);
    push_u16(&mut buf, 0x0008);
    for _ in 0..3 {
        push_u16(&mut buf, 0);
    }
    for _ in 0..3 {
        push_u32(&mut buf, 0);
    }
    push_u16(&mut buf, name.len() as u16);
    push_u16(&mut buf, 0);
    buf.extend_from_slice(name.as_bytes());
    buf.extend_from_slice(data);
    buf.extend_from_slice(&[0x50, 0x4b, 0x07, 0x08]);
    push_u32(&mut buf, crc);
    push_u32(&mut buf, size);
    push_u32(&mut buf, size);

    let cd_start = buf.len() as u32;
    buf.extend_from_slice(&[0x50, 0x4b, 0x01, 0x02]);
    push_u16(&mut buf, 20);
    push_u16(&mut buf, 20);
    push_u16(&mut buf, 0x0008);
    for _ in 0..3 {
        push_u16(&mut buf, 0);
    }
    push_u32(&mut buf, crc);
    push_u32(&mut buf, size);
    push_u32(&mut buf, size);
    push_u16(&mut buf, name.len() as u16);
    for _ in 0..4 {
        push_u16(&mut buf, 0);
    }
    push_u32(&mut buf, 0);
    push_u32(&mut buf, 0);
    buf.extend_from_slice(name.as_bytes());
    let cd_size = buf.len() as u32 - cd_start;

    buf.extend_from_slice(&[0x50, 0x4b, 0x05, 0x06]);
    push_u16(&mut buf, 0);
    push_u16(&mut buf, 0);
    push_u16(&mut buf, 1);
    push_u16(&mut buf, 1);
    push_u32(&mut buf, cd_size);
    push_u32(&mut buf, cd_start);
    push_u16(&mut buf, 0);
    buf
}

#[test]
fn test_extract_zip_accepts_data_descriptor_entries() {
    let dest = TempDir::new().unwrap();
    extract_zip(&data_descriptor_zip("pkg/manifest.json", b"{}"), dest.path()).unwrap();
    assert_eq!(
        fs::read(dest.path().join("pkg/manifest.json")).unwrap(),
        b"{}"
    );
}

#[test]
fn test_extract_zip_rejects_child_listed_before_its_parent_file() {
    let dest = TempDir::new().unwrap();
    let bytes = raw_stored_zip(&[("foo/bar", b"child"), ("foo", b"parent")]);
    let err = extract_zip(&bytes, dest.path()).unwrap_err().to_string();
    assert!(err.contains("conflicting entry names"), "{err}");
}

#[test]
fn test_extract_zip_rejects_directory_below_a_file() {
    let dest = TempDir::new().unwrap();
    let bytes = raw_stored_zip(&[("foo", b"parent"), ("foo/bar/", b"")]);
    let err = extract_zip(&bytes, dest.path()).unwrap_err().to_string();
    assert!(err.contains("conflicting entry names"), "{err}");
}

#[test]
fn test_extract_zip_accepts_root_directory_entry() {
    let dest = TempDir::new().unwrap();
    let bytes = raw_stored_zip(&[("./", b""), ("./manifest.json", b"{}")]);
    extract_zip(&bytes, dest.path()).unwrap();
    assert!(dest.path().join("manifest.json").is_file());
}

// zip strips `a:` on extract, so accepting this name would silently rename the file.
#[test]
fn test_extract_zip_rejects_drive_style_file_name() {
    let dest = TempDir::new().unwrap();
    let bytes = raw_stored_zip(&[("a:b.dcm", b"data")]);
    let err = extract_zip(&bytes, dest.path()).unwrap_err().to_string();
    assert!(err.contains("unsafe path"), "{err}");
}

#[cfg(unix)]
#[test]
fn test_extract_zip_ignores_modes_from_the_archive() {
    use std::os::unix::fs::PermissionsExt;

    let mut buffer = Vec::new();
    {
        let mut zip = ZipWriter::new(Cursor::new(&mut buffer));
        let locked = SimpleFileOptions::default().unix_permissions(0o000);
        zip.add_directory("junk/", locked).unwrap();
        zip.start_file("junk/big.bin", locked).unwrap();
        zip.write_all(b"payload").unwrap();
        zip.finish().unwrap();
    }

    let dest = TempDir::new().unwrap();
    let root = dest.path().to_path_buf();
    extract_zip(&buffer, &root).unwrap();

    let dir_mode = fs::metadata(root.join("junk")).unwrap().permissions().mode();
    assert_eq!(dir_mode & 0o777, 0o755);
    let file_mode = fs::metadata(root.join("junk/big.bin"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(file_mode & 0o777, 0o644);

    dest.close().unwrap();
    assert!(!root.exists());
}
