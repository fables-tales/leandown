use anyhow::{Context, Result};
use std::path::Path;

const TPL_PACKAGE_JSON: &str = include_str!("templates/package.json");
const TPL_INPUT_CSS: &str    = include_str!("templates/input.css");
const TPL_GITIGNORE: &str    = include_str!("templates/gitignore");

/// Scaffold a leandown frontend bundle inside `<root>/leandown_site/`.
/// Skips files that already exist so re-running init is safe.
pub fn init(root: &Path) -> Result<()> {
    let site_dir = root.join("leandown_site");
    let src_dir  = site_dir.join("src");
    let out_dir  = site_dir.join("output");

    std::fs::create_dir_all(&src_dir).context("creating leandown_site/src/")?;
    std::fs::create_dir_all(&out_dir).context("creating leandown_site/output/")?;

    write_if_new(&site_dir.join("package.json"), TPL_PACKAGE_JSON)?;
    write_if_new(&src_dir.join("input.css"),     TPL_INPUT_CSS)?;
    write_if_new(&site_dir.join(".gitignore"),   TPL_GITIGNORE)?;

    println!("\nInitialised leandown frontend bundle in {}/", site_dir.display());
    println!("\nNext steps:");
    println!("  cd {}", site_dir.display());
    println!("  npm install");
    println!("\nThen build the site:");
    println!("  leandown build --root {}", root.display());

    Ok(())
}

fn write_if_new(path: &Path, content: &str) -> Result<()> {
    if path.exists() {
        println!("  skip   {} (already exists)", path.display());
    } else {
        std::fs::write(path, content)
            .with_context(|| format!("writing {}", path.display()))?;
        println!("  create {}", path.display());
    }
    Ok(())
}
