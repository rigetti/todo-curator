//! API integration tests for todo-curator
//!
//! These tests make actual API calls to GitHub and GitLab to verify the tool's
//! ability to check issue/MR status. They require authentication tokens:
//!
//! - GH_TOKEN: GitHub personal access token
//! - GITLAB_TOKEN: GitLab personal access token
//!
//! Run with:
//! ```bash
//! GH_TOKEN=$(gh auth token) GITLAB_TOKEN=$(glab config get token) cargo test --test api_integration_test
//! ```

use std::process::Command;

/// Test that the tool correctly identifies an open GitLab issue
/// Uses issue #1 in rigetti/experimental/kstrand/todo-curator which is kept permanently open
#[test]
#[ignore = "requires GITLAB_TOKEN"]
fn test_open_gitlab_issue() {
    let gitlab_token =
        std::env::var("GITLAB_TOKEN").expect("GITLAB_TOKEN must be set for this test");

    // Create a temporary test file with a TODO referencing the open issue
    let temp_dir = std::env::temp_dir();
    let test_file = temp_dir.join("test_open_gitlab_issue.rs");
    std::fs::write(
        &test_file,
        "// TODO rigetti/experimental/kstrand/todo-curator#1\n",
    )
    .expect("Failed to write test file");

    let output = Command::new(env!("CARGO_BIN_EXE_todo-curator"))
        .arg("check-closed")
        .arg("-p")
        .arg(&test_file)
        .env("GITLAB_TOKEN", gitlab_token)
        .env("GITLAB_URL", "gitlab.com")
        .output()
        .expect("Failed to execute todo-curator");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Clean up
    let _ = std::fs::remove_file(&test_file);

    // The issue is open, so it should NOT appear in closed issues
    assert!(
        !stderr.contains("TODO comments referencing closed issues/MRs:"),
        "Open issue should not be reported as closed. Got:\nSTDOUT:\n{}\nSTDERR:\n{}",
        stdout,
        stderr
    );

    // Should succeed with "All TODO references are valid"
    assert!(
        stdout.contains("All TODO references are valid."),
        "Expected success message for open issue. Got:\nSTDOUT:\n{}\nSTDERR:\n{}",
        stdout,
        stderr
    );

    assert!(
        output.status.success(),
        "Command should succeed for open issue"
    );
}

/// Test that the tool correctly identifies a closed GitHub issue
/// Uses rust-lang/rust#1 which is a well-known closed issue
#[test]
#[ignore = "requires GH_TOKEN"]
fn test_closed_github_issue() {
    let gh_token = std::env::var("GH_TOKEN").expect("GH_TOKEN must be set for this test");

    // Create a temporary test file with a TODO referencing a closed issue
    let temp_dir = std::env::temp_dir();
    let test_file = temp_dir.join("test_closed_github_issue.rs");
    std::fs::write(&test_file, "// TODO github.com/rust-lang/rust#1\n")
        .expect("Failed to write test file");

    let output = Command::new(env!("CARGO_BIN_EXE_todo-curator"))
        .arg("check-closed")
        .arg("-p")
        .arg(&test_file)
        .env("GH_TOKEN", gh_token)
        .output()
        .expect("Failed to execute todo-curator");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Clean up
    let _ = std::fs::remove_file(&test_file);

    // The issue is closed, so it should appear in the output
    assert!(
        stderr.contains("TODO comments referencing closed issues/MRs:"),
        "Closed issue should be reported. Got:\nSTDOUT:\n{}\nSTDERR:\n{}",
        stdout,
        stderr
    );

    assert!(
        stderr.contains("rust-lang/rust#1") || stderr.contains("github.com/rust-lang/rust#1"),
        "Should mention the specific closed issue. Got:\n{}",
        stderr
    );

    // Should exit with error code
    assert!(
        !output.status.success(),
        "Command should fail when closed issues are found"
    );
}

/// Test that the tool correctly identifies a closed GitLab issue
/// Uses rigetti/qcs/magneto#1 which is a known closed issue
#[test]
#[ignore = "requires GITLAB_TOKEN"]
fn test_closed_gitlab_issue() {
    let gitlab_token =
        std::env::var("GITLAB_TOKEN").expect("GITLAB_TOKEN must be set for this test");

    // Create a temporary test file with a TODO referencing a closed issue
    let temp_dir = std::env::temp_dir();
    let test_file = temp_dir.join("test_closed_gitlab_issue.rs");
    std::fs::write(&test_file, "// TODO rigetti/qcs/magneto#1\n")
        .expect("Failed to write test file");

    let output = Command::new(env!("CARGO_BIN_EXE_todo-curator"))
        .arg("check-closed")
        .arg("-p")
        .arg(&test_file)
        .env("GITLAB_TOKEN", gitlab_token)
        .env("GITLAB_URL", "gitlab.com")
        .output()
        .expect("Failed to execute todo-curator");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Clean up
    let _ = std::fs::remove_file(&test_file);

    // The issue is closed, so it should appear in the output
    assert!(
        stderr.contains("TODO comments referencing closed issues/MRs:"),
        "Closed issue should be reported. Got:\nSTDOUT:\n{}\nSTDERR:\n{}",
        stdout,
        stderr
    );

    assert!(
        stderr.contains("rigetti/qcs/magneto#1"),
        "Should mention the specific closed issue. Got:\n{}",
        stderr
    );

    // Should exit with error code
    assert!(
        !output.status.success(),
        "Command should fail when closed issues are found"
    );
}

/// Test that the tool correctly reports non-existent GitHub issues
#[test]
#[ignore = "requires GH_TOKEN"]
fn test_nonexistent_github_issue() {
    let gh_token = std::env::var("GH_TOKEN").expect("GH_TOKEN must be set for this test");

    // Create a temporary test file with a TODO referencing a non-existent issue
    let temp_dir = std::env::temp_dir();
    let test_file = temp_dir.join("test_nonexistent_github_issue.rs");
    std::fs::write(
        &test_file,
        "// TODO github.com/nonexistent-user-12345/nonexistent-repo-67890#99999\n",
    )
    .expect("Failed to write test file");

    let output = Command::new(env!("CARGO_BIN_EXE_todo-curator"))
        .arg("check-closed")
        .arg("-p")
        .arg(&test_file)
        .env("GH_TOKEN", gh_token)
        .output()
        .expect("Failed to execute todo-curator");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Clean up
    let _ = std::fs::remove_file(&test_file);

    // Should report the non-existent issue
    assert!(
        stderr.contains("TODO comments referencing non-existent or inaccessible issues/MRs:"),
        "Non-existent issue should be reported. Got:\nSTDOUT:\n{}\nSTDERR:\n{}",
        stdout,
        stderr
    );

    assert!(
        stderr.contains("nonexistent-user-12345/nonexistent-repo-67890#99999"),
        "Should mention the specific non-existent issue. Got:\n{}",
        stderr
    );

    // Should exit with error code
    assert!(
        !output.status.success(),
        "Command should fail when non-existent issues are found"
    );
}

/// Test that the tool correctly reports non-existent GitLab issues
#[test]
#[ignore = "requires GITLAB_TOKEN"]
fn test_nonexistent_gitlab_issue() {
    let gitlab_token =
        std::env::var("GITLAB_TOKEN").expect("GITLAB_TOKEN must be set for this test");

    // Create a temporary test file with a TODO referencing a non-existent issue
    let temp_dir = std::env::temp_dir();
    let test_file = temp_dir.join("test_nonexistent_gitlab_issue.rs");
    std::fs::write(
        &test_file,
        "// TODO rigetti/experimental/kstrand/todo-curator#999999\n",
    )
    .expect("Failed to write test file");

    let output = Command::new(env!("CARGO_BIN_EXE_todo-curator"))
        .arg("check-closed")
        .arg("-p")
        .arg(&test_file)
        .env("GITLAB_TOKEN", gitlab_token)
        .env("GITLAB_URL", "gitlab.com")
        .output()
        .expect("Failed to execute todo-curator");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Clean up
    let _ = std::fs::remove_file(&test_file);

    // Should report the non-existent issue
    assert!(
        stderr.contains("TODO comments referencing non-existent or inaccessible issues/MRs:"),
        "Non-existent GitLab issue should be reported. Got:\nSTDOUT:\n{}\nSTDERR:\n{}",
        stdout,
        stderr
    );

    assert!(
        stderr.contains("rigetti/experimental/kstrand/todo-curator#999999"),
        "Should mention the specific non-existent issue. Got:\n{}",
        stderr
    );

    // Should exit with error code
    assert!(
        !output.status.success(),
        "Command should fail when non-existent issues are found"
    );
}

/// Test mixed scenario: open, closed, and non-existent issues
#[test]
#[ignore = "requires GH_TOKEN and GITLAB_TOKEN"]
fn test_mixed_issue_states() {
    let gh_token = std::env::var("GH_TOKEN").expect("GH_TOKEN must be set for this test");
    let gitlab_token =
        std::env::var("GITLAB_TOKEN").expect("GITLAB_TOKEN must be set for this test");

    // Create a temporary test file with multiple TODO types
    let temp_dir = std::env::temp_dir();
    let test_file = temp_dir.join("test_mixed_issues.rs");
    std::fs::write(
        &test_file,
        "// TODO rigetti/experimental/kstrand/todo-curator#1 - open GitLab issue\n\
         // TODO rigetti/qcs/magneto#1 - closed GitLab issue\n\
         // TODO github.com/rust-lang/rust#1 - closed GitHub issue\n\
         // TODO github.com/nonexistent-user-xyz/nonexistent-repo-xyz#1 - non-existent\n",
    )
    .expect("Failed to write test file");

    let output = Command::new(env!("CARGO_BIN_EXE_todo-curator"))
        .arg("check-closed")
        .arg("-p")
        .arg(&test_file)
        .env("GH_TOKEN", gh_token)
        .env("GITLAB_TOKEN", gitlab_token)
        .env("GITLAB_URL", "gitlab.com")
        .output()
        .expect("Failed to execute todo-curator");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Clean up
    let _ = std::fs::remove_file(&test_file);

    // Should report the closed issue
    assert!(
        stderr.contains("TODO comments referencing closed issues/MRs:"),
        "Should report closed issues. Got:\nSTDOUT:\n{}\nSTDERR:\n{}",
        stdout,
        stderr
    );
    assert!(
        stderr.contains("rust-lang/rust#1"),
        "Should mention closed GitHub issue. Got:\n{}",
        stderr
    );

    // Should report the non-existent issue
    assert!(
        stderr.contains("TODO comments referencing non-existent or inaccessible issues/MRs:"),
        "Should report non-existent issues. Got:\n{}",
        stderr
    );
    assert!(
        stderr.contains("nonexistent-user-xyz/nonexistent-repo-xyz#1"),
        "Should mention non-existent issue. Got:\n{}",
        stderr
    );

    // Should NOT report the open GitLab issue as a problem
    // (it should be silently accepted as valid)

    // Should exit with error code due to closed and non-existent issues
    assert!(
        !output.status.success(),
        "Command should fail when problems are found"
    );
}
