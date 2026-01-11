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
//! GH_TOKEN=$(gh auth token) GITLAB_TOKEN=$(glab config get token) cargo test --test api_integration_test -- --ignored
//! ```

use serde::Deserialize;
use std::process::Command;

#[derive(Debug, Deserialize)]
struct TestOutput {
    closed: Vec<ClosedRef>,
    not_found: Vec<NotFoundRef>,
    status: String,
}

#[derive(Debug, Deserialize)]
struct ClosedRef {
    reference: TodoRef,
    title: String,
}

#[derive(Debug, Deserialize)]
struct NotFoundRef {
    reference: TodoRef,
    error: String,
}

#[derive(Debug, Deserialize)]
enum TodoRef {
    GitLabIssue {
        project: Option<String>,
        number: u32,
        source_line: String,
        file_path: String,
        line_number: u64,
    },
    GitHubIssue {
        repo: String,
        number: u32,
        source_line: String,
        file_path: String,
        line_number: u64,
    },
    GitLabMr {
        project: Option<String>,
        number: u32,
        source_line: String,
        file_path: String,
        line_number: u64,
    },
    GitHubPr {
        repo: String,
        number: u32,
        source_line: String,
        file_path: String,
        line_number: u64,
    },
}

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
        .arg("--format")
        .arg("json")
        .env("GITLAB_TOKEN", gitlab_token)
        .env("GITLAB_URL", "gitlab.com")
        .output()
        .expect("Failed to execute todo-curator");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Clean up
    let _ = std::fs::remove_file(&test_file);

    // Parse JSON output
    let result: TestOutput = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("Failed to parse JSON output: {}\nOutput: {}", e, stdout));

    // The issue is open, so it should NOT appear in closed issues
    assert!(
        result.closed.is_empty(),
        "Open issue should not be reported as closed. Got: {:?}",
        result.closed
    );

    assert!(
        result.not_found.is_empty(),
        "Open issue should not be reported as not found. Got: {:?}",
        result.not_found
    );

    assert_eq!(
        result.status, "success",
        "Status should be success for open issue"
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
        .arg("--format")
        .arg("json")
        .env("GH_TOKEN", gh_token)
        .output()
        .expect("Failed to execute todo-curator");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Clean up
    let _ = std::fs::remove_file(&test_file);

    // Parse JSON output
    let result: TestOutput = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("Failed to parse JSON output: {}\nOutput: {}", e, stdout));

    // The issue is closed, so it should appear in closed issues
    assert!(
        !result.closed.is_empty(),
        "Closed issue should be reported. Got: {:?}",
        result
    );

    assert_eq!(result.closed.len(), 1, "Should have exactly one closed issue");
    
    // Verify the specific issue is rust-lang/rust#1
    let closed_issue = &result.closed[0];
    match &closed_issue.reference {
        TodoRef::GitHubIssue { repo, number, .. } => {
            assert_eq!(repo, "rust-lang/rust", "Should be rust-lang/rust repo");
            assert_eq!(*number, 1, "Should be issue #1");
        }
        _ => panic!("Expected GitHubIssue, got: {:?}", closed_issue.reference),
    }
    
    assert_eq!(result.status, "failure", "Status should be failure for closed issue");

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
        .arg("--format")
        .arg("json")
        .env("GITLAB_TOKEN", gitlab_token)
        .env("GITLAB_URL", "gitlab.com")
        .output()
        .expect("Failed to execute todo-curator");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Clean up
    let _ = std::fs::remove_file(&test_file);

    // Parse JSON output
    let result: TestOutput = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("Failed to parse JSON output: {}\nOutput: {}", e, stdout));

    // The issue is closed, so it should appear in closed issues
    assert!(
        !result.closed.is_empty(),
        "Closed issue should be reported. Got: {:?}",
        result
    );

    assert_eq!(result.closed.len(), 1, "Should have exactly one closed issue");
    
    // Verify the specific issue is rigetti/qcs/magneto#1
    let closed_issue = &result.closed[0];
    match &closed_issue.reference {
        TodoRef::GitLabIssue { project, number, .. } => {
            assert_eq!(project.as_deref(), Some("rigetti/qcs/magneto"), "Should be rigetti/qcs/magneto project");
            assert_eq!(*number, 1, "Should be issue #1");
        }
        _ => panic!("Expected GitLabIssue, got: {:?}", closed_issue.reference),
    }
    
    assert_eq!(result.status, "failure", "Status should be failure for closed issue");

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
        .arg("--format")
        .arg("json")
        .env("GH_TOKEN", gh_token)
        .output()
        .expect("Failed to execute todo-curator");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Clean up
    let _ = std::fs::remove_file(&test_file);

    // Parse JSON output
    let result: TestOutput = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("Failed to parse JSON output: {}\nOutput: {}", e, stdout));

    // Should report the non-existent issue
    assert!(
        !result.not_found.is_empty(),
        "Non-existent issue should be reported. Got: {:?}",
        result
    );

    assert_eq!(result.not_found.len(), 1, "Should have exactly one not found issue");
    
    // Verify the specific issue is nonexistent-user-12345/nonexistent-repo-67890#99999
    let not_found_issue = &result.not_found[0];
    match &not_found_issue.reference {
        TodoRef::GitHubIssue { repo, number, .. } => {
            assert_eq!(repo, "nonexistent-user-12345/nonexistent-repo-67890", "Should be nonexistent-user-12345/nonexistent-repo-67890 repo");
            assert_eq!(*number, 99999, "Should be issue #99999");
        }
        _ => panic!("Expected GitHubIssue, got: {:?}", not_found_issue.reference),
    }
    
    assert_eq!(result.status, "failure", "Status should be failure for non-existent issue");

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
        .arg("--format")
        .arg("json")
        .env("GITLAB_TOKEN", gitlab_token)
        .env("GITLAB_URL", "gitlab.com")
        .output()
        .expect("Failed to execute todo-curator");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Clean up
    let _ = std::fs::remove_file(&test_file);

    // Parse JSON output
    let result: TestOutput = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("Failed to parse JSON output: {}\nOutput: {}", e, stdout));

    // Should report the non-existent issue
    assert!(
        !result.not_found.is_empty(),
        "Non-existent GitLab issue should be reported. Got: {:?}",
        result
    );

    assert_eq!(result.not_found.len(), 1, "Should have exactly one not found issue");
    
    // Verify the specific issue is rigetti/experimental/kstrand/todo-curator#999999
    let not_found_issue = &result.not_found[0];
    match &not_found_issue.reference {
        TodoRef::GitLabIssue { project, number, .. } => {
            assert_eq!(project.as_deref(), Some("rigetti/experimental/kstrand/todo-curator"), "Should be rigetti/experimental/kstrand/todo-curator project");
            assert_eq!(*number, 999999, "Should be issue #999999");
        }
        _ => panic!("Expected GitLabIssue, got: {:?}", not_found_issue.reference),
    }
    
    assert_eq!(result.status, "failure", "Status should be failure for non-existent issue");

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
        .arg("--format")
        .arg("json")
        .env("GH_TOKEN", gh_token)
        .env("GITLAB_TOKEN", gitlab_token)
        .env("GITLAB_URL", "gitlab.com")
        .output()
        .expect("Failed to execute todo-curator");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Clean up
    let _ = std::fs::remove_file(&test_file);

    // Parse JSON output
    let result: TestOutput = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("Failed to parse JSON output: {}\nOutput: {}", e, stdout));

    // Should report closed issues (both GitHub and GitLab)
    assert_eq!(
        result.closed.len(), 2,
        "Should report exactly 2 closed issues. Got: {:?}",
        result.closed
    );

    // Verify specific closed issues
    let mut found_github = false;
    let mut found_gitlab = false;
    for closed_ref in &result.closed {
        match &closed_ref.reference {
            TodoRef::GitHubIssue { repo, number, .. } => {
                if repo == "rust-lang/rust" && *number == 1 {
                    found_github = true;
                }
            }
            TodoRef::GitLabIssue { project, number, .. } => {
                if project.as_deref() == Some("rigetti/qcs/magneto") && *number == 1 {
                    found_gitlab = true;
                }
            }
            _ => {}
        }
    }
    assert!(found_github, "Should find closed GitHub issue rust-lang/rust#1");
    assert!(found_gitlab, "Should find closed GitLab issue rigetti/qcs/magneto#1");

    // Should report the non-existent issue
    assert_eq!(
        result.not_found.len(), 1,
        "Should report exactly 1 non-existent issue. Got: {:?}",
        result.not_found
    );
    
    // Verify specific non-existent issue
    let not_found_ref = &result.not_found[0];
    match &not_found_ref.reference {
        TodoRef::GitHubIssue { repo, number, .. } => {
            assert_eq!(repo, "nonexistent-user-xyz/nonexistent-repo-xyz", "Should be nonexistent-user-xyz/nonexistent-repo-xyz repo");
            assert_eq!(*number, 1, "Should be issue #1");
        }
        _ => panic!("Expected GitHubIssue for not_found, got: {:?}", not_found_ref.reference),
    }

    // Status should be failure due to closed and non-existent issues
    assert_eq!(result.status, "failure", "Status should be failure when problems are found");

    // Should exit with error code due to closed and non-existent issues
    assert!(
        !output.status.success(),
        "Command should fail when problems are found"
    );
}
