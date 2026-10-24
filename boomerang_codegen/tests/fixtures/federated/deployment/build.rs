fn main() {
    println!(
        "cargo:rustc-env=NATIVE_VEHICLE_BUILD_HOST={}",
        std::env::var("HOST").expect("Cargo supplies HOST")
    );
}
