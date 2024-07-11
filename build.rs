fn main() -> shadow_rs::SdResult<()> {
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/COMMIT_EDITMSG");
    shadow_rs::new()
}
