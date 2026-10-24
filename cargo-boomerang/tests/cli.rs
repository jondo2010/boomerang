use std::process::Command;

#[test]
fn legacy_pipeline_subcommands_are_unrecognized() {
    let mut diagnostics = Vec::new();
    for subcommand in ["build", "check", "run"] {
        let output = Command::new(env!("CARGO_BIN_EXE_cargo-boomerang"))
            .args(["boomerang", subcommand])
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !output.status.success(),
            "{subcommand} unexpectedly succeeded"
        );
        if !stderr.contains(&format!("unrecognized subcommand '{subcommand}'")) {
            diagnostics.push(format!("{subcommand}: {stderr}"));
        }
    }
    assert!(diagnostics.is_empty(), "{}", diagnostics.join("\n"));
}

#[test]
fn help_directs_applications_to_ordinary_cargo() {
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-boomerang"))
        .args(["boomerang", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ordinary Cargo commands"), "{stdout}");
    assert!(stdout.contains("monitor"), "{stdout}");
}
