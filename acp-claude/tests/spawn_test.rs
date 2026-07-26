use std::process::Stdio;
use tokio::process::Command;

#[tokio::test]
async fn echo_subprocess_round_trip() {
    let mut child = Command::new("cat")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();

    let stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();

    tokio::spawn(async move {
        use tokio::io::AsyncWriteExt;
        let mut s = stdin;
        s.write_all(b"hello\n").await.unwrap();
    });

    let mut buf = vec![0u8; 6];
    tokio::io::AsyncReadExt::read_exact(&mut stdout, &mut buf).await.unwrap();
    assert_eq!(&buf, b"hello\n");
}