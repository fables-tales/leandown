use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

const TPL_PACKAGE_JSON: &str  = include_str!("templates/package.json");
const TPL_INPUT_CSS: &str     = include_str!("templates/input.css");
const TPL_GITIGNORE: &str     = include_str!("templates/gitignore");
const TPL_SCRIPT_BUILD: &str  = include_str!("templates/script_build");
const TPL_SCRIPT_SERVER: &str = include_str!("templates/script_server");

/// Scaffold a leandown frontend bundle inside `<root>/leandown_site/`.
/// Skips files that already exist so re-running init is safe.
pub fn init(root: &Path) -> Result<()> {
    check_dependencies();

    let site_dir   = root.join("leandown_site");
    let src_dir    = site_dir.join("src");
    let out_dir    = site_dir.join("output");
    let script_dir = site_dir.join("script");

    std::fs::create_dir_all(&src_dir).context("creating leandown_site/src/")?;
    std::fs::create_dir_all(&out_dir).context("creating leandown_site/output/")?;
    std::fs::create_dir_all(&script_dir).context("creating leandown_site/script/")?;

    write_if_new(&site_dir.join("package.json"), TPL_PACKAGE_JSON)?;
    write_if_new(&src_dir.join("input.css"),     TPL_INPUT_CSS)?;
    write_if_new(&site_dir.join(".gitignore"),   TPL_GITIGNORE)?;

    write_script(&script_dir.join("build"),  TPL_SCRIPT_BUILD)?;
    write_script(&script_dir.join("server"), TPL_SCRIPT_SERVER)?;

    println!("\nRunning npm install...");
    let status = Command::new("npm")
        .arg("install")
        .current_dir(&site_dir)
        .status()
        .context("failed to run npm install — is npm installed?")?;

    if !status.success() {
        anyhow::bail!("npm install failed");
    }

    println!("\nDone! Use the convenience scripts from your project root:");
    println!("  leandown_site/script/build    # one-shot build");
    println!("  leandown_site/script/server   # dev server with live reload");

    Ok(())
}

/// Warn about missing tools before doing any work.
fn check_dependencies() {
    if !command_exists("lake") {
        eprintln!("warning: lake not found — Lean files will not be validated during build.");
        eprintln!("         Install Lean via elan: https://github.com/leanprover/elan");
    }
    if !command_exists("npm") {
        eprintln!("error:   npm not found — cannot install the CSS bundler.");
        eprintln!("         Install Node.js from https://nodejs.org");
        std::process::exit(1);
    }
}

fn command_exists(name: &str) -> bool {
    Command::new(name)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
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

/// Write a script file and mark it executable. Always overwrites so the
/// canonical scripts stay in sync with the installed binary version.
fn write_script(path: &Path, content: &str) -> Result<()> {
    std::fs::write(path, content)
        .with_context(|| format!("writing {}", path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .with_context(|| format!("setting permissions on {}", path.display()))?;
    }

    println!("  create {}", path.display());
    Ok(())
}
