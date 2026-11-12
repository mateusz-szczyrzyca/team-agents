//! Reading a text file with a bound.

#[cfg(test)]
mod text_file {
    use std::os::unix::fs::PermissionsExt;

    use agent_runtime::text_file::{TextFile, TextFileError, read_text_file};

    #[tokio::test]
    async fn a_file_is_read_with_its_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "żółć\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();

        let file = read_text_file(&path, 100).await.unwrap();

        assert_eq!(
            file,
            Some(TextFile {
                text: "żółć\n".to_owned(),
                mode: 0o100_640
            })
        );
    }

    #[tokio::test]
    async fn a_missing_file_is_none() {
        let dir = tempfile::tempdir().unwrap();

        assert_eq!(read_text_file(&dir.path().join("none"), 100).await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_file_of_exactly_the_limit_is_read_and_one_byte_more_is_too_large() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "12345").unwrap();

        let exact = read_text_file(&path, 5).await.unwrap();
        let over = read_text_file(&path, 4).await.unwrap_err();

        assert_eq!(exact.map(|file| file.text), Some("12345".to_owned()));
        assert!(matches!(over, TextFileError::TooLarge { limit: 4 }), "{over:?}");
    }

    #[tokio::test]
    async fn a_file_that_is_not_text_and_a_directory_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.bin");
        std::fs::write(&path, [0xff, 0xfe]).unwrap();

        let binary = read_text_file(&path, 100).await.unwrap_err();
        let directory = read_text_file(dir.path(), 100).await.unwrap_err();

        assert!(matches!(binary, TextFileError::NotUtf8(_)), "{binary:?}");
        assert!(matches!(directory, TextFileError::Read(_)), "{directory:?}");
    }
}
