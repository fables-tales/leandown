use crate::parse::{page_file, parse};
use crate::render::{render_index, render_page};
use crate::validate::validate;
use crate::xref::{compute_backlinks, process_tags_and_xref};
use anyhow::{Context, Result};
use std::collections::HashSet;
use std::path::Path;
use walkdir::WalkDir;

/// Pre-built Tailwind CSS embedded in the binary.
const STYLES_CSS: &str = include_str!("assets/styles.css");

pub fn build(root: &Path, output: &Path) -> Result<()> {
    std::fs::create_dir_all(output)
        .with_context(|| format!("creating output dir {}", output.display()))?;

    // Collect all .lean files under root, excluding anything inside output.
    let lean_files: Vec<_> = WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| !e.path().starts_with(output))
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "lean"))
        .map(|e| e.into_path())
        .collect();

    // Parse all annotated files.
    let mut pages = Vec::new();
    for path in &lean_files {
        match parse(path, root) {
            Ok(Some(p)) => pages.push(p),
            Ok(None) => {}
            Err(e) => eprintln!("  WARN: failed to parse {}: {e}", path.display()),
        }
    }

    if pages.is_empty() {
        println!("No leandown-annotated files found.");
        return Ok(());
    }

    // Validate each page with `lake build`.
    pages.retain(|p| validate(p, root));

    if pages.is_empty() {
        println!("No files passed validation.");
        return Ok(());
    }

    let ld_mods: HashSet<String> = pages.iter().map(|p| p.module.clone()).collect();

    // Process tags and cross-references (mutates page blocks in place).
    let xref_data = process_tags_and_xref(&mut pages);
    let backlinks = compute_backlinks(&pages);

    // Render and write each page.
    for page in &pages {
        let html = render_page(page, &pages, &ld_mods, &xref_data, &backlinks);
        let out_path = output.join(page_file(&page.module));
        std::fs::write(&out_path, html)
            .with_context(|| format!("writing {}", out_path.display()))?;
        println!(
            "  {:<45} → {}",
            page.module,
            out_path
                .strip_prefix(root)
                .unwrap_or(&out_path)
                .display()
        );
    }

    // Render and write the index.
    let index_path = output.join("index.html");
    std::fs::write(&index_path, render_index(&pages))
        .with_context(|| format!("writing {}", index_path.display()))?;
    println!(
        "  {:<45} → {}",
        "index",
        index_path
            .strip_prefix(root)
            .unwrap_or(&index_path)
            .display()
    );

    // Write embedded CSS.
    let css_path = output.join("styles.css");
    std::fs::write(&css_path, STYLES_CSS)
        .with_context(|| format!("writing {}", css_path.display()))?;
    println!("  {:<45} → styles.css ({} bytes)", "css", STYLES_CSS.len());

    println!("\n✓  open {}", index_path.strip_prefix(root).unwrap_or(&index_path).display());
    Ok(())
}
