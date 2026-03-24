#!/usr/bin/env python3
"""
leandown — convert annotated Lean 4 files to a static website.

Usage:
    python build.py --root /path/to/lean/project [--output /path/to/output]

Output:
    <output>/index.html      file listing
    <output>/<Name>.html     one page per annotated file
    <output>/styles.css      built by Tailwind

    --root defaults to the current working directory.
    --output defaults to <root>/leandown_site/output.

Annotating a Lean file
──────────────────────
Add a leandown header anywhere near the top of a .lean file:

    -- leandown
    -- [meta]
    -- title = "Page title"
    -- [content]

Everything after -- [content] is processed as follows:
  Lines starting with  --   are rendered as markdown + LaTeX prose.
  All other lines are rendered as Lean code blocks (<pre>).
  Local imports become clickable links.

In prose blocks:
  {{theorem}}, {{bijection}}, etc.  → numbered label + anchor (e.g. "Theorem 1")
  `decl_name`                       → link to that declaration's label
  [ref:Bijection 1]                 → link to that label's anchor (cross-page safe)
"""

import os, sys, re, html, glob, subprocess, shutil, argparse

try:
    import tomllib
except ImportError:
    try:
        import tomli as tomllib          # pip install tomli  (Python < 3.11)
    except ImportError:
        print("Requires Python 3.11+, or: pip install tomli")
        sys.exit(1)

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))


# ── parsing ──────────────────────────────────────────────────────────────────

def module_of(path, root_dir):
    """Onewayf/Foo.lean  →  Onewayf.Foo"""
    return os.path.relpath(path, root_dir).replace(os.sep, ".").removesuffix(".lean")

def page_file(module):
    """Onewayf.Foo  →  Foo.html"""
    return module.split(".")[-1] + ".html"

def parse(path, root_dir):
    """Parse a leandown-annotated file. Returns None if not annotated."""
    raw   = open(path).read()
    lines = raw.splitlines()

    ld = next((i for i, l in enumerate(lines) if l.strip() == "-- leandown"), None)
    if ld is None:
        return None

    imports = []
    for l in lines[:ld]:
        m = re.match(r"^import\s+(\S+)", l)
        if m:
            imports.append(m.group(1))

    meta_lines, in_meta, content_start = [], False, None
    for i in range(ld + 1, len(lines)):
        s = lines[i].strip()
        if   s == "-- [meta]":    in_meta = True
        elif s == "-- [content]": content_start = i + 1; break
        elif in_meta:
            meta_lines.append(s[3:] if s.startswith("-- ") else ("" if s == "--" else s))

    meta = {}
    if meta_lines:
        try:    meta = tomllib.loads("\n".join(meta_lines))
        except Exception as e: print(f"TOML warning ({path}): {e}")

    if content_start is None:
        return None

    blocks, cur_type, cur = [], None, []

    def flush():
        nonlocal cur_type, cur
        while cur and not cur[-1].strip():
            cur.pop()
        if cur:
            blocks.append((cur_type, "\n".join(cur)))
        cur_type, cur = None, []

    _HIDDEN_OPEN_RE  = re.compile(r'^-- \[hidden(?::\s*(.+))?\]$')
    _HIDDEN_CLOSE_RE = re.compile(r'^-- \[/hidden\]$')

    for line in lines[content_start:]:
        s = line.strip()
        m_open  = _HIDDEN_OPEN_RE.match(s)
        m_close = _HIDDEN_CLOSE_RE.match(s)
        if m_open:
            flush()
            label = (m_open.group(1) or "Details").strip()
            blocks.append(("hidden_open", label))
        elif m_close:
            flush()
            blocks.append(("hidden_close", ""))
        elif s.startswith("-- ") or s == "--":
            if cur_type == "code": flush()
            cur_type = "prose"
            cur.append(s[3:] if s.startswith("-- ") else "")
        elif not s:
            if   cur_type == "prose": cur.append("")
            elif cur_type == "code":  cur.append("")
        else:
            if cur_type == "prose": flush()
            cur_type = "code"
            cur.append(line)

    flush()

    return dict(path=path, module=module_of(path, root_dir), meta=meta,
                imports=imports, blocks=blocks, raw=raw)


# ── validation ───────────────────────────────────────────────────────────────

def validate(p, root_dir):
    """Return True only if `lake build <module>` succeeds with zero warnings."""
    if not shutil.which("lake"):
        print("  WARN: lake not found, skipping validation")
        return True

    rel = os.path.relpath(p["path"], root_dir)
    result = subprocess.run(
        ["lake", "build", p["module"]],
        cwd=root_dir,
        capture_output=True, text=True
    )

    output = result.stdout + result.stderr
    # Warnings are lines like "Onewayf/Foo.lean:12:3: warning: ..."
    warnings = [l.strip() for l in output.splitlines()
                if "warning:" in l and rel in l]

    if result.returncode != 0:
        errors = [l.strip() for l in output.splitlines() if "error:" in l and rel in l]
        print(f"  SKIP {p['module']}: build errors")
        for e in errors:
            print(f"       {e}")
        return False

    if warnings:
        print(f"  SKIP {p['module']}: {len(warnings)} warning(s)")
        for w in warnings:
            print(f"       {w}")
        return False

    return True


# ── tag processing and xref ──────────────────────────────────────────────────

_TAG_RE  = re.compile(r'\{\{(\w+)\}\}')
_DECL_RE = re.compile(
    r'^\s*(?:private\s+)?(?:noncomputable\s+)?(?:theorem|lemma|def)\s+(\S+)',
    re.MULTILINE,
)
_XREF_RE = re.compile(r'`([A-Za-z_][A-Za-z0-9_.]*)`')


def extract_decl_name(code_text):
    """Return the first theorem/lemma/def name found in a code block."""
    m = _DECL_RE.search(code_text)
    return m.group(1) if m else None


def _anchor_id(label):
    """'Theorem 1' → 'theorem-1'"""
    return label.lower().replace(" ", "-")


def process_tags_and_xref(pages):
    """
    Replace {{tag}} placeholders in prose blocks with numbered labels
    (e.g. "Theorem 1", "Bijection 2"), mutating each page's blocks in place.

    Returns:
      xref       — maps declaration names to (href_with_anchor, page_title, label)
      label_xref — maps label strings like "Bijection 1" to href_with_anchor

    Also annotates each page with an "anchors" dict {block_index: anchor_id}.
    """
    xref       = {}
    label_xref = {}

    for p in pages:
        counters   = {}
        blocks     = p["blocks"]
        page_title = title_case(p["meta"].get("title", p["module"]))
        href       = page_file(p["module"])
        p["anchors"] = {}

        for i, (btype, text) in enumerate(blocks):
            if btype != "prose":
                continue

            def replace_tag(m, _i=i):
                tag   = m.group(1).lower()
                counters[tag] = counters.get(tag, 0) + 1
                label = f"{tag.capitalize()} {counters[tag]}"
                aid   = _anchor_id(label)
                full_href = f"{href}#{aid}"

                p["anchors"][_i] = aid
                label_xref[label] = full_href

                # Associate the label with the next code block's declaration.
                for j in range(_i + 1, len(blocks)):
                    if blocks[j][0] == "code":
                        decl = extract_decl_name(blocks[j][1])
                        if decl:
                            xref[decl] = (full_href, page_title, label)
                        break

                return label

            new_text  = _TAG_RE.sub(replace_tag, text)
            blocks[i] = (btype, new_text)

    return xref, label_xref


def compute_backlinks(pages):
    """Returns dict: module → [page, ...] that import it."""
    mod_map    = {p["module"]: p for p in pages}
    backlinks  = {p["module"]: [] for p in pages}
    for p in pages:
        for imp in p["imports"]:
            if imp in mod_map:
                backlinks[imp].append(p)
    return backlinks


def link_xrefs(text, xref):
    """`decl_name` → markdown link if decl_name appears in the xref map."""
    def replace(m):
        name = m.group(1)
        if name in xref:
            href, page_title, label = xref[name]
            return f'[`{name}`]({href} "{page_title}, {label}")'
        return m.group(0)
    return _XREF_RE.sub(replace, text)


_LABEL_REF_RE = re.compile(r'\[ref:([^\]]+)\]')

def link_label_refs(text, label_xref):
    """[ref:Bijection 1] → markdown link to that label's anchor."""
    def replace(m):
        label = m.group(1).strip()
        if label in label_xref:
            return f'[{label}]({label_xref[label]})'
        return m.group(0)
    return _LABEL_REF_RE.sub(replace, text)


# ── rendering ────────────────────────────────────────────────────────────────

JS = """\
// Stash $...$ and $$...$$ before marked.parse so it can't mangle LaTeX backslashes,
// then restore them after. Keeps backtick inline-code working with any marked version.
document.querySelectorAll('.prose').forEach(el => {
  const stash = [];
  // \u2060 = word-joiner: invisible, zero-width, not markdown-significant
  const save = m => { stash.push(m); return '\\u2060' + stash.length + '\\u2060'; };
  let text = el.textContent;
  text = text.replace(/\\$\\$[\\s\\S]*?\\$\\$/g, save);
  text = text.replace(/\\$[^$\\n]+?\\$/g,        save);
  let rendered = marked.parse(text);
  rendered = rendered.replace(/\\u2060(\\d+)\\u2060/g, (_, i) => stash[+i - 1]);
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
"""

KATEX_CSS = "https://cdn.jsdelivr.net/npm/katex@0.16.11/dist/katex.min.css"
KATEX_JS  = "https://cdn.jsdelivr.net/npm/katex@0.16.11/dist/katex.min.js"
KATEX_AR  = "https://cdn.jsdelivr.net/npm/katex@0.16.11/dist/contrib/auto-render.min.js"
MARKED_JS = "https://cdn.jsdelivr.net/npm/marked/marked.min.js"

HEAD = """\
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<link rel="stylesheet" href="styles.css">
<link rel="stylesheet" href="{katex_css}">
<script src="{katex_js}"></script>
<script src="{katex_ar}"></script>
<script src="{marked_js}"></script>
""".format(katex_css=KATEX_CSS, katex_js=KATEX_JS, katex_ar=KATEX_AR, marked_js=MARKED_JS)


_LOWERCASE_WORDS = {
    'a', 'an', 'the', 'and', 'but', 'or', 'nor', 'for', 'so', 'yet',
    'in', 'on', 'at', 'to', 'of', 'by', 'up', 'as', 'is', 'it',
}

def title_case(s):
    words = s.split()
    result = []
    for i, w in enumerate(words):
        if i == 0 or i == len(words) - 1 or w.lower() not in _LOWERCASE_WORDS:
            result.append(w[0].upper() + w[1:] if w else w)
        else:
            result.append(w.lower())
    return " ".join(result)


def topo_sort(pages):
    """Return pages in topological order by import dependencies (leaves first)."""
    ld_mods = {p["module"] for p in pages}
    mod_map  = {p["module"]: p for p in pages}
    # build adjacency: module → set of leandown modules it imports
    deps = {p["module"]: {i for i in p["imports"] if i in ld_mods}
            for p in pages}
    order, visited = [], set()
    def visit(mod):
        if mod in visited:
            return
        visited.add(mod)
        for dep in sorted(deps.get(mod, [])):   # sorted for determinism
            visit(dep)
        order.append(mod)
    for p in sorted(pages, key=lambda p: p["module"]):
        visit(p["module"])
    return [mod_map[m] for m in order]


def grouped(pages):
    """Return OrderedDict of group_name → [page, ...] in topological order."""
    sorted_pages = topo_sort(pages)
    groups = {}
    for p in sorted_pages:
        g = p["meta"].get("group", "Other")
        groups.setdefault(g, []).append(p)
    # preserve topo order within groups; sort groups alphabetically
    return dict(sorted(groups.items()))


def render_sidebar(pages, current_module=None):
    """Build the sidebar HTML. current_module=None for the index page."""
    g = grouped(pages)
    parts = [
        '<nav class="font-sans text-sm px-4 py-6 space-y-2">',
        '  <a href="index.html" class="block text-blue-600 hover:underline text-xs mb-5">← index</a>',
    ]
    for group_name, group_pages in g.items():
        is_current_group = any(p["module"] == current_module for p in group_pages)
        open_attr = " open" if is_current_group else ""
        parts.append(f'  <details{open_attr}>')
        parts.append(
            f'    <summary class="cursor-pointer select-none bg-gray-100 hover:bg-gray-200 '
            f'rounded px-3 py-2 font-bold text-gray-800 uppercase tracking-wider text-xs">'
            f'{html.escape(group_name)}</summary>'
        )
        parts.append('    <ul class="mt-1 space-y-0.5">')
        for p in group_pages:
            href  = page_file(p["module"])
            t     = title_case(p["meta"].get("title", p["module"]))
            title = html.escape(t)
            if p["module"] == current_module:
                parts.append(
                    f'      <li><a href="{href}" '
                    f'class="block px-3 py-1.5 rounded bg-blue-50 text-blue-700 font-semibold text-xs">'
                    f'{title}</a></li>'
                )
            else:
                parts.append(
                    f'      <li><a href="{href}" '
                    f'class="block px-3 py-1.5 rounded text-gray-600 hover:bg-gray-100 hover:text-gray-900 text-xs">'
                    f'{title}</a></li>'
                )
        parts.append('    </ul>')
        parts.append('  </details>')
    parts.append('</nav>')
    return "\n".join(parts)


def page_layout(title, sidebar_html, main_html, extra_head="", extra_body=""):
    return f"""\
<!DOCTYPE html>
<html lang="en">
<head>
<title>{title}</title>
{HEAD}{extra_head}
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
"""


def format_prose(text):
    """Insert markdown line breaks (two trailing spaces) where visual breaks are wanted:
    - after a line ending in ':'
    - before a line starting with 'and ', 'so ', or '$=' (ignoring leading spaces)
    """
    lines = text.split("\n")
    result = []
    for i, line in enumerate(lines):
        cur       = line.rstrip()
        cur_bare  = cur.lstrip()                                         # strip indent for matching
        next_bare = lines[i + 1].lstrip() if i + 1 < len(lines) else "" # strip indent for matching
        break_after = (
            cur_bare.endswith(":") or
            re.match(r"^(and |so |\$=)", next_bare)
        )
        result.append(cur + "  " if (break_after and cur) else line)
    return "\n".join(result)


def code_block(text, ld_mods, xref=None):
    # Build a single regex that matches any xref name (longest first).
    xref = xref or {}
    xref_names = sorted(xref.keys(), key=len, reverse=True)
    xref_re = (
        re.compile(
            r'(?<![A-Za-z0-9_\.])(' +
            '|'.join(re.escape(n) for n in xref_names) +
            r')(?![A-Za-z0-9_\.])'
        )
        if xref_names else None
    )

    def link_decl(m):
        name = m.group(1)
        href, page_title, label = xref[name]
        return (
            f'<a href="{href}" title="{html.escape(page_title)}, {label}" '
            f'class="xref">{name}</a>'
        )

    out = []
    for line in text.split("\n"):
        # Link any import whose module name appears in our leandown pages.
        imp = re.match(r"^(import\s+)(\S+)(.*)", line)
        if imp and imp.group(2) in ld_mods:
            out.append(
                html.escape(imp.group(1)) +
                f'<a href="{page_file(imp.group(2))}">{html.escape(imp.group(2))}</a>' +
                html.escape(imp.group(3))
            )
        else:
            escaped = html.escape(line)
            if xref_re:
                escaped = xref_re.sub(link_decl, escaped)
            out.append(escaped)
    return (
        '<pre class="bg-gray-50 border-l-4 border-gray-200 px-4 py-3 '
        'overflow-x-auto text-sm leading-relaxed my-1 mb-5 rounded-r">'
        "<code>" + "\n".join(out) + "</code></pre>"
    )


def render_page(p, pages, ld_mods, xref=None, label_xref=None, backlinks=None):
    title = html.escape(title_case(p["meta"].get("title", p["module"])))

    anchors = p.get("anchors", {})
    content_parts = [f'<h1 class="text-2xl font-bold font-sans mb-6">{title}</h1>']
    for i, (t, txt) in enumerate(p["blocks"]):
        if t == "hidden_open":
            content_parts.append(
                f'<details class="my-4 border border-gray-200 rounded">\n'
                f'  <summary class="cursor-pointer select-none px-4 py-2 text-sm '
                f'text-gray-500 hover:text-gray-700 hover:bg-gray-50">'
                f'{html.escape(txt)} ▸</summary>\n'
                f'  <div class="px-4 pb-3">'
            )
        elif t == "hidden_close":
            content_parts.append('  </div>\n</details>')
        elif t == "prose":
            linked = link_xrefs(txt, xref or {})
            linked = link_label_refs(linked, label_xref or {})
            aid    = anchors.get(i)
            anchor_html = f'<a id="{aid}"></a>\n' if aid else ''
            content_parts.append(
                anchor_html +
                f'<div class="prose prose-gray prose-sm max-w-none my-3">'
                f'{html.escape(format_prose(linked))}</div>'
            )
        else:
            content_parts.append(code_block(txt, ld_mods, xref=xref))

    back = (backlinks or {}).get(p["module"], [])
    backlink_section = ""
    if back:
        items = "".join(
            f'<li><a href="{page_file(b["module"])}" class="text-blue-600 hover:underline">'
            f'{html.escape(title_case(b["meta"].get("title", b["module"])))}</a></li>'
            for b in sorted(back, key=lambda b: b["meta"].get("title", b["module"]))
        )
        backlink_section = (
            '<div class="mt-12 border-t border-gray-200 pt-4 font-sans">\n'
            '  <p class="text-xs text-gray-400 uppercase tracking-wider mb-2">Referenced by</p>\n'
            f'  <ul class="space-y-1 text-sm">{items}</ul>\n'
            '</div>\n'
        )

    raw_section = (
        '<details class="mt-6 border-t border-gray-200 pt-4 font-sans">\n'
        '  <summary class="cursor-pointer text-gray-400 text-sm">Raw source</summary>\n'
        f'  <pre class="bg-gray-50 px-4 py-3 overflow-x-auto text-xs leading-relaxed mt-3 rounded"><code>{html.escape(p["raw"])}</code></pre>\n'
        '</details>'
    )

    main_html = "\n".join(content_parts) + "\n" + backlink_section + raw_section
    sidebar_html = render_sidebar(pages, current_module=p["module"])

    return page_layout(
        title=title,
        sidebar_html=sidebar_html,
        main_html=main_html,
        extra_body=f"<script>\n{JS}\n</script>",
    )


def render_index(pages):
    sidebar_html = render_sidebar(pages, current_module=None)
    main_html = """\
<h1 class="text-2xl font-bold font-sans mb-4">leandown</h1>
<div class="prose prose-gray prose-sm max-w-none mb-8">
<p>leandown is a literate Lean 4 publishing tool. Each page is a Lean source file
annotated with structured comments that render as prose, with LaTeX math via KaTeX
and clickable cross-references between files. Only files that build cleanly with
no warnings are included.</p>
</div>
"""
    return page_layout(
        title="leandown",
        sidebar_html=sidebar_html,
        main_html=main_html,
    )


# ── css build ────────────────────────────────────────────────────────────────

def build_css(out_dir):
    """Run the Tailwind CLI to produce output/styles.css."""
    result = subprocess.run(
        ["npx", "tailwindcss",
         "-i", "src/input.css",
         "-o", os.path.join(out_dir, "styles.css"),
         "--minify"],
        cwd=SCRIPT_DIR,
        capture_output=True, text=True
    )
    if result.returncode != 0:
        print(f"Tailwind error:\n{result.stderr}")
    else:
        size = os.path.getsize(os.path.join(out_dir, "styles.css"))
        print(f"  {'css':45s} → styles.css ({size:,} bytes)")


# ── main ─────────────────────────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser(description="leandown: convert annotated Lean 4 files to a static website")
    parser.add_argument("--root", default=os.getcwd(),
                        help="Root directory of the Lean project (default: cwd)")
    parser.add_argument("--output", default=None,
                        help="Output directory for generated HTML (default: <root>/leandown_site/output)")
    args = parser.parse_args()

    root_dir = os.path.abspath(args.root)
    out_dir  = os.path.abspath(args.output) if args.output else os.path.join(root_dir, "leandown_site", "output")

    os.makedirs(out_dir, exist_ok=True)

    lean_files = glob.glob(os.path.join(root_dir, "**", "*.lean"), recursive=True)
    # Exclude files inside the output directory itself
    lean_files = [f for f in lean_files if not f.startswith(out_dir)]

    pages = []
    for f in lean_files:
        p = parse(f, root_dir)
        if p:
            pages.append(p)

    if not pages:
        print("No leandown-annotated files found.")
        return

    pages = [p for p in pages if validate(p, root_dir)]

    if not pages:
        print("No files passed validation.")
        return

    ld_mods = {p["module"] for p in pages}

    xref, label_xref = process_tags_and_xref(pages)
    backlinks        = compute_backlinks(pages)

    for p in pages:
        out = os.path.join(out_dir, page_file(p["module"]))
        open(out, "w").write(render_page(p, pages, ld_mods, xref=xref, label_xref=label_xref, backlinks=backlinks))
        print(f"  {p['module']:45s} → {os.path.relpath(out, root_dir)}")

    idx = os.path.join(out_dir, "index.html")
    open(idx, "w").write(render_index(pages))
    print(f"  {'index':45s} → {os.path.relpath(idx, root_dir)}")

    print()
    build_css(out_dir)
    print(f"\n✓  open {os.path.relpath(idx, root_dir)}")


if __name__ == "__main__":
    main()
