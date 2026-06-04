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

/// Test that the extractor correctly parses GitLab epic formats
#[test]
fn test_epic_extraction() {
    use std::fs;
    use todo_curator::todo::{TodoExtractor, TodoReference};

    let temp_dir = std::env::temp_dir();
    let test_file = temp_dir.join("test_epic_extraction.rs");

    // Create a test file with various epic formats
    let content = r#"
// TODO &17 - local epic reference
// TODO rigetti/qcs/services&42 - epic with group path
// TODO https://gitlab.com/groups/rigetti/qcs/services/-/epics/123 - full URL
// TODO gitlab.com/groups/rigetti/qcs/services/-/epics/456 - URL without schema
"#;

    fs::write(&test_file, content).expect("Failed to write test file");

    let extractor = TodoExtractor::new();
    let references = extractor
        .extract_from_directory(&temp_dir)
        .expect("Failed to extract references");

    // Clean up
    let _ = fs::remove_file(&test_file);

    // Filter to only epic references from our test file
    let epic_refs: Vec<_> = references
        .iter()
        .filter(|r| matches!(r, TodoReference::GitLabEpic { .. }))
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
            TodoReference::GitLabEpic {
                group: None,
                number: 17,
                ..
            }
        )
    });
    assert!(has_local, "Should find local epic &17");

    // Verify epic with group path
    let has_group_path = epic_refs.iter().any(|r| {
        matches!(
            r,
            TodoReference::GitLabEpic { group: Some(g), number: 42, .. }
            if g == "rigetti/qcs/services"
        )
    });
    assert!(has_group_path, "Should find epic rigetti/qcs/services&42");

    // Verify full URL epic
    let has_full_url = epic_refs.iter().any(|r| {
        matches!(
            r,
            TodoReference::GitLabEpic { group: Some(g), number: 123, .. }
            if g == "rigetti/qcs/services"
        )
    });
    assert!(has_full_url, "Should find epic from full URL");

    // Verify URL without schema
    let has_no_schema = epic_refs.iter().any(|r| {
        matches!(
            r,
            TodoReference::GitLabEpic { group: Some(g), number: 456, .. }
            if g == "rigetti/qcs/services"
        )
    });
    assert!(has_no_schema, "Should find epic from URL without schema");
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

/// Test that the linter catches forbidden TODO patterns
#[test]
fn test_lint_forbidden_patterns() {
    use std::fs;
    use todo_curator::todo::{LintViolation, TodoLinter};

    let temp_dir = std::env::temp_dir().join("todo_curator_lint_test_forbidden");
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).unwrap();

    let content = r#"
// XXX this is bad
// FIXME something broken
// TEMP hack
// TBD figure this out
// MVP feature
// TODO without a ticket reference
// TODO: also bad, colon but no ticket
// todo lowercase is bad
// fixme lowercase
# tbd in a shell comment
"#;

    fs::write(temp_dir.join("bad.rs"), content).unwrap();

    let linter = TodoLinter::new();
    let violations = linter.lint_directory(&temp_dir).unwrap();

    // Clean up
    let _ = fs::remove_dir_all(&temp_dir);

    // Should catch XXX, FIXME, TEMP, TBD (non-mergeable)
    let non_mergeable: Vec<&LintViolation> = violations
        .iter()
        .filter(|v| v.rule == "non-mergeable TODOs")
        .collect();
    assert!(
        non_mergeable.len() >= 4,
        "Should catch XXX, FIXME, TEMP, TBD. Found: {:?}",
        non_mergeable
    );

    // Should catch MVP
    let mvp: Vec<&LintViolation> = violations
        .iter()
        .filter(|v| v.rule == "MVP comments")
        .collect();
    assert!(!mvp.is_empty(), "Should catch MVP comment");

    // Should catch TODOs with incorrect syntax
    let bad_syntax: Vec<&LintViolation> = violations
        .iter()
        .filter(|v| v.rule == "TODOs with incorrect syntax")
        .collect();
    assert!(
        bad_syntax.len() >= 2,
        "Should catch TODO without ticket and TODO: without ticket. Found: {:?}",
        bad_syntax
    );

    // Should catch uncapitalized patterns
    let uncap: Vec<&LintViolation> = violations
        .iter()
        .filter(|v| v.rule == "uncapitalized TODO patterns")
        .collect();
    assert!(
        uncap.len() >= 3,
        "Should catch todo, fixme, tbd lowercase. Found: {:?}",
        uncap
    );
}

/// Test that valid TODO patterns are NOT flagged by the linter
#[test]
fn test_lint_valid_patterns() {
    use std::fs;
    use todo_curator::todo::TodoLinter;

    let temp_dir = std::env::temp_dir().join("todo_curator_lint_test_valid");
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).unwrap();

    let content = r#"
// TODO #123
// TODO rigetti/qcs/magneto#229
// TODO github.com/owner/repo#456
// TODO !789
// TODO rigetti/qcs/magneto!100
// TODO &17
// TODO rigetti/qcs/services&42
// TODO performance - allowed exception
// TODO(#123) parenthesized local issue
// TODO(rigetti/qcs/magneto#229) parenthesized GitLab issue
// TODO(github.com/owner/repo#456) parenthesized GitHub issue
// TODO(!789) parenthesized MR
// TODO(&17) parenthesized epic
// using todo-curator in a comment is fine
"#;

    fs::write(temp_dir.join("good.rs"), content).unwrap();

    let linter = TodoLinter::new();
    let violations = linter.lint_directory(&temp_dir).unwrap();

    // Clean up
    let _ = fs::remove_dir_all(&temp_dir);

    // Filter out violations that come from the "todo-curator" line (should be excluded)
    // All lines should pass - no violations expected
    assert!(
        violations.is_empty(),
        "Valid TODO patterns should not produce lint violations. Got: {:#?}",
        violations
    );
}

/// Test that TODO(<ref>) form is correctly extracted as references
#[test]
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
"#;

    fs::write(temp_dir.join("parens.rs"), content).unwrap();

    let extractor = TodoExtractor::new();
    let references = extractor.extract_from_directory(&temp_dir).unwrap();

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
            TodoReference::GitLabIssue { project: None, number: 100, .. }
        )),
        "Should extract TODO(#100). Got: {:#?}",
        refs
    );

    // Cross-project GitLab issue
    assert!(
        refs.iter().any(|r| matches!(
            r,
            TodoReference::GitLabIssue { project: Some(p), number: 200, .. }
            if p == "rigetti/qcs/magneto"
        )),
        "Should extract TODO(rigetti/qcs/magneto#200). Got: {:#?}",
        refs
    );

    // GitHub issue
    assert!(
        refs.iter().any(|r| matches!(
            r,
            TodoReference::GitHubIssue { repo: Some(repo), number: 300, .. }
            if repo == "owner/repo"
        )),
        "Should extract TODO(github.com/owner/repo#300). Got: {:#?}",
        refs
    );

    // Local MR
    assert!(
        refs.iter().any(|r| matches!(
            r,
            TodoReference::GitLabMr { project: None, number: 400, .. }
        )),
        "Should extract TODO(!400). Got: {:#?}",
        refs
    );

    // Cross-project MR
    assert!(
        refs.iter().any(|r| matches!(
            r,
            TodoReference::GitLabMr { project: Some(p), number: 500, .. }
            if p == "rigetti/qcs/magneto"
        )),
        "Should extract TODO(rigetti/qcs/magneto!500). Got: {:#?}",
        refs
    );

    // Local epic
    assert!(
        refs.iter().any(|r| matches!(
            r,
            TodoReference::GitLabEpic { group: None, number: 600, .. }
        )),
        "Should extract TODO(&600). Got: {:#?}",
        refs
    );

    // Epic with group
    assert!(
        refs.iter().any(|r| matches!(
            r,
            TodoReference::GitLabEpic { group: Some(g), number: 700, .. }
            if g == "rigetti/qcs/services"
        )),
        "Should extract TODO(rigetti/qcs/services&700). Got: {:#?}",
        refs
    );
}
