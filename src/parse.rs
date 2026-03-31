use anyhow::Result;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Deserialize;
use std::path::{Path, PathBuf};

static HIDDEN_OPEN_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^-- \[hidden(?::\s*(.+))?\]$").unwrap());
static HIDDEN_CLOSE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^-- \[/hidden\]$").unwrap());
static IMPORT_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^import\s+(\S+)").unwrap());

#[derive(Debug, Clone)]
pub enum Block {
    Prose(String),
    Code(String),
    HiddenOpen(String), // label
    HiddenClose,
}

#[derive(Debug, Clone)]
pub struct Page {
    pub path: PathBuf,
    pub module: String,
    pub title: String,
    pub group: String,
    pub imports: Vec<String>,
    pub blocks: Vec<Block>,
    pub raw: String,
}

#[derive(Debug, Deserialize, Default)]
struct Meta {
    title: Option<String>,
    group: Option<String>,
}

/// Derive the module name from a file path relative to the project root.
/// e.g. `Onewayf/Foo.lean` → `Onewayf.Foo`
pub fn module_of(path: &Path, root: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let s = rel.to_string_lossy();
    s.replace(std::path::MAIN_SEPARATOR, ".")
        .trim_end_matches(".lean")
        .to_owned()
}

/// Derive the output HTML filename from a module name.
/// e.g. `Onewayf.Foo` → `Foo.html`
pub fn page_file(module: &str) -> String {
    format!("{}.html", module.split('.').last().unwrap_or(module))
}

/// Parse a leandown-annotated Lean file. Returns None if the file has no `-- leandown` marker.
pub fn parse(path: &Path, root: &Path) -> Result<Option<Page>> {
    let raw = std::fs::read_to_string(path)?;
    let lines: Vec<&str> = raw.lines().collect();

    // Find the `-- leandown` marker line.
    let ld = match lines.iter().position(|l| l.trim() == "-- leandown") {
        Some(i) => i,
        None => return Ok(None),
    };

    // Collect import statements from before the marker.
    let imports: Vec<String> = lines[..ld]
        .iter()
        .filter_map(|l| IMPORT_RE.captures(l).map(|c| c[1].to_owned()))
        .collect();

    // Parse meta block between `-- [meta]` and `-- [content]`.
    let mut meta_lines: Vec<String> = Vec::new();
    let mut in_meta = false;
    let mut content_start = None;

    for (i, line) in lines[ld + 1..].iter().enumerate() {
        let s = line.trim();
        if s == "-- [meta]" {
            in_meta = true;
        } else if s == "-- [content]" {
            content_start = Some(ld + 1 + i + 1);
            break;
        } else if in_meta {
            let stripped = if s.starts_with("-- ") {
                s[3..].to_owned()
            } else if s == "--" {
                String::new()
            } else {
                s.to_owned()
            };
            meta_lines.push(stripped);
        }
    }

    let content_start = match content_start {
        Some(i) => i,
        None => return Ok(None),
    };

    // Parse TOML meta.
    let meta: Meta = if meta_lines.is_empty() {
        Meta::default()
    } else {
        let toml_str = meta_lines.join("\n");
        match toml::from_str(&toml_str) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("TOML warning ({}): {}", path.display(), e);
                Meta::default()
            }
        }
    };

    let title = meta
        .title
        .unwrap_or_else(|| module_of(path, root).split('.').last().unwrap_or("").to_owned());
    let group = meta.group.unwrap_or_else(|| "Other".to_owned());

    // Parse content into blocks.
    let blocks = parse_blocks(&lines[content_start..]);
    let module = module_of(path, root);

    Ok(Some(Page {
        path: path.to_owned(),
        module,
        title,
        group,
        imports,
        blocks,
        raw,
    }))
}

#[derive(PartialEq, Clone, Copy)]
enum BlockType {
    Prose,
    Code,
}

fn parse_blocks(lines: &[&str]) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut cur_type: Option<BlockType> = None;
    let mut cur: Vec<String> = Vec::new();

    let flush = |cur_type: &Option<BlockType>, cur: &mut Vec<String>, blocks: &mut Vec<Block>| {
        // Trim trailing blank lines.
        while cur.last().map_or(false, |l: &String| l.trim().is_empty()) {
            cur.pop();
        }
        if !cur.is_empty() {
            let text = cur.join("\n");
            match cur_type {
                Some(BlockType::Prose) => blocks.push(Block::Prose(text)),
                Some(BlockType::Code) => blocks.push(Block::Code(text)),
                None => {}
            }
        }
        cur.clear();
    };

    for line in lines {
        let s = line.trim();

        if let Some(caps) = HIDDEN_OPEN_RE.captures(s) {
            flush(&cur_type, &mut cur, &mut blocks);
            cur_type = None;
            let label = caps
                .get(1)
                .map(|m| m.as_str().trim().to_owned())
                .unwrap_or_else(|| "Details".to_owned());
            blocks.push(Block::HiddenOpen(label));
        } else if HIDDEN_CLOSE_RE.is_match(s) {
            flush(&cur_type, &mut cur, &mut blocks);
            cur_type = None;
            blocks.push(Block::HiddenClose);
        } else if s.starts_with("-- ") || s == "--" {
            if cur_type == Some(BlockType::Code) {
                flush(&cur_type, &mut cur, &mut blocks);
            }
            cur_type = Some(BlockType::Prose);
            let prose_line = if s.starts_with("-- ") {
                s[3..].to_owned()
            } else {
                String::new()
            };
            cur.push(prose_line);
        } else if s.is_empty() {
            match cur_type {
                Some(BlockType::Prose) | Some(BlockType::Code) => cur.push(String::new()),
                None => {}
            }
        } else {
            if cur_type == Some(BlockType::Prose) {
                flush(&cur_type, &mut cur, &mut blocks);
            }
            cur_type = Some(BlockType::Code);
            cur.push((*line).to_owned());
        }
    }

    flush(&cur_type, &mut cur, &mut blocks);
    blocks
}
