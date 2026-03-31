/// End-to-end smoke test: initialise a real Lean project, add an annotated
/// file with a valid proof, build the site, and verify the output.
///
/// Requires `lake` and `npm` on PATH. Skipped automatically if either is
/// missing so CI environments without Lean can still run `cargo test`.

use std::fs;
use std::path::Path;
use std::process::Command;

fn has_command(name: &str) -> bool {
    Command::new(name)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

fn leandown_binary() -> std::path::PathBuf {
    std::env::current_exe()
        .unwrap()
        .parent()   // deps/
        .unwrap()
        .parent()   // debug/ or release/
        .unwrap()
        .join("leandown")
}

#[test]
fn smoke_build() {
    if !has_command("lake") {
        eprintln!("smoke_build: skipping — lake not found");
        return;
    }
    if !has_command("npm") {
        eprintln!("smoke_build: skipping — npm not found");
        return;
    }

    let project = std::env::temp_dir().join("leandown_smoke_test");
    if project.exists() {
        fs::remove_dir_all(&project).unwrap();
    }
    fs::create_dir_all(&project).unwrap();

    // ── 1. lake init (creates lakefile.toml and SmokeTest.lean in project/) ──
    //
    // `lake init SmokeTest lib` creates the project files IN the cwd, with
    // `SmokeTest` as the library/module name prefix. It does NOT create a
    // subdirectory named SmokeTest.
    let status = Command::new("lake")
        .args(["init", "SmokeTest", "lib"])
        .current_dir(&project)
        .status()
        .expect("lake init failed");
    assert!(status.success(), "lake init failed");

    // ── 2. Overwrite the root module with a leandown-annotated proof ──────────
    let lean_src = r#"-- leandown
-- [meta]
-- title = "Smoke Test"
-- group  = "Tests"
-- [content]

-- # {{theorem}}: One equals one
--
-- The simplest possible proof, included to verify the build pipeline.
theorem one_eq_one : 1 = 1 := by
  -- `rfl` closes the goal because both sides are definitionally equal
  rfl
"#;
    fs::write(project.join("SmokeTest.lean"), lean_src).unwrap();

    // ── 3. leandown init ──────────────────────────────────────────────────────
    let binary = leandown_binary();
    let status = Command::new(&binary)
        .args(["init", project.to_str().unwrap()])
        .status()
        .expect("leandown init failed");
    assert!(status.success(), "leandown init failed");

    // ── 4. leandown build ─────────────────────────────────────────────────────
    let output_dir = project.join("leandown_site").join("output");
    let status = Command::new(&binary)
        .args([
            "build",
            "--root",   project.to_str().unwrap(),
            "--output", output_dir.to_str().unwrap(),
        ])
        .status()
        .expect("leandown build failed");
    assert!(status.success(), "leandown build failed");

    // ── 5. Verify output ──────────────────────────────────────────────────────
    assert_file_contains(&output_dir.join("index.html"),     "leandown");
    assert_file_contains(&output_dir.join("SmokeTest.html"), "one_eq_one");
    assert_file_contains(&output_dir.join("SmokeTest.html"), "Theorem 1");
    assert_file_contains(&output_dir.join("SmokeTest.html"), "One equals one");
    assert!(output_dir.join("styles.css").exists(),          "styles.css missing");

    fs::remove_dir_all(&project).unwrap();
    println!("smoke_build: passed");
}

fn assert_file_contains(path: &Path, needle: &str) {
    let content = fs::read_to_string(path)
        .unwrap_or_else(|_| panic!("could not read {}", path.display()));
    assert!(
        content.contains(needle),
        "{} did not contain {:?}",
        path.display(),
        needle
    );
}
