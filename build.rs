fn main() -> shadow_rs::SdResult<()> {
    println!("cargo:rerun-if-changed=.git/HEAD");
    shadow_rs::new()
}
