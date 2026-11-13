//! Replacing a text file: the new text in one piece, the mode asked for, and nothing left behind.

#[cfg(test)]
mod text_file_replace {
    use std::os::unix::fs::PermissionsExt;

    use agent_runtime::text_file::{ReplaceError, replace_text_file};

    #[tokio::test]
    async fn the_new_text_takes_the_place_of_the_old_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("questions-1.md");
        std::fs::write(&path, "old\n").unwrap();

        replace_text_file(&path, "żółć\nnew\n", 0o644).await.unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "żółć\nnew\n");
    }

    #[tokio::test]
    async fn a_file_that_was_not_there_is_made() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new.md");

        replace_text_file(&path, "text", 0o644).await.unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "text");
    }

    #[tokio::test]
    async fn the_file_has_the_mode_that_was_asked_for() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.md");
        std::fs::write(&path, "old").unwrap();

        replace_text_file(&path, "new", 0o640).await.unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o640);
    }

    #[tokio::test]
    async fn no_temporary_file_is_left_in_the_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.md");

        replace_text_file(&path, "one", 0o644).await.unwrap();
        replace_text_file(&path, "two", 0o644).await.unwrap();

        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, ["a.md"]);
    }

    #[tokio::test]
    async fn a_directory_that_is_not_there_is_a_write_error_that_names_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing").join("a.md");

        let error = replace_text_file(&path, "text", 0o644).await.unwrap_err();

        assert!(
            matches!(&error, ReplaceError::Write { path: named, .. } if *named == path),
            "{error:?}"
        );
    }
}
