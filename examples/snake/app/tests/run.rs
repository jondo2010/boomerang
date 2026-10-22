//! Execute the application through a Unix pseudo-terminal.

#[cfg(unix)]
#[test]
fn arrow_input_and_ctrl_c_restore_the_terminal() {
    let output = std::process::Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/support/pty_run.py"
        ))
        .arg(env!("CARGO_BIN_EXE_snake"))
        .output()
        .expect("Unix PTY coverage requires Python 3");
    assert!(
        output.status.success(),
        "PTY regression failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    print!("{}", String::from_utf8_lossy(&output.stdout));
}
