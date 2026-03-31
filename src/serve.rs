use crate::build::build;
use anyhow::{Context, Result};
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub fn serve(root: &Path, output: &Path, port: u16) -> Result<()> {
    // Initial build.
    println!("Building...");
    build(root, output)?;

    let root = root.to_path_buf();
    let root_display = root.display().to_string();
    let output_watcher = output.to_path_buf();
    let output_server = output.to_path_buf();

    // Channel for file-change events from the watcher.
    let (tx, rx) = mpsc::channel::<notify::Result<Event>>();

    // Spawn file watcher thread.
    let mut watcher = RecommendedWatcher::new(tx, Config::default())
        .context("creating file watcher")?;
    watcher
        .watch(&root, RecursiveMode::Recursive)
        .context("watching root directory")?;

    // Spawn rebuilder thread: debounces events and calls build().
    std::thread::spawn(move || {
        let mut pending = false;
        let mut last_change = Instant::now();
        let debounce = Duration::from_millis(300);

        loop {
            // Drain all available events with a short timeout.
            match rx.recv_timeout(debounce / 2) {
                Ok(Ok(event)) => {
                    let is_lean = event
                        .paths
                        .iter()
                        .any(|p| p.extension().map_or(false, |e| e == "lean"));
                    // Skip changes inside output to avoid build loops.
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
                if let Err(e) = build(&root, &output_watcher) {
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
    println!("Watching {root_display} for changes. Ctrl-C to stop.\n");

    for request in server.incoming_requests() {
        let url = request.url().to_owned();
        // Strip query string.
        let path_str = url.split('?').next().unwrap_or("/");
        // Resolve to a file in the output directory.
        let file_path = resolve_path(&output_server, path_str);
        serve_file(request, &file_path);
    }

    Ok(())
}

/// Map a URL path to a file in the output directory.
/// Falls back to index.html for directory requests.
fn resolve_path(output: &Path, url_path: &str) -> PathBuf {
    // Strip leading slash and percent-decode basic cases.
    let rel = url_path.trim_start_matches('/');
    let file = if rel.is_empty() {
        output.join("index.html")
    } else {
        let candidate = output.join(rel);
        if candidate.is_dir() {
            candidate.join("index.html")
        } else {
            candidate
        }
    };
    file
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css",
        Some("js") => "application/javascript",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        _ => "application/octet-stream",
    }
}

fn serve_file(request: tiny_http::Request, path: &Path) {
    match std::fs::File::open(path) {
        Ok(mut file) => {
            let mut buf = Vec::new();
            if file.read_to_end(&mut buf).is_ok() {
                let ct = content_type(path);
                let response = tiny_http::Response::from_data(buf)
                    .with_header(
                        tiny_http::Header::from_bytes("Content-Type", ct).unwrap(),
                    );
                let _ = request.respond(response);
            } else {
                let _ = request.respond(tiny_http::Response::from_string("read error").with_status_code(500));
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
