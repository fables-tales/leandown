use crate::build::{build, find_local_tailwind};
use anyhow::{Context, Result};
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub fn serve(root: &Path, output: &Path, port: u16) -> Result<()> {
    // Initial build.
    println!("Building...");
    build(root, output)?;

    let root_display = root.display().to_string();
    let root_buf = root.to_path_buf();
    let output_watcher = output.to_path_buf();
    let output_server = output.to_path_buf();

    // Spawn tailwind watch if the project has a frontend bundle.
    let mut tailwind_child: Option<Child> = output
        .parent()
        .and_then(|site_dir| find_local_tailwind(site_dir).map(|tw| (tw, site_dir.to_path_buf())))
        .and_then(|(tailwind, site_dir)| {
            let css_out = output.join("styles.css");
            match Command::new(&tailwind)
                .args([
                    "-i", "src/input.css",
                    "-o", css_out.to_str().unwrap_or("output/styles.css"),
                    "--watch",
                ])
                .current_dir(&site_dir)
                .spawn()
            {
                Ok(child) => {
                    println!("  tailwind watch started (pid {})", child.id());
                    Some(child)
                }
                Err(e) => {
                    eprintln!("  WARN: failed to start tailwind watch: {e}");
                    None
                }
            }
        });

    // Channel for file-change events from the watcher.
    let (tx, rx) = mpsc::channel::<notify::Result<Event>>();

    let mut watcher = RecommendedWatcher::new(tx, Config::default())
        .context("creating file watcher")?;
    watcher
        .watch(&root_buf, RecursiveMode::Recursive)
        .context("watching root directory")?;

    // Rebuilder thread: debounces .lean changes and calls build().
    std::thread::spawn(move || {
        let mut pending = false;
        let mut last_change = Instant::now();
        let debounce = Duration::from_millis(300);

        loop {
            match rx.recv_timeout(debounce / 2) {
                Ok(Ok(event)) => {
                    let is_lean = event
                        .paths
                        .iter()
                        .any(|p| p.extension().map_or(false, |e| e == "lean"));
                    let is_output = event.paths.iter().any(|p| p.starts_with(&output_watcher));
                    if is_lean && !is_output {
                        pending = true;
                        last_change = Instant::now();
                    }
                }
                Ok(Err(e)) => eprintln!("watch error: {e}"),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }

            if pending && last_change.elapsed() >= debounce {
                pending = false;
                println!("\nFile changed, rebuilding...");
                if let Err(e) = build(&root_buf, &output_watcher) {
                    eprintln!("Build error: {e}");
                }
            }
        }
    });

    // HTTP server on main thread.
    let addr = format!("0.0.0.0:{port}");
    let server = tiny_http::Server::http(&addr)
        .map_err(|e| anyhow::anyhow!("failed to start server on {addr}: {e}"))?;

    println!("\nServing at http://localhost:{port}");
    println!("Watching {root_display} for .lean changes. Ctrl-C to stop.\n");

    for request in server.incoming_requests() {
        let url = request.url().to_owned();
        let path_str = url.split('?').next().unwrap_or("/");
        let file_path = resolve_path(&output_server, path_str);
        serve_file(request, &file_path);
    }

    // Kill tailwind watch when the server exits.
    if let Some(ref mut child) = tailwind_child {
        let _ = child.kill();
    }

    Ok(())
}

fn resolve_path(output: &Path, url_path: &str) -> PathBuf {
    let rel = url_path.trim_start_matches('/');
    if rel.is_empty() {
        output.join("index.html")
    } else {
        let candidate = output.join(rel);
        if candidate.is_dir() {
            candidate.join("index.html")
        } else {
            candidate
        }
    }
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("css")  => "text/css",
        Some("js")   => "application/javascript",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("svg")  => "image/svg+xml",
        Some("png")  => "image/png",
        Some("ico")  => "image/x-icon",
        _            => "application/octet-stream",
    }
}

fn serve_file(request: tiny_http::Request, path: &Path) {
    match std::fs::File::open(path) {
        Ok(mut file) => {
            let mut buf = Vec::new();
            if file.read_to_end(&mut buf).is_ok() {
                let ct = content_type(path);
                let response = tiny_http::Response::from_data(buf).with_header(
                    tiny_http::Header::from_bytes("Content-Type", ct).unwrap(),
                );
                let _ = request.respond(response);
            } else {
                let _ = request.respond(
                    tiny_http::Response::from_string("read error").with_status_code(500),
                );
            }
        }
        Err(_) => {
            let body = format!("404 Not Found: {}", path.display());
            let _ = request.respond(
                tiny_http::Response::from_string(body).with_status_code(404),
            );
        }
    }
}
