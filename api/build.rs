// `sqlx::migrate!` embeds the migrations at compile time; without this a
// changed migration file does not rebuild the crate.
fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
