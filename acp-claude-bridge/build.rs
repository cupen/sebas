use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=../hooks/pretooluse.sh");
    let src = Path::new("../hooks/pretooluse.sh");
    if src.exists() {
        let dst = Path::new("hooks/pretooluse.sh");
        fs::create_dir_all("hooks").expect("create hooks dir");
        fs::copy(src, dst).expect("copy hook script");
        fs::set_permissions(dst, fs::Permissions::from_mode(0o755)).expect("chmod hook");
    }
}
