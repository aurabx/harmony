#[cfg(test)]
mod tests {
    use super::super::*;
    use std::fs;
    use std::io::Write;
    use tempfile::TempDir;
    use zip::write::{FileOptions, ZipWriter};

    fn create_test_zip() -> Vec<u8> {
        let mut buffer = Vec::new();
        {
            let mut zip = ZipWriter::new(std::io::Cursor::new(&mut buffer));
            let options = FileOptions::default();

            // Add a regular file
            zip.start_file("test.txt", options).unwrap();
            zip.write_all(b"Hello, World!").unwrap();

            // Add a file in a subdirectory
            zip.start_file("subdir/nested.txt", options).unwrap();
            zip.write_all(b"Nested content").unwrap();

            // Add an empty directory
            zip.add_directory("empty_dir/", options).unwrap();

            zip.finish().unwrap();
        }
        buffer
    }

    #[test]
    fn test_extract_zip_basic() {
        let zip_data = create_test_zip();
        let temp_dir = TempDir::new().unwrap();

        let result = extract_zip(&zip_data, temp_dir.path());
        assert!(result.is_ok(), "Failed to extract zip: {:?}", result.err());

        // Verify extracted files
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

        // Verify nested file
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

        // Verify empty directory exists
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

        // Verify directory structure
        assert!(temp_dir.path().join("test.txt").exists());
        assert!(temp_dir.path().join("subdir").is_dir());
        assert!(temp_dir.path().join("subdir").join("nested.txt").exists());
        assert!(temp_dir.path().join("empty_dir").is_dir());
    }

    #[test]
    fn test_extract_zip_multiple_levels() {
        let mut buffer = Vec::new();
        {
            let mut zip = ZipWriter::new(std::io::Cursor::new(&mut buffer));
            let options = FileOptions::default();

            // Create deeply nested structure
            zip.start_file("level1/level2/level3/deep.txt", options)
                .unwrap();
            zip.write_all(b"Deep content").unwrap();

            zip.finish().unwrap();
        }

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

        // Create existing file
        let existing_file = temp_dir.path().join("test.txt");
        fs::write(&existing_file, b"Old content").unwrap();

        // Extract zip that contains the same file
        let zip_data = create_test_zip();
        let result = extract_zip(&zip_data, temp_dir.path());
        assert!(result.is_ok());

        // Verify file was overwritten
        let content = fs::read_to_string(&existing_file).unwrap();
        assert_eq!(content, "Hello, World!");
    }

    #[test]
    fn test_extract_zip_empty_archive() {
        let mut buffer = Vec::new();
        {
            let mut zip = ZipWriter::new(std::io::Cursor::new(&mut buffer));
            zip.finish().unwrap();
        }

        let temp_dir = TempDir::new().unwrap();
        let result = extract_zip(&buffer, temp_dir.path());
        assert!(result.is_ok(), "Empty zip should extract successfully");
    }
}
