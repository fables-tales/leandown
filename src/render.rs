use crate::parse::{Block, Page, page_file};
use crate::xref::{XrefData, XrefEntry, grouped};
use html_escape::encode_text;
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::{HashMap, HashSet};

static LABEL_REF_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\[ref:([^\]]+)\]").unwrap());

const KATEX_CSS: &str =
    "https://cdn.jsdelivr.net/npm/katex@0.16.11/dist/katex.min.css";
const KATEX_JS: &str =
    "https://cdn.jsdelivr.net/npm/katex@0.16.11/dist/katex.min.js";
const KATEX_AR: &str =
    "https://cdn.jsdelivr.net/npm/katex@0.16.11/dist/contrib/auto-render.min.js";
const MARKED_JS: &str =
    "https://cdn.jsdelivr.net/npm/marked/marked.min.js";

const JS: &str = r#"
document.querySelectorAll('.prose').forEach(el => {
  const stash = [];
  const save = m => { stash.push(m); return '\u2060' + stash.length + '\u2060'; };
  let text = el.textContent;
  text = text.replace(/\$\$[\s\S]*?\$\$/g, save);
  text = text.replace(/\$[^$\n]+?\$/g,      save);
  let rendered = marked.parse(text);
  rendered = rendered.replace(/\u2060(\d+)\u2060/g, (_, i) => stash[+i - 1]);
  el.innerHTML = rendered;
});
if (typeof renderMathInElement !== 'undefined') {
  renderMathInElement(document.body, {
    delimiters: [
      { left: '$$', right: '$$', display: true  },
      { left: '$',  right: '$',  display: false }
    ],
    throwOnError: false
  });
}
"#;

static LOWERCASE_WORDS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    [
        "a", "an", "the", "and", "but", "or", "nor", "for", "so", "yet", "in", "on", "at", "to",
        "of", "by", "up", "as", "is", "it",
    ]
    .into()
});

pub fn title_case(s: &str) -> String {
    let words: Vec<&str> = s.split_whitespace().collect();
    let n = words.len();
    words
        .iter()
        .enumerate()
        .map(|(i, w)| {
            if i == 0 || i == n - 1 || !LOWERCASE_WORDS.contains(w.to_lowercase().as_str()) {
                let mut c = w.chars();
                match c.next() {
                    None => String::new(),
                    Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                }
            } else {
                w.to_lowercase()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn head() -> String {
    format!(
        r#"<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<link rel="stylesheet" href="styles.css">
<link rel="stylesheet" href="{KATEX_CSS}">
<script src="{KATEX_JS}"></script>
<script src="{KATEX_AR}"></script>
<script src="{MARKED_JS}"></script>
"#
    )
}

fn page_layout(title: &str, sidebar_html: &str, main_html: &str, extra_body: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<title>{title}</title>
{head}
</head>
<body class="text-gray-900">
<aside class="fixed left-0 top-0 h-screen w-56 overflow-y-auto bg-white border-r border-gray-200 z-10">
{sidebar_html}
</aside>
<main class="ml-56 max-w-3xl px-10 py-10">
{main_html}
</main>
{extra_body}
</body>
</html>
"#,
        head = head(),
    )
}

pub fn render_sidebar(pages: &[Page], current_module: Option<&str>) -> String {
    let groups = grouped(pages);
    let mut parts = vec![
        r#"<nav class="font-sans text-sm px-4 py-6 space-y-2">"#.to_owned(),
        r#"  <a href="index.html" class="block text-blue-600 hover:underline text-xs mb-5">← index</a>"#.to_owned(),
    ];

    for (group_name, indices) in &groups {
        let is_current = current_module
            .map(|m| indices.iter().any(|&i| pages[i].module == m))
            .unwrap_or(false);
        let open_attr = if is_current { " open" } else { "" };

        parts.push(format!("  <details{open_attr}>"));
        parts.push(format!(
            r#"    <summary class="cursor-pointer select-none bg-gray-100 hover:bg-gray-200 rounded px-3 py-2 font-bold text-gray-800 uppercase tracking-wider text-xs">{}</summary>"#,
            encode_text(group_name)
        ));
        parts.push(r#"    <ul class="mt-1 space-y-0.5">"#.to_owned());

        for &idx in indices {
            let p = &pages[idx];
            let href = page_file(&p.module);
            let t = title_case(&p.title);
            let title_esc = encode_text(&t);
            if current_module == Some(&p.module) {
                parts.push(format!(
                    r#"      <li><a href="{href}" class="block px-3 py-1.5 rounded bg-blue-50 text-blue-700 font-semibold text-xs">{title_esc}</a></li>"#
                ));
            } else {
                parts.push(format!(
                    r#"      <li><a href="{href}" class="block px-3 py-1.5 rounded text-gray-600 hover:bg-gray-100 hover:text-gray-900 text-xs">{title_esc}</a></li>"#
                ));
            }
        }

        parts.push("    </ul>".to_owned());
        parts.push("  </details>".to_owned());
    }

    parts.push("</nav>".to_owned());
    parts.join("\n")
}

/// Insert markdown line breaks where visual breaks are wanted:
/// after a line ending in ':', before a line starting with 'and ', 'so ', or '$='.
fn format_prose(text: &str) -> String {
    static BREAK_NEXT_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"^(and |so |\$=)").unwrap());

    let lines: Vec<&str> = text.lines().collect();
    let mut result = Vec::with_capacity(lines.len());

    for (i, &line) in lines.iter().enumerate() {
        let cur = line.trim_end();
        let next = lines.get(i + 1).map(|l| l.trim_start()).unwrap_or("");
        let break_after =
            cur.trim_start().ends_with(':') || BREAK_NEXT_RE.is_match(next);
        if break_after && !cur.is_empty() {
            result.push(format!("{}  ", cur));
        } else {
            result.push(line.to_owned());
        }
    }

    result.join("\n")
}

/// Substitute `decl_name` backtick references with markdown links.
fn link_xrefs(text: &str, xref: &HashMap<String, XrefEntry>) -> String {
    // Build a regex matching any known declaration name in backticks.
    if xref.is_empty() {
        return text.to_owned();
    }

    // Use regex on the pattern "`name`" rather than the static XREF_RE, because
    // xref names are only known at runtime. Build a combined regex.
    let mut names: Vec<String> = xref.keys().map(|k| regex::escape(k)).collect();
    names.sort_by_key(|s| std::cmp::Reverse(s.len()));
    let pattern = format!(r"`({})`", names.join("|"));

    let re = match Regex::new(&pattern) {
        Ok(r) => r,
        Err(_) => return text.to_owned(),
    };

    re.replace_all(text, |caps: &regex::Captures| {
        let name = &caps[1];
        if let Some(entry) = xref.get(name) {
            format!(
                "[`{}`]({} \"{}, {}\")",
                name, entry.href, entry.page_title, entry.label
            )
        } else {
            caps[0].to_owned()
        }
    })
    .into_owned()
}

fn link_label_refs(text: &str, label_xref: &HashMap<String, String>) -> String {
    LABEL_REF_RE
        .replace_all(text, |caps: &regex::Captures| {
            let label = caps[1].trim();
            if let Some(href) = label_xref.get(label) {
                format!("[{label}]({href})")
            } else {
                caps[0].to_owned()
            }
        })
        .into_owned()
}

fn render_code_block(
    text: &str,
    ld_mods: &HashSet<String>,
    xref: &HashMap<String, XrefEntry>,
) -> String {
    // Build a combined xref regex for identifier names that appear in code.
    let xref_re: Option<Regex> = if xref.is_empty() {
        None
    } else {
        let mut names: Vec<String> = xref.keys().map(|k| regex::escape(k)).collect();
        names.sort_by_key(|s| std::cmp::Reverse(s.len()));
        let pattern = format!(
            r"(?<![A-Za-z0-9_.])({})(?![A-Za-z0-9_.])",
            names.join("|")
        );
        Regex::new(&pattern).ok()
    };

    let mut lines_out: Vec<String> = Vec::new();

    for line in text.lines() {
        // Check if this is an import line whose module is in our leandown pages.
        if let Some(caps) = Regex::new(r"^(import\s+)(\S+)(.*)").unwrap().captures(line) {
            let prefix = &caps[1];
            let modname = &caps[2];
            let suffix = &caps[3];
            if ld_mods.contains(modname) {
                let href = page_file(modname);
                lines_out.push(format!(
                    r#"{}<a href="{href}">{}</a>{}"#,
                    encode_text(prefix),
                    encode_text(modname),
                    encode_text(suffix),
                ));
                continue;
            }
        }

        // HTML-escape first, then apply xref links.
        let escaped = encode_text(line).into_owned();
        if let Some(ref re) = xref_re {
            let linked = re.replace_all(&escaped, |caps: &regex::Captures| {
                let name = &caps[1];
                // name is already HTML-escaped (it's an identifier, so no special chars)
                if let Some(entry) = xref.get(name) {
                    format!(
                        r#"<a href="{}" title="{}, {}" class="xref">{}</a>"#,
                        entry.href,
                        encode_text(&entry.page_title),
                        entry.label,
                        name,
                    )
                } else {
                    caps[0].to_owned()
                }
            });
            lines_out.push(linked.into_owned());
        } else {
            lines_out.push(escaped);
        }
    }

    format!(
        r#"<pre class="bg-gray-50 border-l-4 border-gray-200 px-4 py-3 overflow-x-auto text-sm leading-relaxed my-1 mb-5 rounded-r"><code>{}</code></pre>"#,
        lines_out.join("\n")
    )
}

/// Render a single page to an HTML string.
pub fn render_page(
    page: &Page,
    pages: &[Page],
    ld_mods: &HashSet<String>,
    xref_data: &XrefData,
    backlinks: &HashMap<String, Vec<usize>>,
) -> String {
    let title = title_case(&page.title);
    let title_esc = encode_text(&title);
    let anchors = xref_data.anchors.get(&page.module).cloned().unwrap_or_default();

    let mut content_parts = vec![format!(
        r#"<h1 class="text-2xl font-bold font-sans mb-6">{title_esc}</h1>"#
    )];

    for (i, block) in page.blocks.iter().enumerate() {
        match block {
            Block::HiddenOpen(label) => {
                content_parts.push(format!(
                    r#"<details class="my-4 border border-gray-200 rounded">
  <summary class="cursor-pointer select-none px-4 py-2 text-sm text-gray-500 hover:text-gray-700 hover:bg-gray-50">{} ▸</summary>
  <div class="px-4 pb-3">"#,
                    encode_text(label)
                ));
            }
            Block::HiddenClose => {
                content_parts.push("  </div>\n</details>".to_owned());
            }
            Block::Prose(text) => {
                let linked = link_xrefs(text, &xref_data.xref);
                let linked = link_label_refs(&linked, &xref_data.label_xref);
                let formatted = format_prose(&linked);
                let escaped = encode_text(&formatted);
                let anchor_html = if let Some(aid) = anchors.get(&i) {
                    format!(r#"<a id="{aid}"></a>"#) + "\n"
                } else {
                    String::new()
                };
                content_parts.push(format!(
                    r#"{anchor_html}<div class="prose prose-gray prose-sm max-w-none my-3">{escaped}</div>"#
                ));
            }
            Block::Code(text) => {
                content_parts.push(render_code_block(text, ld_mods, &xref_data.xref));
            }
        }
    }

    // Backlinks section.
    let back_indices = backlinks.get(&page.module).cloned().unwrap_or_default();
    let backlink_section = if !back_indices.is_empty() {
        let mut sorted_back: Vec<&Page> = back_indices.iter().map(|&i| &pages[i]).collect();
        sorted_back.sort_by_key(|p| &p.title);
        let items: String = sorted_back
            .iter()
            .map(|b| {
                let href = page_file(&b.module);
                let t = title_case(&b.title);
                format!(
                    r#"<li><a href="{href}" class="text-blue-600 hover:underline">{}</a></li>"#,
                    encode_text(&t)
                )
            })
            .collect();
        format!(
            r#"<div class="mt-12 border-t border-gray-200 pt-4 font-sans">
  <p class="text-xs text-gray-400 uppercase tracking-wider mb-2">Referenced by</p>
  <ul class="space-y-1 text-sm">{items}</ul>
</div>
"#
        )
    } else {
        String::new()
    };

    // Raw source section.
    let raw_section = format!(
        r#"<details class="mt-6 border-t border-gray-200 pt-4 font-sans">
  <summary class="cursor-pointer text-gray-400 text-sm">Raw source</summary>
  <pre class="bg-gray-50 px-4 py-3 overflow-x-auto text-xs leading-relaxed mt-3 rounded"><code>{}</code></pre>
</details>"#,
        encode_text(&page.raw)
    );

    let main_html = content_parts.join("\n") + "\n" + &backlink_section + &raw_section;
    let sidebar_html = render_sidebar(pages, Some(&page.module));

    page_layout(
        &title_esc,
        &sidebar_html,
        &main_html,
        &format!("<script>{JS}</script>"),
    )
}

/// Render the index page listing all pages.
pub fn render_index(pages: &[Page]) -> String {
    let sidebar_html = render_sidebar(pages, None);
    let main_html = r#"<h1 class="text-2xl font-bold font-sans mb-4">leandown</h1>
<div class="prose prose-gray prose-sm max-w-none mb-8">
<p>leandown is a literate Lean 4 publishing tool. Each page is a Lean source file
annotated with structured comments that render as prose, with LaTeX math via KaTeX
and clickable cross-references between files. Only files that build cleanly with
no warnings are included.</p>
</div>"#;

    page_layout("leandown", &sidebar_html, main_html, "")
}
