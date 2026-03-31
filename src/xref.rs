use crate::parse::{Block, Page, page_file};
use indexmap::IndexMap;
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::{HashMap, HashSet};

static TAG_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\{\{(\w+)\}\}").unwrap());
static DECL_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?m)^\s*(?:private\s+)?(?:noncomputable\s+)?(?:theorem|lemma|def)\s+(\S+)",
    )
    .unwrap()
});

/// Cross-reference entry for a tagged declaration.
#[derive(Debug, Clone)]
pub struct XrefEntry {
    pub href: String,       // e.g. "Foo.html#theorem-1"
    pub page_title: String,
    pub label: String,      // e.g. "Theorem 1"
}

/// Per-block anchor IDs assigned during tag processing.
/// Maps block index → anchor id string.
pub type AnchorMap = HashMap<usize, String>;

pub struct XrefData {
    /// decl_name → XrefEntry
    pub xref: HashMap<String, XrefEntry>,
    /// "Theorem 1" → "Foo.html#theorem-1"
    pub label_xref: HashMap<String, String>,
    /// module → AnchorMap
    pub anchors: HashMap<String, AnchorMap>,
}

/// Extract the first theorem/lemma/def name from a code block.
pub fn extract_decl_name(code: &str) -> Option<String> {
    DECL_RE
        .captures(code)
        .map(|c| c[1].trim_end_matches(':').to_owned())
}

fn anchor_id(label: &str) -> String {
    label.to_lowercase().replace(' ', "-")
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

/// Process {{tag}} placeholders in all pages, numbering them per-page per-type.
/// Mutates blocks in place (replaces Prose text with expanded labels).
/// Returns xref maps and per-page anchor assignments.
pub fn process_tags_and_xref(pages: &mut Vec<Page>) -> XrefData {
    let mut xref: HashMap<String, XrefEntry> = HashMap::new();
    let mut label_xref: HashMap<String, String> = HashMap::new();
    let mut anchors: HashMap<String, AnchorMap> = HashMap::new();

    for page in pages.iter_mut() {
        let mut counters: HashMap<String, u32> = HashMap::new();
        let href = page_file(&page.module);
        let page_title = page.title.clone();
        let mut page_anchors: AnchorMap = HashMap::new();

        let block_count = page.blocks.len();

        // We need to mutate blocks[i] while reading blocks[i+1..].
        // Collect which prose blocks contain tags first, process them.
        for i in 0..block_count {
            let prose_text = match &page.blocks[i] {
                Block::Prose(t) => t.clone(),
                _ => continue,
            };

            if !TAG_RE.is_match(&prose_text) {
                continue;
            }

            // Manually iterate matches to allow mutable counter access.
            let mut result = String::new();
            let mut last_end = 0;
            for cap in TAG_RE.captures_iter(&prose_text) {
                let m = cap.get(0).unwrap();
                result.push_str(&prose_text[last_end..m.start()]);

                let tag = cap[1].to_lowercase();
                let count = counters.entry(tag.clone()).or_insert(0);
                *count += 1;
                let label = format!("{} {}", capitalize(&tag), count);
                let aid = anchor_id(&label);
                let full_href = format!("{}#{}", href, aid);

                page_anchors.insert(i, aid);
                label_xref.insert(label.clone(), full_href.clone());

                // Find the next code block's declaration to register in xref.
                for j in (i + 1)..block_count {
                    if let Block::Code(code) = &page.blocks[j] {
                        if let Some(decl) = extract_decl_name(code) {
                            xref.insert(
                                decl,
                                XrefEntry {
                                    href: full_href.clone(),
                                    page_title: page_title.clone(),
                                    label: label.clone(),
                                },
                            );
                        }
                        break;
                    }
                }

                result.push_str(&label);
                last_end = m.end();
            }
            result.push_str(&prose_text[last_end..]);

            page.blocks[i] = Block::Prose(result);
        }

        anchors.insert(page.module.clone(), page_anchors);
    }

    XrefData {
        xref,
        label_xref,
        anchors,
    }
}

/// Returns a map: module → list of pages that import it.
pub fn compute_backlinks(pages: &[Page]) -> HashMap<String, Vec<usize>> {
    let mod_index: HashMap<&str, usize> = pages
        .iter()
        .enumerate()
        .map(|(i, p)| (p.module.as_str(), i))
        .collect();

    let mut backlinks: HashMap<String, Vec<usize>> = pages
        .iter()
        .map(|p| (p.module.clone(), vec![]))
        .collect();

    for (i, page) in pages.iter().enumerate() {
        for imp in &page.imports {
            if mod_index.contains_key(imp.as_str()) {
                backlinks.entry(imp.clone()).or_default().push(i);
            }
        }
    }

    backlinks
}

/// Return page indices in topological order (leaves first) by import dependency.
pub fn topo_sort(pages: &[Page]) -> Vec<usize> {
    let ld_mods: HashSet<&str> = pages.iter().map(|p| p.module.as_str()).collect();
    let mod_index: HashMap<&str, usize> = pages
        .iter()
        .enumerate()
        .map(|(i, p)| (p.module.as_str(), i))
        .collect();

    let mut order: Vec<usize> = Vec::with_capacity(pages.len());
    let mut visited: HashSet<usize> = HashSet::new();
    let mut in_stack: HashSet<usize> = HashSet::new();

    // Iterative DFS using an explicit stack.
    // Each stack entry is (page_index, iterator_over_deps).
    for start in 0..pages.len() {
        if visited.contains(&start) {
            continue;
        }

        let mut stack: Vec<(usize, Vec<usize>)> = Vec::new();
        let deps_of = |idx: usize| -> Vec<usize> {
            let mut deps: Vec<usize> = pages[idx]
                .imports
                .iter()
                .filter(|m| ld_mods.contains(m.as_str()))
                .filter_map(|m| mod_index.get(m.as_str()).copied())
                .collect();
            deps.sort();
            deps
        };

        stack.push((start, deps_of(start)));
        in_stack.insert(start);

        while let Some((idx, ref mut deps)) = stack.last_mut() {
            let idx = *idx;
            if let Some(dep) = deps.pop() {
                if !visited.contains(&dep) && !in_stack.contains(&dep) {
                    in_stack.insert(dep);
                    let dep_deps = deps_of(dep);
                    stack.push((dep, dep_deps));
                }
            } else {
                visited.insert(idx);
                in_stack.remove(&idx);
                order.push(idx);
                stack.pop();
            }
        }
    }

    order
}

/// Return pages grouped by their `group` meta field, in topological order within each group,
/// with groups sorted alphabetically. Returns an IndexMap to preserve group order.
pub fn grouped(pages: &[Page]) -> IndexMap<String, Vec<usize>> {
    let topo = topo_sort(pages);
    let mut groups: IndexMap<String, Vec<usize>> = IndexMap::new();

    for idx in topo {
        let g = pages[idx].group.clone();
        groups.entry(g).or_default().push(idx);
    }

    // Sort groups alphabetically.
    groups.sort_keys();
    groups
}
