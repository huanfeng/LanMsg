//! 遍历仓库根 `web/`（排除 `web/test/`），生成内嵌资源表。

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

fn mime_of(name: &str) -> &'static str {
    match name.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "txt" => "text/plain; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        _ => "application/octet-stream",
    }
}

fn collect(dir: &Path, rel: &str, out: &mut Vec<(String, PathBuf)>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let rel_path = if rel.is_empty() {
            name.clone()
        } else {
            format!("{rel}/{name}")
        };
        if entry.file_type()?.is_dir() {
            if rel_path == "test" {
                continue;
            }
            collect(&entry.path(), &rel_path, out)?;
        } else {
            out.push((rel_path, entry.path()));
        }
    }
    Ok(())
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let web = manifest.join("../../web");
    println!("cargo:rerun-if-changed=../../web");
    let mut files = Vec::new();
    if let Err(e) = collect(&web, "", &mut files) {
        panic!("cannot read web/ directory {}: {e}", web.display());
    }
    files.sort();
    let mut code = String::from("pub static ASSETS: &[(&str, &[u8], &str)] = &[\n");
    for (rel, path) in &files {
        let abs = path.canonicalize().unwrap_or_else(|_| path.clone());
        println!("cargo:rerun-if-changed={}", abs.display());
        let _ = writeln!(
            code,
            "    ({rel:?}, include_bytes!({:?}), {:?}),",
            abs.display().to_string(),
            mime_of(rel)
        );
    }
    code.push_str("];\n");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap_or_default()).join("assets.rs");
    if let Err(e) = fs::write(&out, code) {
        panic!("cannot write {}: {e}", out.display());
    }
}
