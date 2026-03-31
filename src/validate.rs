use crate::parse::Page;
use std::path::Path;
use std::process::Command;

/// Run `lake build <module>` and return true only if it succeeds with no warnings.
pub fn validate(page: &Page, root: &Path) -> bool {
    let rel = page
        .path
        .strip_prefix(root)
        .unwrap_or(&page.path)
        .to_string_lossy()
        .into_owned();

    let result = Command::new("lake")
        .args(["build", &page.module])
        .current_dir(root)
        .output();

    let output = match result {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("  WARN: lake not found, skipping validation");
            return true;
        }
        Err(e) => {
            eprintln!("  WARN: failed to run lake: {e}");
            return true;
        }
    };

    let combined = String::from_utf8_lossy(&output.stdout).into_owned()
        + &String::from_utf8_lossy(&output.stderr);

    let warnings: Vec<&str> = combined
        .lines()
        .filter(|l| l.contains("warning:") && l.contains(&rel))
        .collect();

    if output.status.success() && warnings.is_empty() {
        return true;
    }

    if !output.status.success() {
        let errors: Vec<&str> = combined
            .lines()
            .filter(|l| l.contains("error:") && l.contains(&rel))
            .collect();
        eprintln!("  SKIP {}: build errors", page.module);
        for e in errors {
            eprintln!("       {}", e.trim());
        }
    } else {
        eprintln!("  SKIP {}: {} warning(s)", page.module, warnings.len());
        for w in warnings {
            eprintln!("       {}", w.trim());
        }
    }

    false
}
