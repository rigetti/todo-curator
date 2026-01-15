//! API integration tests for todo-curator
//!
//! These tests make actual API calls to GitHub and GitLab to verify the tool's
//! ability to check issue/MR status. They require authentication tokens:
//!
//! - GH_TOKEN: GitHub personal access token
//! - GITLAB_TOKEN: GitLab personal access token
//!
//! Tests are gated by feature flags:
//! - `test-integration-github`: Enables GitHub API tests
//! - `test-integration-gitlab`: Enables GitLab API tests
//!
//! Run with:
//! ```bash
//! # GitHub tests only
//! GH_TOKEN=$(gh auth token) cargo test --test api_integration_test --features test-integration-github
//!
//! # GitLab tests only
//! GITLAB_TOKEN=$(glab config get token) cargo test --test api_integration_test --features test-integration-gitlab
//!
//! # All tests (combined)
//! GH_TOKEN=$(gh auth token) GITLAB_TOKEN=$(glab config get token) cargo test --test api_integration_test --features test-integration-github,test-integration-gitlab
//! ```

use todo_curator::{check_closed_references, todo::TodoReference};

/// Test that the tool correctly identifies an open GitLab issue
/// Uses issue #1 in rigetti/experimental/kstrand/todo-curator which is kept permanently open
#[tokio::test]
#[cfg(feature = "test-integration-gitlab")]
async fn test_open_gitlab_issue() {
    let _gitlab_token =
        std::env::var("GITLAB_TOKEN").expect("GITLAB_TOKEN must be set for this test");

    // Create a temporary test file with a TODO referencing the open issue
    let temp_dir = std::env::temp_dir();
    let test_file = temp_dir.join("test_open_gitlab_issue.rs");
    std::fs::write(
        &test_file,
        "// TODO rigetti/experimental/kstrand/todo-curator#1\n",
    )
    .expect("Failed to write test file");

    // Call check_closed_references directly
    let result = check_closed_references(test_file.clone())
        .await
        .expect("Failed to check closed references");

    // Clean up
    let _ = std::fs::remove_file(&test_file);

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
}

/// Test that the tool correctly identifies a closed GitHub issue
/// Uses rust-lang/rust#1 which is a well-known closed issue
#[tokio::test]
#[cfg(feature = "test-integration-github")]
async fn test_closed_github_issue() {
    let _gh_token = std::env::var("GH_TOKEN").expect("GH_TOKEN must be set for this test");

    // Create a temporary test file with a TODO referencing a closed issue
    let temp_dir = std::env::temp_dir();
    let test_file = temp_dir.join("test_closed_github_issue.rs");
    std::fs::write(&test_file, "// TODO github.com/rust-lang/rust#1\n")
        .expect("Failed to write test file");

    // Call check_closed_references directly
    let result = check_closed_references(test_file.clone())
        .await
        .expect("Failed to check closed references");

    // Clean up
    let _ = std::fs::remove_file(&test_file);

    // The issue is closed, so it should appear in closed issues
    assert!(
        !result.closed.is_empty(),
        "Closed issue should be reported. Got: {:?}",
        result
    );

    assert_eq!(
        result.closed.len(),
        1,
        "Should have exactly one closed issue"
    );

    // Verify the specific issue is rust-lang/rust#1
    let closed_issue = &result.closed[0];
    match &closed_issue.reference {
        TodoReference::GitHubIssue { repo, number, .. } => {
            assert_eq!(
                repo.as_deref(),
                Some("rust-lang/rust"),
                "Should be rust-lang/rust repo"
            );
            assert_eq!(number, &1, "Should be issue #1");
        }
        _ => panic!("Expected GitHubIssue, got: {:?}", closed_issue.reference),
    }

    assert_eq!(
        result.status, "failure",
        "Status should be failure for closed issue"
    );
}

/// Test that the tool correctly identifies a closed GitLab issue
/// Uses rigetti/qcs/magneto#617 which is a known closed issue
#[tokio::test]
#[cfg(feature = "test-integration-gitlab")]
async fn test_closed_gitlab_issue() {
    let _gitlab_token =
        std::env::var("GITLAB_TOKEN").expect("GITLAB_TOKEN must be set for this test");

    // Create a temporary test file with a TODO referencing a closed issue
    let temp_dir = std::env::temp_dir();
    let test_file = temp_dir.join("test_closed_gitlab_issue.rs");
    std::fs::write(&test_file, "// TODO rigetti/qcs/magneto#617\n")
        .expect("Failed to write test file");

    // Call check_closed_references directly
    let result = check_closed_references(test_file.clone())
        .await
        .expect("Failed to check closed references");

    // Clean up
    let _ = std::fs::remove_file(&test_file);

    // The issue is closed, so it should appear in closed issues
    assert!(
        !result.closed.is_empty(),
        "Closed issue should be reported. Got: {:?}",
        result
    );

    assert_eq!(
        result.closed.len(),
        1,
        "Should have exactly one closed issue"
    );

    // Verify the specific issue is rigetti/qcs/magneto#617
    let closed_issue = &result.closed[0];
    match &closed_issue.reference {
        TodoReference::GitLabIssue {
            project, number, ..
        } => {
            assert_eq!(
                project.as_deref(),
                Some("rigetti/qcs/magneto"),
                "Should be rigetti/qcs/magneto project"
            );
            assert_eq!(number, &617, "Should be issue #617");
        }
        _ => panic!("Expected GitLabIssue, got: {:?}", closed_issue.reference),
    }

    assert_eq!(
        result.status, "failure",
        "Status should be failure for closed issue"
    );
}

/// Test that the tool correctly reports non-existent GitHub issues
#[tokio::test]
#[cfg(feature = "test-integration-github")]
async fn test_nonexistent_github_issue() {
    let _gh_token = std::env::var("GH_TOKEN").expect("GH_TOKEN must be set for this test");

    // Create a temporary test file with a TODO referencing a non-existent issue
    let temp_dir = std::env::temp_dir();
    let test_file = temp_dir.join("test_nonexistent_github_issue.rs");
    std::fs::write(
        &test_file,
        "// TODO github.com/nonexistent-user-12345/nonexistent-repo-67890#99999\n",
    )
    .expect("Failed to write test file");

    // Call check_closed_references directly
    let result = check_closed_references(test_file.clone())
        .await
        .expect("Failed to check closed references");

    // Clean up
    let _ = std::fs::remove_file(&test_file);

    // Should report the non-existent issue
    assert!(
        !result.not_found.is_empty(),
        "Non-existent issue should be reported. Got: {:?}",
        result
    );

    assert_eq!(
        result.not_found.len(),
        1,
        "Should have exactly one not found issue"
    );

    // Verify the specific issue is nonexistent-user-12345/nonexistent-repo-67890#99999
    let not_found_issue = &result.not_found[0];
    match &not_found_issue.reference {
        TodoReference::GitHubIssue { repo, number, .. } => {
            assert_eq!(
                repo.as_deref(),
                Some("nonexistent-user-12345/nonexistent-repo-67890"),
                "Should be nonexistent-user-12345/nonexistent-repo-67890 repo"
            );
            assert_eq!(number, &99999, "Should be issue #99999");
        }
        _ => panic!("Expected GitHubIssue, got: {:?}", not_found_issue.reference),
    }

    assert_eq!(
        result.status, "failure",
        "Status should be failure for non-existent issue"
    );
}

/// Test that the tool correctly reports non-existent GitLab issues
#[tokio::test]
#[cfg(feature = "test-integration-gitlab")]
async fn test_nonexistent_gitlab_issue() {
    let _gitlab_token =
        std::env::var("GITLAB_TOKEN").expect("GITLAB_TOKEN must be set for this test");

    // Create a temporary test file with a TODO referencing a non-existent issue
    let temp_dir = std::env::temp_dir();
    let test_file = temp_dir.join("test_nonexistent_gitlab_issue.rs");
    std::fs::write(
        &test_file,
        "// TODO rigetti/experimental/kstrand/todo-curator#999999\n",
    )
    .expect("Failed to write test file");

    // Call check_closed_references directly
    let result = check_closed_references(test_file.clone())
        .await
        .expect("Failed to check closed references");

    // Clean up
    let _ = std::fs::remove_file(&test_file);

    // Should report the non-existent issue
    assert!(
        !result.not_found.is_empty(),
        "Non-existent GitLab issue should be reported. Got: {:?}",
        result
    );

    assert_eq!(
        result.not_found.len(),
        1,
        "Should have exactly one not found issue"
    );

    // Verify the specific issue is rigetti/experimental/kstrand/todo-curator#999999
    let not_found_issue = &result.not_found[0];
    match &not_found_issue.reference {
        TodoReference::GitLabIssue {
            project, number, ..
        } => {
            assert_eq!(
                project.as_deref(),
                Some("rigetti/experimental/kstrand/todo-curator"),
                "Should be rigetti/experimental/kstrand/todo-curator project"
            );
            assert_eq!(number, &999999, "Should be issue #999999");
        }
        _ => panic!("Expected GitLabIssue, got: {:?}", not_found_issue.reference),
    }

    assert_eq!(
        result.status, "failure",
        "Status should be failure for non-existent issue"
    );
}

/// Test mixed scenario: open, closed, and non-existent issues
#[tokio::test]
#[cfg(all(
    feature = "test-integration-github",
    feature = "test-integration-gitlab"
))]
async fn test_mixed_issue_states() {
    let _gh_token = std::env::var("GH_TOKEN").expect("GH_TOKEN must be set for this test");
    let _gitlab_token =
        std::env::var("GITLAB_TOKEN").expect("GITLAB_TOKEN must be set for this test");

    // Create a temporary test file with multiple TODO types
    let temp_dir = std::env::temp_dir();
    let test_file = temp_dir.join("test_mixed_issues.rs");
    std::fs::write(
        &test_file,
        "// TODO rigetti/experimental/kstrand/todo-curator#1 - open GitLab issue\n\
         // TODO rigetti/qcs/magneto#617 - closed GitLab issue\n\
         // TODO github.com/rust-lang/rust#1 - closed GitHub issue\n\
         // TODO github.com/nonexistent-user-xyz/nonexistent-repo-xyz#1 - non-existent\n",
    )
    .expect("Failed to write test file");

    // Call check_closed_references directly
    let result = check_closed_references(test_file.clone())
        .await
        .expect("Failed to check closed references");

    // Clean up
    let _ = std::fs::remove_file(&test_file);

    // Should report closed issues (both GitHub and GitLab)
    assert_eq!(
        result.closed.len(),
        2,
        "Should report exactly 2 closed issues. Got: {:?}",
        result.closed
    );

    // Verify specific closed issues
    let mut found_github = false;
    let mut found_gitlab = false;
    for closed_ref in &result.closed {
        match &closed_ref.reference {
            TodoReference::GitHubIssue { repo, number, .. } => {
                if repo.as_deref() == Some("rust-lang/rust") && number == &1 {
                    found_github = true;
                }
            }
            TodoReference::GitLabIssue {
                project, number, ..
            } => {
                if project.as_deref() == Some("rigetti/qcs/magneto") && number == &617 {
                    found_gitlab = true;
                }
            }
            _ => {}
        }
    }
    assert!(
        found_github,
        "Should find closed GitHub issue rust-lang/rust#1"
    );
    assert!(
        found_gitlab,
        "Should find closed GitLab issue rigetti/qcs/magneto#617"
    );

    // Should report the non-existent issue
    assert_eq!(
        result.not_found.len(),
        1,
        "Should report exactly 1 non-existent issue. Got: {:?}",
        result.not_found
    );

    // Verify specific non-existent issue
    let not_found_ref = &result.not_found[0];
    match &not_found_ref.reference {
        TodoReference::GitHubIssue { repo, number, .. } => {
            assert_eq!(
                repo.as_deref(),
                Some("nonexistent-user-xyz/nonexistent-repo-xyz"),
                "Should be nonexistent-user-xyz/nonexistent-repo-xyz repo"
            );
            assert_eq!(number, &1, "Should be issue #1");
        }
        _ => panic!(
            "Expected GitHubIssue for not_found, got: {:?}",
            not_found_ref.reference
        ),
    }

    // Status should be failure due to closed and non-existent issues
    assert_eq!(
        result.status, "failure",
        "Status should be failure when problems are found"
    );
}
