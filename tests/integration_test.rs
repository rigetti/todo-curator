//! Integration tests for todo-curator
//!
//! These tests run the compiled binary against test data in the `data/` directory
//! to verify end-to-end functionality. Tests work both with and without authentication.
//!
//! To run with GitHub authentication:
//! ```bash
//! # Prefer using the helper to populate auth env vars
//! source scripts/auth-setup.sh
//! cargo test --test integration_test
//! ```

use std::path::PathBuf;
use std::process::Command;
use todo_curator::todo::TodoReferenceKind;
use todo_curator::{
    check_closed_from_extraction, check_closed_references, check_invalid_from_extraction,
    checker::{ProjectDetection, StatusChecker},
    extract_todos, CheckOutput,
};

async fn run_closed_reference_check(path: PathBuf) -> CheckOutput {
    run_closed_reference_check_with_excludes(path, "").await
}

async fn run_closed_reference_check_with_excludes(
    path: PathBuf,
    exclude_file_regex: &str,
) -> CheckOutput {
    let checker = StatusChecker::new()
        .await
        .expect("Failed to initialize status checker");
    let project_detection = ProjectDetection::None;

    check_closed_references(path, &project_detection, &checker, exclude_file_regex)
        .await
        .expect("Failed to check closed references")
}

/// Integration test that runs the todo-curator binary against the test data
#[test_log::test]
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
    // Note: This will fail if GITHUB_TOKEN is not set, which is expected behavior
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_todo-curator"));
    cmd.arg("check-closed").arg("-p").arg(&data_path);

    // Pass through GITHUB_TOKEN if it exists
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        cmd.env("GITHUB_TOKEN", token);
    }

    // Remove RUST_LOG to avoid tracing output interfering with tests
    cmd.env_remove("RUST_LOG");

    let output = cmd.output().expect("Failed to execute todo-curator");

    // Convert output to string for easier debugging
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // The command should either succeed or fail based on authentication
    // If GITHUB_TOKEN is not set, it should fail with an auth error
    // If it is set, it should find some closed issues

    if std::env::var("GITHUB_TOKEN").is_ok() {
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
#[test_log::test]
fn test_todo_extraction_from_data_file() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let data_path = PathBuf::from(manifest_dir)
        .join("data")
        .join("example_todos.txt");

    // Read the file content
    let content = std::fs::read_to_string(&data_path).expect("Failed to read test data file");

    // Verify the file contains various TODO formats we want to test
    assert!(
        content.contains("TODO #4949:"),
        "Should contain local GitLab issue format"
    );
    assert!(
        content.contains("TODO github.com/knope-dev/knope#1686:"),
        "Should contain GitHub shorthand format"
    );
    assert!(
        content.contains("TODO github.com/rust-lang/rust/issues/1"),
        "Should contain GitHub full URL format"
    );
    assert!(
        content.contains("TODO rigetti/qcs/magneto#229:"),
        "Should contain GitLab rendered format"
    );
    assert!(
        content.contains("TODO github.com/doesnotexist/knope#1:"),
        "Should contain non-existent GitHub issue"
    );
}

/// Test that the extractor correctly parses GitLab epic formats
#[test_log::test]
fn test_epic_extraction() {
    use std::fs;
    use todo_curator::todo::{TodoExtractor, TodoReference};

    let temp_dir = std::env::temp_dir();
    let test_file = temp_dir.join("test_epic_extraction.rs");

    // Create a test file with various epic formats
    let content = r#"
// TODO &17: local epic reference
// TODO rigetti/qcs/services&42: epic with group path
// TODO https://gitlab.com/groups/rigetti/qcs/services/-/epics/123: full URL
// TODO gitlab.com/groups/rigetti/qcs/services/-/epics/456: URL without schema
"#;

    fs::write(&test_file, content).expect("Failed to write test file");

    let extractor = TodoExtractor::new();
    let extraction = extractor
        .extract_from_directory(&temp_dir)
        .expect("Failed to extract references");
    let references = extraction.references;

    // Clean up
    let _ = fs::remove_file(&test_file);

    // Filter to only epic references from our test file
    let epic_refs: Vec<_> = references
        .iter()
        .filter(|r| {
            matches!(
                r,
                TodoReference {
                    kind: TodoReferenceKind::GitLabEpic { .. },
                    ..
                }
            )
        })
        .collect();

    // Should find all 4 epic references
    assert!(
        epic_refs.len() >= 4,
        "Should extract at least 4 epic references. Found: {}",
        epic_refs.len()
    );

    // Verify local epic reference
    let has_local = epic_refs.iter().any(|r| {
        matches!(
            r,
            TodoReference {
                kind: TodoReferenceKind::GitLabEpic {
                    group: None,
                    number: 17,
                    ..
                },
                ..
            }
        )
    });
    assert!(has_local, "Should find local epic &17");

    // Verify epic with group path
    let has_group_path = epic_refs.iter().any(|r| {
        matches!(
            r,
            TodoReference {
                kind: TodoReferenceKind::GitLabEpic {
                    group: Some(g),
                    number: 42,
                    ..
                },
                ..
            }
            if g == "rigetti/qcs/services"
        )
    });
    assert!(has_group_path, "Should find epic rigetti/qcs/services&42");

    // Verify full URL epic
    let has_full_url = epic_refs.iter().any(|r| {
        matches!(
            r,
            TodoReference {
                kind: TodoReferenceKind::GitLabEpic {
                    group: Some(g),
                    number: 123,
                    ..
                },
                ..
            }
            if g == "rigetti/qcs/services"
        )
    });
    assert!(has_full_url, "Should find epic from full URL");

    // Verify URL without schema
    let has_no_schema = epic_refs.iter().any(|r| {
        matches!(
            r,
            TodoReference {
                kind: TodoReferenceKind::GitLabEpic {
                    group: Some(g),
                    number: 456,
                    ..
                },
                ..
            }
            if g == "rigetti/qcs/services"
        )
    });
    assert!(has_no_schema, "Should find epic from URL without schema");
}

/// Test that the binary can handle a directory path
#[test_log::test]
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

/// Test that check-invalid does not require GitHub/GitLab auth.
#[test_log::test]
fn test_check_invalid_skips_auth_validation() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let temp_dir = std::env::temp_dir().join("todo_curator_check_invalid_no_auth");
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    let test_file = temp_dir.join("bad.rs");
    std::fs::write(&test_file, "// TODO without a reference\n").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_todo-curator"))
        .arg("check-invalid")
        .arg("-p")
        .arg(&temp_dir)
        .env_remove("GITHUB_TOKEN")
        .env_remove("GITLAB_TOKEN")
        .env("GL_TOKEN", "invalid_token")
        .env_remove("RUST_LOG")
        .current_dir(manifest_dir)
        .output()
        .expect("Failed to execute todo-curator");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    let _ = std::fs::remove_dir_all(&temp_dir);

    assert!(
        !stderr.contains("auth") && !stderr.contains("token"),
        "check-invalid should not prompt for auth. Got:\nSTDOUT:\n{}\nSTDERR:\n{}",
        stdout,
        stderr
    );
    assert!(
        stdout.contains("Invalid TODO-like comments:")
            || stdout.contains("TODO comments referencing"),
        "Expected invalid TODO output. Got:\nSTDOUT:\n{}\nSTDERR:\n{}",
        stdout,
        stderr
    );
}

/// Test that an empty GitLab token is treated as no token at all, rather than
/// as a token that fails to build a client. CI systems routinely pass unset
/// secrets through as empty strings.
#[test_log::test]
fn test_empty_gitlab_token_is_not_configured_auth() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let temp_dir = std::env::temp_dir().join("todo_curator_empty_gitlab_token");
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    // A GitHub reference, in a directory with no GitLab references at all.
    let test_file = temp_dir.join("todo.rs");
    std::fs::write(&test_file, "// TODO github.com/rust-lang/rust#1:\n").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_todo-curator"))
        .arg("check-closed")
        .arg("-p")
        .arg(&temp_dir)
        .env("GITHUB_TOKEN", "invalid_token")
        .env("GITLAB_TOKEN", "")
        .env("GL_TOKEN", "")
        .env_remove("RUST_LOG")
        .current_dir(manifest_dir)
        .output()
        .expect("Failed to execute todo-curator");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    let _ = std::fs::remove_dir_all(&temp_dir);

    assert!(
        !stderr.contains("Failed to initialize GitLab client"),
        "An empty GITLAB_TOKEN should be ignored, not used to build a client. Got:\nSTDOUT:\n{}\nSTDERR:\n{}",
        stdout,
        stderr
    );
}

/// Test that FIXME markers are reported as improper TODO-like violations.
#[test_log::test]
fn test_extractor_reports_fixme_as_invalid_todo_like() {
    use std::fs;
    use todo_curator::todo::TodoExtractor;

    let temp_dir = std::env::temp_dir().join("todo_curator_extractor_fixme");
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).unwrap();

    fs::write(
        temp_dir.join("fixme.rs"),
        "// FIXME: remove this workaround\n",
    )
    .unwrap();

    let extractor = TodoExtractor::new();
    let extraction = extractor.extract_from_directory(&temp_dir).unwrap();

    let _ = fs::remove_dir_all(&temp_dir);

    assert_eq!(
        extraction.lint_violations.len(),
        1,
        "Expected one invalid TODO-like violation from FIXME. Found: {:#?}",
        extraction.lint_violations
    );
}

#[test_log::test]
fn test_invalid_projection_includes_simplification_warnings() {
    let temp_dir = std::env::temp_dir().join("todo_curator_invalid_projection_warnings");
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).expect("Failed to create temp dir");
    let test_file = temp_dir.join("sample.rs");
    std::fs::write(
        &test_file,
        "// TODO https://github.com/foo/bar/issues/123:\n",
    )
    .expect("Failed to write test file");

    let extraction = extract_todos(&temp_dir, "").expect("Failed to extract TODOs");
    let _ = std::fs::remove_dir_all(&temp_dir);

    let output = check_invalid_from_extraction(&extraction, &ProjectDetection::None)
        .expect("Failed to project invalid check");

    assert!(
        output.lint_violations.is_empty(),
        "Should have no lint violations"
    );
    assert!(
        !output.warnings.is_empty(),
        "check-invalid projection should include simplification warnings"
    );
}

#[tokio::test]
#[test_log::test]
async fn test_closed_check_ignores_lint_only_todos() {
    let temp_dir = std::env::temp_dir();
    let test_file = temp_dir.join("test_closed_check_ignores_lints.rs");
    std::fs::write(&test_file, "// TODO without a reference\n").expect("Failed to write test file");

    let result = run_closed_reference_check(test_file.clone()).await;

    let _ = std::fs::remove_file(&test_file);

    assert!(result.closed.is_empty(), "No closed refs expected");
    assert!(result.not_found.is_empty(), "No not-found refs expected");
    assert!(
        result.lint_violations.is_empty(),
        "check-closed should ignore lint-only TODOs"
    );
    assert_eq!(
        result.status, "success",
        "check-closed should succeed for lint-only TODOs"
    );
}

#[tokio::test]
#[test_log::test]
async fn test_check_all_projection_merges_closed_and_lint_errors() {
    let temp_dir = std::env::temp_dir();
    let test_file = temp_dir.join("test_check_all_projection.rs");
    let has_remote_auth =
        std::env::var("GITHUB_TOKEN").is_ok() || std::env::var("GITLAB_TOKEN").is_ok();
    let file_content = if has_remote_auth {
        "// TODO github.com/nonexistent-user-12345/nonexistent-repo-67890#99999:\n// TODO without a reference\n"
    } else {
        "// TODO without a reference\n"
    };
    std::fs::write(&test_file, file_content).expect("Failed to write test file");

    let extraction = extract_todos(&test_file, "").expect("Failed to extract TODOs");
    let checker = match StatusChecker::new().await {
        Ok(checker) => checker,
        Err(e) => {
            eprintln!("Skipping due to auth/client init issue: {e}");
            let _ = std::fs::remove_file(&test_file);
            return;
        }
    };
    let project_detection = ProjectDetection::None;

    let mut closed = check_closed_from_extraction(&extraction, &project_detection, &checker)
        .await
        .expect("Failed to project closed check");
    let invalid = check_invalid_from_extraction(&extraction, &project_detection)
        .expect("Failed to project invalid check");

    let _ = std::fs::remove_file(&test_file);

    closed.lint_violations.extend(invalid.lint_violations);

    if closed.has_errors() {
        closed.status = "failure".to_string();
    }

    if has_remote_auth {
        assert!(
            !closed.not_found.is_empty(),
            "check-all should include stale/nonexistent refs"
        );
    } else {
        assert!(
            closed.not_found.is_empty(),
            "No remote auth means no stale-ref lookup"
        );
    }
    assert!(
        !closed.lint_violations.is_empty(),
        "check-all should include lint violations"
    );
    assert_eq!(
        closed.status, "failure",
        "check-all should fail when either slice has errors"
    );
}

#[tokio::test]
#[test_log::test]
async fn test_excluded_file_regex_skips_closed_reference_checks() {
    let temp_root = std::env::temp_dir().join("todo_curator_closed_exclude_test");
    let docs_dir = temp_root.join("docs");
    let _ = std::fs::remove_dir_all(&temp_root);
    std::fs::create_dir_all(&docs_dir).expect("Failed to create temp docs dir");

    let test_file = docs_dir.join("todo-comments.md");
    std::fs::write(
        &test_file,
        "TODO github.com/nonexistent-user-12345/nonexistent-repo-67890#99999:\n",
    )
    .expect("Failed to write test file");

    let result =
        run_closed_reference_check_with_excludes(temp_root.clone(), "todo-comments.md").await;

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
#[test_log::test]
async fn test_performance_reference_skips_remote_checks() {
    let temp_dir = std::env::temp_dir();
    let test_file = temp_dir.join("test_performance_reference.rs");
    std::fs::write(&test_file, "// TODO performance: local-only marker\n")
        .expect("Failed to write test file");

    let result = run_closed_reference_check(test_file.clone()).await;

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

/// Test that valid TODO patterns produce no syntax errors during extraction
#[test_log::test]
fn test_lint_valid_patterns() {
    use std::fs;
    use todo_curator::todo::TodoExtractor;

    let temp_dir = std::env::temp_dir().join("todo_curator_lint_test_valid");
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).unwrap();

    let content = r#"
// TODO #123:
// TODO rigetti/qcs/magneto#229:
// TODO github.com/owner/repo#456:
// TODO !789:
// TODO rigetti/qcs/magneto!100:
// TODO &17:
// TODO rigetti/qcs/services&42:
// TODO performance: allowed exception
// TODO(#123) parenthesized local issue
// TODO(rigetti/qcs/magneto#229) parenthesized GitLab issue
// TODO(github.com/owner/repo#456) parenthesized GitHub issue
// TODO(!789) parenthesized MR
// TODO(&17) parenthesized epic
// TODO (#321) parenthesized local issue with spacing
// TODO (rigetti/qcs/magneto#654) parenthesized GitLab issue with spacing
// TODO (github.com/owner/repo#987) parenthesized GitHub issue with spacing
// TODO (!222) parenthesized MR with spacing
// TODO (&333) parenthesized epic with spacing
// TODO(#7, #8, github.com/foo/bar#1) parenthesized comma-delimited refs
// TODO( #9 , rigetti/qcs/magneto#10 , github.com/owner/repo#11 ) parenthesized comma-delimited refs with spacing
// using todo-curator in a comment is fine
"#;

    fs::write(temp_dir.join("good.rs"), content).unwrap();

    let extractor = TodoExtractor::new();
    let extraction = extractor.extract_from_directory(&temp_dir).unwrap();

    // Clean up
    let _ = fs::remove_dir_all(&temp_dir);

    // All lines should pass - no syntax violations expected
    assert!(
        extraction.lint_violations.is_empty(),
        "Valid TODO patterns should not produce extraction violations. Got: {:#?}",
        extraction.lint_violations
    );
}

/// Test that exclude file regexes skip matching files during extraction
#[test_log::test]
fn test_lint_exclude_file_regexes() {
    use std::fs;
    use todo_curator::todo::TodoExtractor;

    let temp_dir = std::env::temp_dir().join("todo_curator_lint_test_exclude");
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).unwrap();

    fs::create_dir_all(temp_dir.join("skip_dir")).unwrap();
    fs::write(temp_dir.join("skip_me.rs"), "// TODO without a reference\n").unwrap();
    fs::write(
        temp_dir.join("include_me.rs"),
        "// TODO without a reference\n",
    )
    .unwrap();

    let extractor = TodoExtractor::with_exclude_file_regex("skip_me\\.rs$").unwrap();
    let extraction = extractor.extract_from_directory(&temp_dir).unwrap();

    // Clean up
    let _ = fs::remove_dir_all(&temp_dir);

    assert_eq!(
        extraction.lint_violations.len(),
        1,
        "Expected one extraction violation from include_me.rs only. Found: {:#?}",
        extraction.lint_violations
    );
    assert!(
        extraction.lint_violations[0]
            .file_path
            .contains("include_me.rs"),
        "Expected violation to come from include_me.rs. Got: {:#?}",
        extraction.lint_violations
    );
}

/// Test that duplicate walk entries (e.g. .gitlab-ci.yml) do not duplicate violations.
#[test_log::test]
fn test_lint_violations_are_deduplicated() {
    use std::fs;
    use todo_curator::todo::TodoExtractor;

    let temp_dir = std::env::temp_dir().join("todo_curator_lint_dedup");
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).unwrap();

    // walk_source_files explicitly adds this file; it may also be discovered by normal walk.
    fs::write(temp_dir.join(".gitlab-ci.yml"), "# FIXME\n").unwrap();

    let extractor = TodoExtractor::new();
    let extraction = extractor.extract_from_directory(&temp_dir).unwrap();

    let _ = fs::remove_dir_all(&temp_dir);

    assert_eq!(
        extraction.lint_violations.len(),
        1,
        "Expected deduplicated lint violations. Found: {:#?}",
        extraction.lint_violations
    );
}

/// Test that lowercase `todo`/`temp` require comment markers to be linted.
#[test_log::test]
fn test_lowercase_todo_temp_require_comment_indicator() {
    use std::fs;
    use todo_curator::todo::TodoExtractor;

    let temp_dir = std::env::temp_dir().join("todo_curator_lowercase_comment_gated");
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).unwrap();

    let content = r#"
temp
todo
todo-item
# temp
// todo
# todo-curator
"#;

    fs::write(temp_dir.join("note.txt"), content).unwrap();

    let extractor = TodoExtractor::new();
    let extraction = extractor.extract_from_directory(&temp_dir).unwrap();

    let _ = fs::remove_dir_all(&temp_dir);

    assert_eq!(
        extraction.lint_violations.len(),
        2,
        "Expected only comment-marked lowercase todo/temp (excluding todo-) to lint. Found: {:#?}",
        extraction.lint_violations
    );

    assert!(
        extraction
            .lint_violations
            .iter()
            .any(|v| v.source_line == "# temp"),
        "Expected '# temp' to be linted. Found: {:#?}",
        extraction.lint_violations
    );
    assert!(
        extraction
            .lint_violations
            .iter()
            .any(|v| v.source_line == "// todo"),
        "Expected '// todo' to be linted. Found: {:#?}",
        extraction.lint_violations
    );
}

/// Test that extractor reports TODO lines that do not match any extraction rule.
#[test_log::test]
fn test_extractor_reports_incorrect_todo_syntax() {
    use std::fs;
    use todo_curator::todo::TodoExtractor;

    let temp_dir = std::env::temp_dir().join("todo_curator_extractor_invalid_todo");
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).unwrap();

    let content = r#"
// TODO without a ticket reference
// TODO: still no ticket
// TODO #123: valid
"#;

    fs::write(temp_dir.join("bad_todos.rs"), content).unwrap();

    let extractor = TodoExtractor::new();
    let extraction = extractor.extract_from_directory(&temp_dir).unwrap();

    // Clean up
    let _ = fs::remove_dir_all(&temp_dir);

    assert_eq!(
        extraction.lint_violations.len(),
        2,
        "Expected two extraction violations from extractor. Found: {:#?}",
        extraction.lint_violations
    );
}

/// Test that multiple TODO matches on a single line are all extracted.
#[test_log::test]
fn test_extractor_multiple_todos_on_one_line() {
    use std::fs;
    use todo_curator::todo::{TodoExtractor, TodoReference};

    let temp_dir = std::env::temp_dir().join("todo_curator_multiple_todos_one_line");
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).unwrap();

    let content = r#"
// TODO (#7) TODO #18: TODO(#1,#2)
"#;

    fs::write(temp_dir.join("multi_todo.rs"), content).unwrap();

    let extractor = TodoExtractor::new();
    let extraction = extractor.extract_from_directory(&temp_dir).unwrap();

    // Clean up
    let _ = fs::remove_dir_all(&temp_dir);

    let refs: Vec<_> = extraction
        .references
        .iter()
        .filter(|r| r.file_path().contains("multi_todo.rs"))
        .collect();

    for number in [7_u32, 18_u32, 1_u32, 2_u32] {
        assert!(
            refs.iter().any(|r| matches!(
                r,
                TodoReference {
                    kind: TodoReferenceKind::GitLabIssue {
                        project: None,
                        number: n,
                        ..
                    },
                    ..
                } if *n == number
            )),
            "Should extract local issue #{} from multiple TODOs on one line. Got: {:#?}",
            number,
            refs
        );
    }
}

/// Test that TODO(<ref>) form is correctly extracted as references
#[test_log::test]
fn test_parenthesized_todo_extraction() {
    use std::fs;
    use todo_curator::todo::{TodoExtractor, TodoReference};

    let temp_dir = std::env::temp_dir().join("todo_curator_paren_test");
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).unwrap();

    let content = r#"
// TODO(#100) local GitLab issue
// TODO(rigetti/qcs/magneto#200) cross-project GitLab issue
// TODO(github.com/owner/repo#300) GitHub issue
// TODO(!400) local MR
// TODO(rigetti/qcs/magneto!500) cross-project MR
// TODO(&600) local epic
// TODO(rigetti/qcs/services&700) epic with group
// TODO (#800) local GitLab issue with spacing
// TODO (rigetti/qcs/magneto#810) cross-project GitLab issue with spacing
// TODO (github.com/owner/repo#820) GitHub issue with spacing
// TODO (!830) local MR with spacing
// TODO (&840) local epic with spacing
// TODO(#7, #8, github.com/foo/bar#1) comma-delimited refs
// TODO( #9 , rigetti/qcs/magneto#10 , github.com/owner/repo#11 ) comma-delimited refs with spacing
"#;

    fs::write(temp_dir.join("parens.rs"), content).unwrap();

    let extractor = TodoExtractor::new();
    let extraction = extractor.extract_from_directory(&temp_dir).unwrap();
    let references = extraction.references;

    // Clean up
    let _ = fs::remove_dir_all(&temp_dir);

    // Filter to references from our test file
    let refs: Vec<_> = references
        .iter()
        .filter(|r| r.file_path().contains("parens.rs"))
        .collect();

    // Local GitLab issue
    assert!(
        refs.iter().any(|r| matches!(
            r,
            TodoReference {
                kind: TodoReferenceKind::GitLabIssue {
                    project: None,
                    number: 100,
                    ..
                },
                ..
            }
        )),
        "Should extract TODO(#100). Got: {:#?}",
        refs
    );

    // Cross-project GitLab issue
    assert!(
        refs.iter().any(|r| matches!(
            r,
            TodoReference {
                kind: TodoReferenceKind::GitLabIssue {
                    project: Some(p),
                    number: 200,
                    ..
                },
                ..
            }
            if p == "rigetti/qcs/magneto"
        )),
        "Should extract TODO(rigetti/qcs/magneto#200). Got: {:#?}",
        refs
    );

    // GitHub issue
    assert!(
        refs.iter().any(|r| matches!(
            r,
            TodoReference {
                kind: TodoReferenceKind::GitHubIssueOrPr {
                    repo: Some(repo),
                    number: 300,
                    ..
                },
                ..
            }
            if repo == "owner/repo"
        )),
        "Should extract TODO(github.com/owner/repo#300). Got: {:#?}",
        refs
    );

    // Local MR
    assert!(
        refs.iter().any(|r| matches!(
            r,
            TodoReference {
                kind: TodoReferenceKind::GitLabMr {
                    project: None,
                    number: 400,
                    ..
                },
                ..
            }
        )),
        "Should extract TODO(!400). Got: {:#?}",
        refs
    );

    // Cross-project MR
    assert!(
        refs.iter().any(|r| matches!(
            r,
            TodoReference {
                kind: TodoReferenceKind::GitLabMr {
                    project: Some(p),
                    number: 500,
                    ..
                },
                ..
            }
            if p == "rigetti/qcs/magneto"
        )),
        "Should extract TODO(rigetti/qcs/magneto!500). Got: {:#?}",
        refs
    );

    // Local epic
    assert!(
        refs.iter().any(|r| matches!(
            r,
            TodoReference {
                kind: TodoReferenceKind::GitLabEpic {
                    group: None,
                    number: 600,
                    ..
                },
                ..
            }
        )),
        "Should extract TODO(&600). Got: {:#?}",
        refs
    );

    // Epic with group
    assert!(
        refs.iter().any(|r| matches!(
            r,
            TodoReference {
                kind: TodoReferenceKind::GitLabEpic {
                    group: Some(g),
                    number: 700,
                    ..
                },
                ..
            }
            if g == "rigetti/qcs/services"
        )),
        "Should extract TODO(rigetti/qcs/services&700). Got: {:#?}",
        refs
    );

    // Local GitLab issue with spacing before parenthesized ref
    assert!(
        refs.iter().any(|r| matches!(
            r,
            TodoReference {
                kind: TodoReferenceKind::GitLabIssue {
                    project: None,
                    number: 800,
                    ..
                },
                ..
            }
        )),
        "Should extract TODO (#800). Got: {:#?}",
        refs
    );

    // Cross-project GitLab issue with spacing before parenthesized ref
    assert!(
        refs.iter().any(|r| matches!(
            r,
            TodoReference {
                kind: TodoReferenceKind::GitLabIssue {
                    project: Some(p),
                    number: 810,
                    ..
                },
                ..
            }
            if p == "rigetti/qcs/magneto"
        )),
        "Should extract TODO (rigetti/qcs/magneto#810). Got: {:#?}",
        refs
    );

    // GitHub issue with spacing before parenthesized ref
    assert!(
        refs.iter().any(|r| matches!(
            r,
            TodoReference {
                kind: TodoReferenceKind::GitHubIssueOrPr {
                    repo: Some(repo),
                    number: 820,
                    ..
                },
                ..
            }
            if repo == "owner/repo"
        )),
        "Should extract TODO (github.com/owner/repo#820). Got: {:#?}",
        refs
    );

    // Local MR with spacing before parenthesized ref
    assert!(
        refs.iter().any(|r| matches!(
            r,
            TodoReference {
                kind: TodoReferenceKind::GitLabMr {
                    project: None,
                    number: 830,
                    ..
                },
                ..
            }
        )),
        "Should extract TODO (!830). Got: {:#?}",
        refs
    );

    // Local epic with spacing before parenthesized ref
    assert!(
        refs.iter().any(|r| matches!(
            r,
            TodoReference {
                kind: TodoReferenceKind::GitLabEpic {
                    group: None,
                    number: 840,
                    ..
                },
                ..
            }
        )),
        "Should extract TODO (&840). Got: {:#?}",
        refs
    );

    // Comma-delimited refs: local GitLab issues #7, #8, #9
    for number in [7_u32, 8_u32, 9_u32] {
        assert!(
            refs.iter().any(|r| matches!(
                r,
                TodoReference {
                    kind: TodoReferenceKind::GitLabIssue {
                        project: None,
                        number: n,
                        ..
                    },
                    ..
                } if *n == number
            )),
            "Should extract local issue #{} from comma-delimited TODO list. Got: {:#?}",
            number,
            refs
        );
    }

    // Comma-delimited refs: GitLab rendered issue
    assert!(
        refs.iter().any(|r| matches!(
            r,
            TodoReference {
                kind: TodoReferenceKind::GitLabIssue {
                    project: Some(p),
                    number: 10,
                    ..
                },
                ..
            }
            if p == "rigetti/qcs/magneto"
        )),
        "Should extract rigetti/qcs/magneto#10 from comma-delimited TODO list. Got: {:#?}",
        refs
    );

    // Comma-delimited refs: GitHub shorthand issues
    for (repo, number) in [("foo/bar", 1_u32), ("owner/repo", 11_u32)] {
        assert!(
            refs.iter().any(|r| matches!(
                r,
                TodoReference {
                    kind: TodoReferenceKind::GitHubIssueOrPr {
                        repo: Some(rp),
                        number: n,
                        ..
                    },
                    ..
                }
                if rp == repo && *n == number
            )),
            "Should extract github.com/{repo}#{number} from comma-delimited TODO list. Got: {:#?}",
            refs
        );
    }
}

/// Test that GitLab work item URL forms are parsed as GitLab issues.
#[test_log::test]
fn test_gitlab_work_item_url_extraction() {
    use std::fs;
    use todo_curator::todo::{TodoExtractor, TodoReference};

    let temp_dir = std::env::temp_dir().join("todo_curator_work_items_test");
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).unwrap();

    let content = r#"
// TODO https://gitlab.com/rigetti/qcs/services/compute-v2/-/work_items/123:
// TODO gitlab.com/rigetti/qcs/services/compute-v2/-/work_items/456:
"#;

    fs::write(temp_dir.join("work_items.rs"), content).unwrap();

    let extractor = TodoExtractor::new();
    let extraction = extractor.extract_from_directory(&temp_dir).unwrap();
    let references = extraction.references;

    // Clean up
    let _ = fs::remove_dir_all(&temp_dir);

    let refs: Vec<_> = references
        .iter()
        .filter(|r| r.file_path().contains("work_items.rs"))
        .collect();

    assert!(
        refs.iter().any(|r| matches!(
            r,
            TodoReference {
                kind: TodoReferenceKind::GitLabIssue {
                    project: Some(p),
                    number: 123,
                    ..
                },
                ..
            }
            if p == "rigetti/qcs/services/compute-v2"
        )),
        "Should extract full-schema work_items URL as GitLab issue. Got: {:#?}",
        refs
    );

    assert!(
        refs.iter().any(|r| matches!(
            r,
            TodoReference {
                kind: TodoReferenceKind::GitLabIssue {
                    project: Some(p),
                    number: 456,
                    ..
                },
                ..
            }
            if p == "rigetti/qcs/services/compute-v2"
        )),
        "Should extract no-schema work_items URL as GitLab issue. Got: {:#?}",
        refs
    );
}

/// Test shorthand disambiguation: owner/repo#N is GitLab-style, github.com/owner/repo#N is GitHub-style.
#[test_log::test]
fn test_cross_project_shorthand_disambiguation() {
    use std::fs;
    use todo_curator::todo::{TodoExtractor, TodoReference};

    let temp_dir = std::env::temp_dir().join("todo_curator_cross_project_shorthand_test");
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).unwrap();

    let content = r#"
// TODO owner/repo#7: rendered shorthand resolves as GitLab issue
// TODO github.com/owner/repo#7: explicit GitHub shorthand
"#;

    fs::write(temp_dir.join("shorthand.rs"), content).unwrap();

    let extractor = TodoExtractor::new();
    let extraction = extractor.extract_from_directory(&temp_dir).unwrap();

    // Clean up
    let _ = fs::remove_dir_all(&temp_dir);

    assert!(
        extraction.lint_violations.is_empty(),
        "Expected no lint violations. Got: {:#?}",
        extraction.lint_violations
    );

    let refs: Vec<_> = extraction
        .references
        .iter()
        .filter(|r| r.file_path().contains("shorthand.rs"))
        .collect();

    assert!(
        refs.iter().any(|r| matches!(
            r,
            TodoReference {
                kind: TodoReferenceKind::GitLabIssue {
                    project: Some(p),
                    number: 7,
                    ..
                },
                ..
            }
            if p == "owner/repo"
        )),
        "owner/repo#7 should parse as GitLabIssue. Got: {:#?}",
        refs
    );

    assert!(
        refs.iter().any(|r| matches!(
            r,
            TodoReference {
                kind: TodoReferenceKind::GitHubIssueOrPr {
                    repo: Some(p),
                    number: 7,
                    ..
                },
                ..
            }
            if p == "owner/repo"
        )),
        "github.com/owner/repo#7 should parse as GitHubIssueOrPr. Got: {:#?}",
        refs
    );
}

/// Files (relative to the scan root) matched by the default exclusion regex.
const DEFAULT_EXCLUDED_FIXTURES: &[&str] = &[
    "Cargo.lock",
    "sub/Cargo.lock",
    "relint-foo.yaml",
    "relint.yml",
    "mermaid.min.js",
    "CHANGELOG.md",
    "sub/CHANGELOG.md",
    "crates/foo/CHANGELOG.md",
];

/// Files that the default exclusion regex must not match. The relint and mermaid
/// alternatives are anchored to the scan root, so nested copies are still checked.
const NOT_DEFAULT_EXCLUDED_FIXTURES: &[&str] = &[
    "keep.rs",
    "todo.yaml",
    "MY_CHANGELOG.md",
    "docs/MY_CHANGELOG.md",
    "docs/relint-foo.yaml",
    "sub/mermaid.min.js",
];

fn all_exclusion_fixtures() -> Vec<&'static str> {
    DEFAULT_EXCLUDED_FIXTURES
        .iter()
        .chain(NOT_DEFAULT_EXCLUDED_FIXTURES)
        .copied()
        .collect()
}

/// Create a fresh directory containing every exclusion fixture,
/// each with one invalid TODO-like line.
fn write_exclusion_fixtures(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(name);
    let _ = std::fs::remove_dir_all(&root);
    for rel in all_exclusion_fixtures() {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "- reword comment to avoid TODO-lint false positive\n",
        )
        .unwrap();
    }
    root
}

/// Violation file paths, relative to `root`, with `/` separators, sorted.
fn violation_paths(
    root: &std::path::Path,
    violations: &[todo_curator::todo::LintViolation],
) -> Vec<String> {
    let mut paths: Vec<String> = violations
        .iter()
        .map(|v| {
            let p = std::path::Path::new(&v.file_path);
            let p = p.strip_prefix(root).unwrap_or(p);
            p.components()
                .filter_map(|c| match c {
                    std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("/")
        })
        .collect();
    paths.sort();
    paths
}

fn sorted(items: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = items.iter().map(|s| (*s).to_string()).collect();
    v.sort();
    v
}

fn without(items: &[&str], excluded: &[&str]) -> Vec<String> {
    let kept: Vec<&str> = items
        .iter()
        .copied()
        .filter(|p| !excluded.contains(p))
        .collect();
    sorted(&kept)
}

/// Lock files, relint configs, mermaid bundles, and the root changelog are skipped
/// by default, without any `--exclude-file-regex`.
#[test_log::test]
fn test_default_exclusions_apply_without_custom_regex() {
    let root = write_exclusion_fixtures("todo_curator_default_exclusions");

    let extraction = extract_todos(&root, "").unwrap();
    let _ = std::fs::remove_dir_all(&root);

    assert_eq!(
        violation_paths(&root, &extraction.lint_violations),
        sorted(NOT_DEFAULT_EXCLUDED_FIXTURES),
        "Default-excluded files should not be linted. Got: {:#?}",
        extraction.lint_violations
    );
}

/// A user-supplied exclude regex adds to the default exclusions rather than
/// replacing them (the failure mode in qcs-infrastructure!92, where a job-level
/// override dropped the template's `CHANGELOG\.md` alternative).
#[test_log::test]
fn test_custom_exclude_regex_is_additive_to_defaults() {
    let root = write_exclusion_fixtures("todo_curator_additive_exclusions");

    let extraction = extract_todos(&root, r"todo\.yaml").unwrap();
    let _ = std::fs::remove_dir_all(&root);

    assert_eq!(
        violation_paths(&root, &extraction.lint_violations),
        without(NOT_DEFAULT_EXCLUDED_FIXTURES, &["todo.yaml"]),
        "Custom regex should apply in addition to the defaults. Got: {:#?}",
        extraction.lint_violations
    );
}

/// `TodoExtractor::exclude_file_defaults(false)` disables only the default exclusions.
#[test_log::test]
fn test_exclude_file_defaults_disabled() {
    use todo_curator::todo::TodoExtractor;
    let root = write_exclusion_fixtures("todo_curator_defaults_disabled");

    let extraction = TodoExtractor::with_exclude_file_regex(r"todo\.yaml")
        .unwrap()
        .exclude_file_defaults(false)
        .extract_from_directory(&root)
        .unwrap();
    let _ = std::fs::remove_dir_all(&root);

    assert_eq!(
        violation_paths(&root, &extraction.lint_violations),
        without(&all_exclusion_fixtures(), &["todo.yaml"]),
        "Only the custom regex should apply. Got: {:#?}",
        extraction.lint_violations
    );
}

/// Run `check-invalid --format json` in `root` and return the violation paths,
/// without the `./` prefix, sorted.
fn cli_violation_paths(
    root: &std::path::Path,
    args: &[&str],
    envs: &[(&str, &str)],
) -> Vec<String> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_todo-curator"));
    cmd.args(["check-invalid", "--format", "json"])
        .args(args)
        .current_dir(root)
        .env_remove("TODO_CURATOR_EXCLUDE_FILE_REGEX")
        .env_remove("TODO_CURATOR_EXCLUDE_FILE_DEFAULTS")
        .env_remove("TODO_CURATOR_FORMAT")
        .env_remove("TODO_CURATOR_PATH")
        .env_remove("RUST_LOG");
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let output = cmd.output().expect("Failed to execute todo-curator");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Log lines (tracing writes to stdout) precede the JSON document.
    let json_start = if stdout.starts_with('{') {
        0
    } else {
        stdout.find("\n{").map_or(0, |i| i + 1)
    };
    let json: serde_json::Value = serde_json::from_str(&stdout[json_start..]).unwrap_or_else(|e| {
        panic!("Expected JSON output ({e}). Got:\nSTDOUT:\n{stdout}\nSTDERR:\n{stderr}")
    });
    let mut paths: Vec<String> = json["lint_violations"]
        .as_array()
        .unwrap_or_else(|| panic!("Expected lint_violations array. Got:\n{stdout}"))
        .iter()
        .map(|v| {
            let p = v["file_path"]
                .as_str()
                .expect("file_path should be a string");
            p.strip_prefix("./").unwrap_or(p).to_string()
        })
        .collect();
    paths.sort();
    paths
}

/// The CLI applies the default exclusions, additively with `--exclude-file-regex`.
#[test_log::test]
fn test_cli_default_exclusions_are_additive() {
    let root = write_exclusion_fixtures("todo_curator_cli_default_exclusions");
    let default_only = cli_violation_paths(&root, &[], &[]);
    let with_custom = cli_violation_paths(&root, &["--exclude-file-regex", r"todo\.yaml"], &[]);
    let with_custom_env = cli_violation_paths(
        &root,
        &[],
        &[("TODO_CURATOR_EXCLUDE_FILE_REGEX", r"todo\.yaml")],
    );
    let _ = std::fs::remove_dir_all(&root);

    assert_eq!(default_only, sorted(NOT_DEFAULT_EXCLUDED_FIXTURES));
    let expected = without(NOT_DEFAULT_EXCLUDED_FIXTURES, &["todo.yaml"]);
    assert_eq!(with_custom, expected);
    assert_eq!(with_custom_env, expected);
}

/// `--exclude-file-defaults=false` disables the default exclusions; the user regex
/// still applies.
#[test_log::test]
fn test_cli_exclude_file_defaults_false_flag() {
    let root = write_exclusion_fixtures("todo_curator_cli_defaults_off_flag");
    let paths = cli_violation_paths(
        &root,
        &[
            "--exclude-file-defaults=false",
            "--exclude-file-regex",
            r"todo\.yaml",
        ],
        &[],
    );
    let _ = std::fs::remove_dir_all(&root);

    assert_eq!(paths, without(&all_exclusion_fixtures(), &["todo.yaml"]));
}

/// Falsey `TODO_CURATOR_EXCLUDE_FILE_DEFAULTS` disables the default exclusions;
/// the user regex still applies.
#[test_log::test]
fn test_cli_exclude_file_defaults_env_falsey() {
    let root = write_exclusion_fixtures("todo_curator_cli_defaults_off_env");
    let expected = without(&all_exclusion_fixtures(), &["todo.yaml"]);
    for value in ["false", "0", "no", "off", "FALSE"] {
        let paths = cli_violation_paths(
            &root,
            &[],
            &[
                ("TODO_CURATOR_EXCLUDE_FILE_DEFAULTS", value),
                ("TODO_CURATOR_EXCLUDE_FILE_REGEX", r"todo\.yaml"),
            ],
        );
        assert_eq!(
            paths, expected,
            "TODO_CURATOR_EXCLUDE_FILE_DEFAULTS={value:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// Truthy or empty `TODO_CURATOR_EXCLUDE_FILE_DEFAULTS` keeps the defaults. Empty
/// matters because CI systems routinely pass unset variables through as empty strings.
#[test_log::test]
fn test_cli_exclude_file_defaults_env_truthy_or_empty() {
    let root = write_exclusion_fixtures("todo_curator_cli_defaults_on_env");
    let expected = sorted(NOT_DEFAULT_EXCLUDED_FIXTURES);
    for value in ["", "true", "1", "yes", "on"] {
        let paths =
            cli_violation_paths(&root, &[], &[("TODO_CURATOR_EXCLUDE_FILE_DEFAULTS", value)]);
        assert_eq!(
            paths, expected,
            "TODO_CURATOR_EXCLUDE_FILE_DEFAULTS={value:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

fn run_check_invalid(root: &std::path::Path, args: &[&str], envs: &[(&str, &str)]) -> String {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_todo-curator"));
    cmd.arg("check-invalid")
        .args(args)
        .current_dir(root)
        .env_remove("TODO_CURATOR_EXCLUDE_FILE_REGEX")
        .env_remove("TODO_CURATOR_EXCLUDE_FILE_DEFAULTS")
        .env_remove("TODO_CURATOR_FORMAT")
        .env_remove("TODO_CURATOR_PATH")
        .env_remove("RUST_LOG");
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let output = cmd.output().expect("Failed to execute todo-curator");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stdout.contains("Invalid TODO-like comments:"),
        "Expected invalid TODO output. Got:\nSTDOUT:\n{stdout}\nSTDERR:\n{stderr}"
    );
    stdout
}

/// The exclude regex is matched against the path relative to `--path`, *without* a
/// leading `./`, even though violations are reported as `./<file>` when `--path` is
/// the default `.`. So anchored patterns such as `^notes\.md$` work as expected.
#[test_log::test]
fn test_cli_exclude_regex_is_matched_without_dot_slash_prefix() {
    let root = std::env::temp_dir().join("todo_curator_exclude_dot_slash");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("sub")).unwrap();
    for rel in ["notes.md", "sub/notes.md", "keep.rs"] {
        std::fs::write(root.join(rel), "// TODO without a reference\n").unwrap();
    }

    let stdout = run_check_invalid(&root, &["--exclude-file-regex", r"^notes\.md$"], &[]);
    let _ = std::fs::remove_dir_all(&root);

    assert!(
        stdout.contains("./keep.rs") && stdout.contains("./sub/notes.md"),
        "Unanchored-match files should be reported with a ./ prefix. Got:\n{stdout}"
    );
    assert!(
        !stdout.contains("./notes.md"),
        "^notes\\.md$ should exclude the top-level notes.md. Got:\n{stdout}"
    );
}
