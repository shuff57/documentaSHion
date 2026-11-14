//! Reads the printed worksheet from a scan with a vision model (Claude through the signed-in
//! Claude Code CLI), as a [`Sheet`].
use super::*;
use base64::Engine;
use std::{path::Path, time::Duration};

/// The verSHion figures a math worksheet uses; their parameters are listed in the prompt.
const FIGURES: [&str; 2] = ["coordinate-grid", "number-line"];

const PROMPT: &str = r#"You are rebuilding a scanned, printed school worksheet as an editable Word document.
The image is one sheet of paper, {W}×{H} pixels. It is US Letter (612×792 points) unless it is clearly another size, so 1 point ≈ {K} pixels.

Rebuild ONLY what was PRINTED. Leave out everything a person added by hand: students' answers, working, names, dates, circles, ticks, scores, stamps, staples and scanner marks. Where handwriting covers printed text, give the printed text. Keep every printed answer blank exactly as printed (a run of underscores, or a tab with an underscore leader).

Match the original's look and position as closely as you can: fonts, sizes, bold/italic, colours, alignment, tab stops, indents, and above all the vertical space. Answer space (empty room for work) must be the same height as on the scan, so every question starts at the same height on the page as in the scan. Measure positions on the image and convert pixels to points.

Return ONLY a JSON object (no commentary, no code fence) of this form; all sizes are points:
{"page":{"width":612,"height":792,"margins":[top,right,bottom,left],"header_at":36,"footer_at":36,
         "background":null or {"spacing":18,"color":"D5DCE6","width":0.5,"offset":[x,y]}},
 "font":"Arial","size":11,
 "header":[blocks], "footer":[blocks],
 "blocks":[blocks]}

Blocks:
- Paragraph: {"type":"p","align":"left|center|right|justify","before":0,"after":0,"line":null or exact line height,
   "indent":0,"hanging":0,"tabs":[{"pos":252,"align":"left|center|right|decimal","leader":"none|underscore|dot|hyphen"}],
   "rule_below":null or {"width":0.75,"color":"000000","style":"solid|dashed|dotted|double"},"rule_above":null or {...},
   "runs":[runs]}
- Table: {"type":"table","widths":[column widths],"borders":"none|all|outer|inner","align":"left|center|right",
   "rows":[{"height":null or minimum height,"cells":[{"blocks":[blocks],"span":1,"shade":null or "RRGGBB","valign":"top|center|bottom"}]}]}
   Use a borderless table to set things side by side (two columns of questions, a graph beside its questions), and a bordered one for printed tables (x/y tables, answer boxes).
- Figure: {"type":"figure","vershion":"coordinate-grid","params":{...},"width":w,"height":h,"align":"left|center|right","alt":"what it shows"}
   Draw printed graphs, axes, grids and number lines with these verSHion figures, setting the parameters so the drawing matches the print (range, step, numbering, arrows, gridlines). Parameters you leave out keep their defaults:
{FIGURES}
- Vertical space: {"type":"space","height":h}
- Page break: {"type":"break"}

Runs:
- Text: {"text":"…","font":null or name,"size":null or pt,"bold":false,"italic":false,"underline":false,"sup":false,"sub":false,"color":null or "RRGGBB"}
  "\t" in text moves to the next tab stop; "\n" breaks the line.
- Equation: {"math":"LaTeX","display":false,"size":null or pt} — use for printed math with structure: fractions, powers, roots, subscripts, or math set in an italic math font. Plain arithmetic printed in the text font stays text; write minus signs as "−" (U+2212) when the print uses a true minus.
- Tab: {"tab":true}    Line break: {"br":true}

Fonts: name the closest common font: Arial, Helvetica, Times New Roman, Georgia, Calibri, Cambria, Verdana, Merriweather, Comic Sans MS, Century Gothic. Measure sizes from the height of capital letters (cap height ≈ 0.7 of the point size).
Order blocks top to bottom. Header and footer are only for text repeated in the page's top or bottom margin; otherwise leave them empty.
Two columns of numbered questions are usually one paragraph per row with a tab stop for the second column, with the answer space as that paragraph's "after"."#;

/// The prompt for an image `w`×`h` pixels.
pub fn prompt(w: u32, h: u32) -> String {
    let figures = describe_figures();
    PROMPT
        .replace("{W}", &w.to_string())
        .replace("{H}", &h.to_string())
        .replace("{K}", &format!("{:.2}", w as f32 / 612.0))
        .replace("{FIGURES}", &figures)
}

/// The parameters of [`FIGURES`], from the verSHion figure program.
fn describe_figures() -> String {
    let out = std::process::Command::new(super::cli::vershion_figure()).arg("--describe").args(FIGURES).output();
    let Some(out) = out.ok().filter(|o| o.status.success()) else {
        return "  (verSHion figures are not available; use a borderless table or leave the figure out)".into();
    };
    let mut s = String::new();
    let mut last = "";
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    for line in text.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 5 {
            continue;
        }
        if f[0] != last {
            s.push_str(&format!("  {}:\n", f[0]));
        }
        last = f[0];
        s.push_str(&format!("    {} ({}; default {}): {}\n", f[1], f[2], f[3], f[4]));
    }
    s
}

/// Reads `scan` with Claude `model` and returns the worksheet. The reply is kept beside the
/// scratch files for checking.
pub fn extract(scan: &Path, model: &str) -> Result<Sheet, String> {
    let img = image::open(scan).map_err(|e| format!("{}: {e}", scan.display()))?;
    let img = if img.height() > 2000 { img.resize(1600, 2000, image::imageops::FilterType::Lanczos3) } else { img };
    let mut png = std::io::Cursor::new(Vec::new());
    img.write_to(&mut png, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(png.into_inner());
    let reply = crate::claude::chat(model, &prompt(img.width(), img.height()), &b64, Duration::from_secs(900))?;
    let _ = std::fs::write(super::cli::tmp("extract-reply.txt"), &reply);
    parse(&reply)
}

/// The sheet in a model's reply, tolerating a code fence or words around the JSON.
pub fn parse(reply: &str) -> Result<Sheet, String> {
    let (a, b) = (reply.find('{'), reply.rfind('}'));
    let (Some(a), Some(b)) = (a, b) else {
        return Err("The model's reply held no worksheet.".into());
    };
    let mut sheet: Sheet = serde_json::from_str(&reply[a..=b]).map_err(|e| format!("The model's worksheet didn't parse: {e}"))?;
    sheet.normalize();
    Ok(sheet)
}
