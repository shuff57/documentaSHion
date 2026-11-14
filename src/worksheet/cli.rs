//! `documentashion ws …`: the worksheet rebuild from a terminal.
//!
//! ```text
//! documentashion ws extract <scan.png> <sheet.json> [--model opus]   read the printed worksheet
//! documentashion ws build   <sheet.json> <out.docx>                   write the Word document
//! documentashion ws render  <doc.docx> <out.png>                      draw it with WordCraft
//! documentashion ws compare <scan.png> <doc.docx> <out.png> [--shear S] scan and rebuild overlaid
//! documentashion ws fit     <scan.png> <sheet.json> [--rounds N]      line the rebuild up with the scan
//! documentashion ws graphs  <scan.png> <sheet.json> [--shear S]       read each graph again, close up
//! documentashion ws upright <scan.png> <out.png>                       turn the page the right way up
//! documentashion ws rebuild <scan.png> <out.docx> [--model opus]      all of it: upright, extract, fit, build
//! ```
use super::*;
use std::{
    path::{Path, PathBuf},
    process::Command,
};

const USAGE: &str = "usage: documentashion ws extract|build|render|compare|fit|graphs|upright|rebuild …";

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
        "upright" => upright(Path::new(arg(1)?), Path::new(arg(2)?)).map(|_| ()),
        "graphs" => {
            let path = Path::new(arg(2)?);
            let mut sheet = load(path)?;
            let shear = flag("--shear").and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let n = graphs::reread(Path::new(arg(1)?), &mut sheet, shear, &model)?;
            eprintln!("read {n} graphs again");
            save(&sheet, path)
        }
        "rebuild" => {
            let out = Path::new(arg(2)?);
            rebuild(Path::new(arg(1)?), out, &model).map(|r| eprintln!("{}: {r}", out.display()))
        }
        _ => Err(USAGE.into()),
    }
}

/// Writes `scan` turned the right way up to `out`; returns the quarter turns it took.
pub fn upright(scan: &Path, out: &Path) -> Result<u8, String> {
    let img = image::open(scan).map_err(|e| format!("{}: {e}", scan.display()))?;
    let turns = orient::upright(&img);
    orient::turn(img, turns).save(out).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(turns)
}

/// The whole rebuild of one scanned page: turned upright (kept beside `out` as
/// `<name>.scan.png`), read, fitted to the scan and written as `out`; the worksheet itself is
/// kept as `<name>.json`. Returns how well it fits.
pub fn rebuild(scan: &Path, out: &Path, model: &str) -> Result<fit::Report, String> {
    let page = out.with_extension("scan.png");
    let turns = upright(scan, &page)?;
    if turns != 0 {
        eprintln!("turned the page {} degrees", turns as u32 * 90);
    }
    let mut sheet = extract::extract(&page, model)?;
    let json = out.with_extension("json");
    save(&sheet, &json)?;
    let mut report = fit::fit(&page, &mut sheet, 10)?;
    save(&sheet, &json)?;
    // Graphs are read again close up once the fit has found them on the scan, then fitted again
    // as redrawn.
    if graphs::reread(&page, &mut sheet, report.shear, model)? > 0 {
        save(&sheet, &json)?;
        report = fit::fit(&page, &mut sheet, 10)?;
        save(&sheet, &json)?;
    }
    build(&sheet, out)?;
    Ok(report)
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

/// The program that draws verSHion figures (`tools/vershion-figure`, which `install.sh` puts
/// beside the app): `DOCUMENTASHION_VERSHION_FIGURE`, else one next to this program, else in this
/// checkout's `tools` build, else `vershion-figure` on PATH. It reads `{"id":…, "params":{…}}` on
/// stdin and prints the SVG.
pub fn vershion_figure() -> PathBuf {
    if let Some(p) = std::env::var_os("DOCUMENTASHION_VERSHION_FIGURE") {
        return p.into();
    }
    let exe = std::env::current_exe().ok();
    let beside = exe.as_ref().and_then(|e| e.parent()).map(|d| d.join("vershion-figure"));
    // target/{debug,release}/documentashion → tools/vershion-figure/target/release/…
    let checkout = exe
        .as_ref()
        .and_then(|e| e.ancestors().nth(3))
        .map(|root| root.join("tools/vershion-figure/target/release/vershion-figure"));
    beside.into_iter().chain(checkout).find(|p| p.is_file()).unwrap_or_else(|| "vershion-figure".into())
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
    // The figure's own blank margin is trimmed off: the size read from the scan is the size of
    // what is printed, not of a box around it.
    let out = Command::new("magick")
        .args(["-density", "300", "-background", "white"])
        .arg(&src)
        .args(["-flatten", "-fuzz", "2%", "-trim", "+repage", "-resize", &format!("{px_w}x{px_h}!")])
        .arg(&png)
        .output()
        .map_err(|e| format!("magick: {e}"))?;
    if !out.status.success() {
        return Err(format!("magick: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    std::fs::read(&png).map_err(|e| e.to_string())
}
