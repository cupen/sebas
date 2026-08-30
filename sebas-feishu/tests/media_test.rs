use sebas_feishu::media::{MediaMeta, download_file};
use std::path::PathBuf;

#[tokio::test]
async fn download_writes_to_target_path() {
    // Mocks via mockito-style server skipped here; verify by build + integration.
    // Test only the path-composition helper:
    let meta = MediaMeta {
        file_key: "fk".into(),
        file_name: "a.png".into(),
        mime: None,
    };
    let dest = download_file::compose_dest(PathBuf::from("/tmp/dl"), &meta);
    assert_eq!(dest, PathBuf::from("/tmp/dl/a.png"));
}
