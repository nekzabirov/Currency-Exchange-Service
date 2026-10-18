fn main() {
    // `sqlx::migrate!` embeds the files into the binary at COMPILE time. Cargo does not know about
    // them and would not rebuild the crate on an incremental build: the image would ship silently
    // without the new migration, and that would only be found on a failed migrate run.
    println!("cargo:rerun-if-changed=../../migrations");
}
