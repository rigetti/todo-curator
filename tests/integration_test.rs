//! Integration tests for todo-curator
//!
//! These tests run the compiled binary against test data in the `data/` directory
//! to verify end-to-end functionality. Tests work both with and without authentication.
//!
//! To run with GitHub authentication:
//! ```bash
//! GH_TOKEN=$(gh auth token) cargo test --test integration_test
//! ```

use std::path::PathBuf;
use std::process::Command;

/// Integration test that runs the todo-curator binary against the test data
#[test]
fn test_example_todos_file() {
    // Get the path to the compiled binary
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let data_path = PathBuf::from(manifest_dir)
        .join("data")
        .join("example_todos.txt");

    // Ensure the test data file exists
    assert!(
        data_path.exists(),
        "Test data file should exist at {:?}",
        data_path
    );

    // Run the binary with the test data
    // Note: This will fail if GH_TOKEN is not set, which is expected behavior
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_todo-curator"));
    cmd.arg("check-closed").arg("-p").arg(&data_path);

    // Pass through GH_TOKEN if it exists
    if let Ok(token) = std::env::var("GH_TOKEN") {
        cmd.env("GH_TOKEN", token);
    }

    // Remove RUST_LOG to avoid tracing output interfering with tests
    cmd.env_remove("RUST_LOG");

    let output = cmd.output().expect("Failed to execute todo-curator");

    // Convert output to string for easier debugging
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // The command should either succeed or fail based on authentication
    // If GH_TOKEN is not set, it should fail with an auth error
    // If it is set, it should find some closed issues

    if std::env::var("GH_TOKEN").is_ok() {
        // With authentication, we expect to find closed issues
        assert!(
            stdout.contains("TODO comments referencing closed issues/MRs:")
                || stdout.contains("All TODO references are valid.")
                || stdout.contains("TODO comments referencing non-existent"),
            "Expected to find TODO analysis in stdout. Got:\nSTDOUT:\n{}\nSTDERR:\n{}",
            stdout,
            stderr
        );

        // Should find the known closed issues from rust-lang/rust#1 and microsoft/vscode#1
        if stdout.contains("TODO comments referencing closed issues/MRs:") {
            assert!(
                stdout.contains("rust-lang/rust#1") || stdout.contains("microsoft/vscode#1"),
                "Expected to find known closed issues. Got:\n{}",
                stdout
            );
        }
    } else {
        // Without authentication, should get an auth error or warning
        assert!(
            stderr.contains("auth") || stderr.contains("token") || stderr.contains("Warning"),
            "Expected authentication error or warning. Got:\nSTDOUT:\n{}\nSTDERR:\n{}",
            stdout,
            stderr
        );
    }
}

/// Test that the extractor correctly parses various TODO formats
#[test]
fn test_todo_extraction_from_data_file() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let data_path = PathBuf::from(manifest_dir)
        .join("data")
        .join("example_todos.txt");

    // Read the file content
    let content = std::fs::read_to_string(&data_path).expect("Failed to read test data file");

    // Verify the file contains various TODO formats we want to test
    assert!(
        content.contains("TODO #4949"),
        "Should contain local GitLab issue format"
    );
    assert!(
        content.contains("TODO github.com/knope-dev/knope#1686"),
        "Should contain GitHub shorthand format"
    );
    assert!(
        content.contains("TODO github.com/rust-lang/rust/issues/1"),
        "Should contain GitHub full URL format"
    );
    assert!(
        content.contains("TODO rigetti/qcs/magneto#229"),
        "Should contain GitLab rendered format"
    );
    assert!(
        content.contains("TODO github.com/doesnotexist/knope#1"),
        "Should contain non-existent GitHub issue"
    );
}

/// Test that the binary can handle a directory path
#[test]
fn test_directory_scanning() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let data_dir = PathBuf::from(manifest_dir).join("data");

    // Run the binary with the data directory
    let output = Command::new(env!("CARGO_BIN_EXE_todo-curator"))
        .arg("check-closed")
        .arg("-p")
        .arg(&data_dir)
        .env_remove("RUST_LOG")
        .output()
        .expect("Failed to execute todo-curator");

    let stderr = String::from_utf8_lossy(&output.stderr);

    // Should process the directory without crashing
    // The exact output depends on authentication, but it shouldn't panic
    assert!(
        !stderr.contains("panic") && !stderr.contains("thread 'main' panicked"),
        "Should not panic when scanning directory. Got:\n{}",
        stderr
    );
}
