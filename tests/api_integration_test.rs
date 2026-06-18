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

use std::path::PathBuf;
use todo_curator::{
    check_closed_from_extraction, check_invalid_from_extraction, extract_todos,
    check_closed_references,
    checker::{ProjectDetection, StatusChecker},
    todo::TodoReference,
    CheckOutput,
};

async fn run_closed_reference_check(path: PathBuf) -> CheckOutput {
    run_closed_reference_check_with_excludes(path, &[]).await
}

async fn run_closed_reference_check_with_excludes(
    path: PathBuf,
    exclude_file_regexes: &[String],
) -> CheckOutput {
    let checker = StatusChecker::new()
        .await
        .expect("Failed to initialize status checker");
    let project_detection = ProjectDetection::None;

    check_closed_references(path, &project_detection, &checker, exclude_file_regexes)
        .await
        .expect("Failed to check closed references")
}

#[tokio::test]
async fn test_closed_check_ignores_lint_only_todos() {
    let temp_dir = std::env::temp_dir();
    let test_file = temp_dir.join("test_closed_check_ignores_lints.rs");
    std::fs::write(&test_file, "// TODO without a reference\n")
        .expect("Failed to write test file");

    let result = run_closed_reference_check(test_file.clone()).await;

    let _ = std::fs::remove_file(&test_file);

    assert!(result.closed.is_empty(), "No closed refs expected");
    assert!(result.not_found.is_empty(), "No not-found refs expected");
    assert!(result.lint_violations.is_empty(), "check-closed should ignore lint-only TODOs");
    assert_eq!(result.status, "success", "check-closed should succeed for lint-only TODOs");
}

#[tokio::test]
async fn test_check_all_projection_merges_closed_and_lint_errors() {
    let temp_dir = std::env::temp_dir();
    let test_file = temp_dir.join("test_check_all_projection.rs");
    let has_remote_auth = std::env::var("GH_TOKEN").is_ok() || std::env::var("GITLAB_TOKEN").is_ok();
    let file_content = if has_remote_auth {
        "// TODO github.com/nonexistent-user-12345/nonexistent-repo-67890#99999\n// TODO without a reference\n"
    } else {
        "// TODO without a reference\n"
    };
    std::fs::write(
        &test_file,
        file_content,
    )
    .expect("Failed to write test file");

    let extraction = extract_todos(&test_file, &[]).expect("Failed to extract TODOs");
    let checker = StatusChecker::new()
        .await
        .expect("Failed to initialize status checker");
    let project_detection = ProjectDetection::None;

    let mut closed = check_closed_from_extraction(&extraction, &project_detection, &checker)
        .await
        .expect("Failed to project closed check");
    let invalid = check_invalid_from_extraction(&extraction).expect("Failed to project invalid check");

    let _ = std::fs::remove_file(&test_file);

    for (category, mut violations) in invalid.lint_violations {
        closed
            .lint_violations
            .entry(category)
            .or_default()
            .append(&mut violations);
    }

    if closed.has_errors() {
        closed.status = "failure".to_string();
    }

    if has_remote_auth {
        assert!(!closed.not_found.is_empty(), "check-all should include stale/nonexistent refs");
    } else {
        assert!(closed.not_found.is_empty(), "No remote auth means no stale-ref lookup");
    }
    assert!(!closed.lint_violations.is_empty(), "check-all should include lint violations");
    assert_eq!(closed.status, "failure", "check-all should fail when either slice has errors");
}

#[tokio::test]
async fn test_excluded_file_regex_skips_closed_reference_checks() {
    let temp_root = std::env::temp_dir().join("todo_curator_closed_exclude_test");
    let docs_dir = temp_root.join("docs");
    let _ = std::fs::remove_dir_all(&temp_root);
    std::fs::create_dir_all(&docs_dir).expect("Failed to create temp docs dir");

    let test_file = docs_dir.join("todo-comments.md");
    std::fs::write(
        &test_file,
        "TODO github.com/nonexistent-user-12345/nonexistent-repo-67890#99999\n",
    )
    .expect("Failed to write test file");

    let excludes = vec!["todo-comments.md".to_string()];
    let result = run_closed_reference_check_with_excludes(temp_root.clone(), &excludes).await;

    let _ = std::fs::remove_dir_all(&temp_root);

    assert!(
        result.closed.is_empty(),
        "Excluded file should not contribute closed references. Got: {:?}",
        result.closed
    );
    assert!(
        result.not_found.is_empty(),
        "Excluded file should not trigger remote checks. Got: {:?}",
        result.not_found
    );
    assert_eq!(
        result.status, "success",
        "Excluded file content should not affect closed-check status"
    );
}

/// Test that `performance` TODOs are treated as valid and never sent to remote APIs.
#[tokio::test]
async fn test_performance_reference_skips_remote_checks() {
    let temp_dir = std::env::temp_dir();
    let test_file = temp_dir.join("test_performance_reference.rs");
    std::fs::write(&test_file, "// TODO performance - local-only marker\n")
        .expect("Failed to write test file");

    let result = run_closed_reference_check(test_file.clone()).await;

    // Clean up
    let _ = std::fs::remove_file(&test_file);

    assert!(
        result.closed.is_empty(),
        "`performance` should not be checked as a closed remote reference. Got: {:?}",
        result.closed
    );
    assert!(
        result.not_found.is_empty(),
        "`performance` should not be checked against remote APIs. Got: {:?}",
        result.not_found
    );
    assert!(
        result.lint_violations.is_empty(),
        "`performance` should not create lint violations. Got: {:?}",
        result.lint_violations
    );
    assert_eq!(
        result.status, "success",
        "`performance` TODO should produce success status"
    );
}

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
    let result = run_closed_reference_check(test_file.clone()).await;

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
    let result = run_closed_reference_check(test_file.clone()).await;

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
    let result = run_closed_reference_check(test_file.clone()).await;

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
    let result = run_closed_reference_check(test_file.clone()).await;

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
    let result = run_closed_reference_check(test_file.clone()).await;

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
    let result = run_closed_reference_check(test_file.clone()).await;

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

/// Test that the tool correctly identifies a closed GitLab epic
/// Uses epic #16 in rigetti/qcs/services which is closed
#[tokio::test]
#[cfg(feature = "test-integration-gitlab")]
async fn test_closed_gitlab_epic() {
    let _gitlab_token =
        std::env::var("GITLAB_TOKEN").expect("GITLAB_TOKEN must be set for this test");

    // Create a dedicated temporary directory for this test
    let test_dir = std::env::temp_dir().join("test_closed_gitlab_epic");
    std::fs::create_dir_all(&test_dir).expect("Failed to create test directory");

    let test_file = test_dir.join("test.rs");
    std::fs::write(&test_file, "// TODO rigetti/qcs/services&16\n")
        .expect("Failed to write test file");

    // Call check_closed_references with the directory path
    let result = run_closed_reference_check(test_dir.clone()).await;

    // Clean up
    let _ = std::fs::remove_dir_all(&test_dir);

    // Epic #16 is closed, so it should be reported
    assert!(
        !result.closed.is_empty(),
        "Closed epic should be reported. Got: {:?}",
        result
    );

    assert_eq!(
        result.closed.len(),
        1,
        "Should have exactly one closed epic"
    );

    let closed_epic = &result.closed[0];
    match &closed_epic.reference {
        TodoReference::GitLabEpic { group, number, .. } => {
            assert_eq!(
                group.as_deref(),
                Some("rigetti/qcs/services"),
                "Should be rigetti/qcs/services group"
            );
            assert_eq!(number, &16, "Should be epic #16");
        }
        _ => panic!("Expected GitLabEpic, got: {:?}", closed_epic.reference),
    }

    assert_eq!(
        result.status, "failure",
        "Status should be failure for closed epic"
    );
}

/// Test that the tool correctly identifies an open GitLab epic
/// Uses epic #1 in rigetti/experimental, which should be left open permanently
#[tokio::test]
#[cfg(feature = "test-integration-gitlab")]
async fn test_open_gitlab_epic() {
    let _gitlab_token =
        std::env::var("GITLAB_TOKEN").expect("GITLAB_TOKEN must be set for this test");

    // Create a dedicated temporary directory for this test
    let test_dir = std::env::temp_dir().join("test_open_gitlab_epic");
    std::fs::create_dir_all(&test_dir).expect("Failed to create test directory");

    let test_file = test_dir.join("test.rs");
    std::fs::write(&test_file, "// TODO rigetti/experimental&1\n")
        .expect("Failed to write test file");

    // Call check_closed_references with the directory path
    let result = run_closed_reference_check(test_dir.clone()).await;

    // Clean up
    let _ = std::fs::remove_dir_all(&test_dir);

    // Epic #25 is open, so it should NOT appear in closed epics
    assert!(
        result.closed.is_empty(),
        "Open epic should not be reported as closed. Got: {:?}",
        result.closed
    );

    assert!(
        result.not_found.is_empty(),
        "Open epic should not be reported as not found. Got: {:?}",
        result.not_found
    );

    assert_eq!(
        result.status, "success",
        "Status should be success for open epic"
    );
}

/// Test that the tool correctly handles epic references with full URLs
/// Uses epic #16 in rigetti/qcs/services which is closed
#[tokio::test]
#[cfg(feature = "test-integration-gitlab")]
async fn test_gitlab_epic_full_url() {
    let _gitlab_token =
        std::env::var("GITLAB_TOKEN").expect("GITLAB_TOKEN must be set for this test");

    // Create a dedicated temporary directory for this test
    let test_dir = std::env::temp_dir().join("test_gitlab_epic_full_url");
    std::fs::create_dir_all(&test_dir).expect("Failed to create test directory");

    let test_file = test_dir.join("test.rs");
    std::fs::write(
        &test_file,
        "// TODO https://gitlab.com/groups/rigetti/qcs/services/-/epics/16\n",
    )
    .expect("Failed to write test file");

    // Call check_closed_references with the directory path
    let result = run_closed_reference_check(test_dir.clone()).await;

    // Clean up
    let _ = std::fs::remove_dir_all(&test_dir);

    // Epic #16 is closed, so it should be reported
    assert!(
        !result.closed.is_empty(),
        "Closed epic should be reported. Got: {:?}",
        result
    );

    let closed_epic = &result.closed[0];
    match &closed_epic.reference {
        TodoReference::GitLabEpic { group, number, .. } => {
            assert_eq!(
                group.as_deref(),
                Some("rigetti/qcs/services"),
                "Should be rigetti/qcs/services group"
            );
            assert_eq!(number, &16, "Should be epic #16");
        }
        _ => panic!("Expected GitLabEpic, got: {:?}", closed_epic.reference),
    }

    assert_eq!(
        result.status, "failure",
        "Status should be failure for closed epic"
    );
}

/// Test that the tool correctly reports non-existent GitLab epics
#[tokio::test]
#[cfg(feature = "test-integration-gitlab")]
async fn test_nonexistent_gitlab_epic() {
    let _gitlab_token =
        std::env::var("GITLAB_TOKEN").expect("GITLAB_TOKEN must be set for this test");

    // Create a dedicated temporary directory for this test
    let test_dir = std::env::temp_dir().join("test_nonexistent_gitlab_epic");
    std::fs::create_dir_all(&test_dir).expect("Failed to create test directory");

    let test_file = test_dir.join("test.rs");
    std::fs::write(&test_file, "// TODO rigetti/qcs/services&999999\n")
        .expect("Failed to write test file");

    // Call check_closed_references with the directory path
    let result = run_closed_reference_check(test_dir.clone()).await;

    // Clean up
    let _ = std::fs::remove_dir_all(&test_dir);

    // Should report the non-existent epic
    assert!(
        !result.not_found.is_empty(),
        "Non-existent epic should be reported. Got: {:?}",
        result
    );

    // Verify the specific epic is rigetti/qcs/services&999999
    let not_found_epic = result.not_found.iter().find(|n| {
        matches!(
            &n.reference,
            TodoReference::GitLabEpic { group, number, .. }
            if group.as_deref() == Some("rigetti/qcs/services") && *number == 999999
        )
    });

    assert!(
        not_found_epic.is_some(),
        "Should find non-existent epic in not_found list. Got: {:?}",
        result.not_found
    );

    assert_eq!(
        result.status, "failure",
        "Status should be failure for non-existent epic"
    );
}
