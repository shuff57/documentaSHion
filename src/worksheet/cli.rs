//! `documentashion ws …`: the worksheet rebuild from a terminal.
//!
//! ```text
//! documentashion ws extract <scan.png> <sheet.json> [--model opus]   read the printed worksheet
//! documentashion ws build   <sheet.json> <out.docx>                   write the Word document
//! documentashion ws render  <doc.docx> <out.png>                      draw it with WordCraft
//! documentashion ws compare <scan.png> <doc.docx> <out.png> [--shear S] scan and rebuild overlaid
//! documentashion ws fit     <scan.png> <sheet.json> [--rounds N]      line the rebuild up with the scan
//! documentashion ws rebuild <scan.png> <out.docx> [--model opus]      extract, fit and build
//! ```
use super::*;
use std::{
    path::{Path, PathBuf},
    process::Command,
};

const USAGE: &str = "usage: documentashion ws extract|build|render|compare|fit|rebuild …";

pub fn main(args: &[String]) -> Result<(), String> {
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let pos: Vec<&String> = {
        let mut v = vec![];
        let mut skip = false;
        for a in args {
            if skip {
                skip = false;
            } else if a.starts_with("--") {
                skip = true;
            } else {
                v.push(a);
            }
        }
        v
    };
    let arg = |i: usize| pos.get(i).map(|s| s.as_str()).ok_or_else(|| USAGE.to_string());
    let model = flag("--model").unwrap_or_else(|| "opus".into());
    match arg(0)? {
        "build" => build_file(Path::new(arg(1)?), Path::new(arg(2)?)),
        "render" => render(Path::new(arg(1)?), Path::new(arg(2)?), 0).map(|_| ()),
        "compare" => {
            let png = tmp("compare.png");
            render(Path::new(arg(2)?), &png, 0)?;
            let shear = flag("--shear").and_then(|s| s.parse().ok()).unwrap_or(0.0);
            fit::overlay(Path::new(arg(1)?), &png, Path::new(arg(3)?), shear)
        }
        "extract" => {
            let sheet = extract::extract(Path::new(arg(1)?), &model)?;
            save(&sheet, Path::new(arg(2)?))
        }
        "fit" => {
            let path = Path::new(arg(2)?);
            let mut sheet = load(path)?;
            let rounds = flag("--rounds").and_then(|n| n.parse().ok()).unwrap_or(4);
            let r = fit::fit(Path::new(arg(1)?), &mut sheet, rounds)?;
            eprintln!("{r}");
            save(&sheet, path)
        }
        "rebuild" => {
            let scan = Path::new(arg(1)?);
            let out = Path::new(arg(2)?);
            let mut sheet = extract::extract(scan, &model)?;
            save(&sheet, &out.with_extension("json"))?;
            fit::fit(scan, &mut sheet, 4)?;
            save(&sheet, &out.with_extension("json"))?;
            build(&sheet, out)
        }
        _ => Err(USAGE.into()),
    }
}

pub fn load(path: &Path) -> Result<Sheet, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut sheet: Sheet = serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    sheet.normalize();
    Ok(sheet)
}
pub fn save(sheet: &Sheet, path: &Path) -> Result<(), String> {
    std::fs::write(path, serde_json::to_vec_pretty(sheet).map_err(|e| e.to_string())?).map_err(|e| format!("{}: {e}", path.display()))
}

fn build_file(src: &Path, out: &Path) -> Result<(), String> {
    build(&load(src)?, out)
}

pub fn build(sheet: &Sheet, out: &Path) -> Result<(), String> {
    docx::write(sheet, out, &figure_png)
}

/// A scratch file under the system temp folder.
pub fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("documentashion-ws-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    dir.join(name)
}

/// Draws page `page` (from 0) of a document with WordCraft at 150 dpi.
pub fn render(doc: &Path, png: &Path, page: usize) -> Result<PathBuf, String> {
    let out = Command::new("wordcraft-cli")
        .arg("render")
        .arg(doc)
        .arg(png)
        .args(["--page", &(page + 1).to_string(), "--scale", &(150.0 / 72.0).to_string()])
        .output()
        .map_err(|e| format!("wordcraft-cli: {e} (is WordCraft installed?)"))?;
    if !out.status.success() {
        return Err(format!("wordcraft-cli render: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(png.to_path_buf())
}

/// The program that draws verSHion figures: `DOCUMENTASHION_VERSHION_FIGURE`, or
/// `vershion-figure` on PATH. It reads `{"id":…, "params":{…}}` on stdin and prints the SVG.
pub fn vershion_figure() -> PathBuf {
    std::env::var_os("DOCUMENTASHION_VERSHION_FIGURE").map(PathBuf::from).unwrap_or_else(|| "vershion-figure".into())
}

/// A figure as PNG bytes at 300 dpi of its size on the page.
pub fn figure_png(f: &Figure) -> Result<Vec<u8>, String> {
    let svg_path;
    let src: PathBuf = if let Some(id) = &f.vershion {
        let req = serde_json::json!({"id": id, "params": f.params});
        let mut child = Command::new(vershion_figure())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("verSHion figures: {e}"))?;
        {
            use std::io::Write;
            child.stdin.take().unwrap().write_all(req.to_string().as_bytes()).map_err(|e| e.to_string())?;
        }
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(format!("verSHion figure {id}: {}", String::from_utf8_lossy(&out.stderr).trim()));
        }
        svg_path = tmp(&format!("fig-{id}-{}.svg", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::write(&svg_path, &out.stdout).map_err(|e| e.to_string())?;
        svg_path.clone()
    } else if let Some(p) = &f.path {
        PathBuf::from(p)
    } else {
        return Err("A figure needs a verSHion id or an image path.".into());
    };
    let px_w = (f.width / 72.0 * 300.0).round().max(1.0) as u32;
    let px_h = (f.height / 72.0 * 300.0).round().max(1.0) as u32;
    let png = tmp("figure.png");
    let out = Command::new("magick")
        .args(["-density", "300", "-background", "white"])
        .arg(&src)
        .args(["-flatten", "-resize", &format!("{px_w}x{px_h}!")])
        .arg(&png)
        .output()
        .map_err(|e| format!("magick: {e}"))?;
    if !out.status.success() {
        return Err(format!("magick: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    std::fs::read(&png).map_err(|e| e.to_string())
}
