fn main() {
    // `sqlx::migrate!` embeds the migrations at compile time; without this a
    // changed migration alone would keep the old one in the binary.
    println!("cargo:rerun-if-changed=migrations");
}
