use std::{
    fs,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT_TEMP_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

#[test]
fn concrete_tinyvec_adapters_work_downstream_while_storage_operations_remain_private() {
    let lifecycle = cargo_check(
        "lifecycle",
        r#"
use boomerang_tinymap::tiny_vec::{InlineStorage, TinyVecBuilder};

fn main() {
    let mut builder = TinyVecBuilder::<u16, InlineStorage<u16, 2>>::inline();
    builder.try_extend_exact([10, 20].into_iter()).unwrap();
    let sealed = builder.seal();
    assert_eq!(sealed.as_ref().iter().copied().collect::<Vec<_>>(), [10, 20]);
}
"#,
    );
    assert!(
        lifecycle.status.success(),
        "concrete TinyVec adapters must remain usable downstream:\n{}",
        String::from_utf8_lossy(&lifecycle.stderr),
    );

    let storage = cargo_check(
        "storage-is-private",
        "use boomerang_tinymap::tiny_vec::Storage;\n\nfn main() {}\n",
    );
    assert!(
        !storage.status.success(),
        "Storage must not be importable by downstream safe code",
    );
    let stderr = String::from_utf8_lossy(&storage.stderr);
    assert!(
        stderr.contains("Storage") && stderr.contains("private"),
        "Storage import must fail because it is private, not for an unrelated reason:\n{stderr}",
    );
}

fn cargo_check(name: &str, source: &str) -> Output {
    let directory = std::env::temp_dir().join(format!(
        "boomerang-tinymap-downstream-{name}-{}-{}",
        std::process::id(),
        NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed),
    ));
    fs::create_dir_all(directory.join("src")).unwrap();
    fs::write(
        directory.join("Cargo.toml"),
        format!(
            "[package]\nname = \"tiny-vec-downstream-{name}\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[dependencies]\nboomerang_tinymap = {{ path = \"{}\", default-features = false }}\n",
            env!("CARGO_MANIFEST_DIR"),
        ),
    )
    .unwrap();
    fs::write(directory.join("src/main.rs"), source).unwrap();

    let output = Command::new(env!("CARGO"))
        .args(["check", "--quiet", "--offline"])
        .current_dir(&directory)
        .env("CARGO_TARGET_DIR", directory.join("target"))
        .output()
        .unwrap();
    fs::remove_dir_all(directory).unwrap();
    output
}
