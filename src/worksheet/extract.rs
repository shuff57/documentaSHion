//! Reads the printed worksheet from a scan with a vision model (Claude through the signed-in
//! Claude Code CLI), as a [`Sheet`].
use super::*;
use base64::Engine;
use std::path::Path;

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
   Draw printed graphs, axes, grids and number lines with these verSHion figures, setting the parameters so the drawing matches the print (range, step, numbering, arrows, gridlines). Width and height are the printed graph's own extent (its grid and axis numbers), not the space around it. Always set "style":"Print" and "show_answers":false, and set "equations" and "points" to "" unless lines or points are PRINTED on the graph (never what a student drew). For a coordinate grid, "square": true makes one unit on x as long as one unit on y; when the axes count by different steps but the printed squares are square, set "square": false. Parameters you leave out keep their defaults:
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
    describe(&FIGURES)
}

/// The parameters of verSHion figures `ids`, one per line, from the verSHion figure program.
pub fn describe(ids: &[&str]) -> String {
    let out = std::process::Command::new(super::cli::vershion_figure()).arg("--describe").args(ids).output();
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

/// Reads `scan` with the model and returns the worksheet. The reply is kept beside the
/// scratch files for checking.
pub fn extract(scan: &Path, ask: Ask) -> Result<Sheet, String> {
    let img = image::open(scan).map_err(|e| format!("{}: {e}", scan.display()))?;
    let img = if img.height() > 2000 { img.resize(1600, 2000, image::imageops::FilterType::Lanczos3) } else { img };
    let mut png = std::io::Cursor::new(Vec::new());
    img.write_to(&mut png, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(png.into_inner());
    let reply = ask(&prompt(img.width(), img.height()), &b64)?;
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

const REVIEW: &str = r#"Left: a scanned, printed school worksheet. Right: our rebuild of its PRINTED content, drawn from the worksheet below.
Compare them. Find PRINTED content on the left that is missing on the right, or printed wrong (wrong words, numbers or symbols, a missing table row or column, a missing rule, blank or graph). Ignore everything handwritten, stamps and scanner marks, and ignore small differences of font, size or position.
The rebuild's top-level blocks, numbered:
{BLOCKS}
Return ONLY a JSON object: {"edits":[...]} where each edit is either
  {"replace": n, "block": {...}}   a whole corrected block n (for a table, the whole table with every row), or
  {"insert_after": n, "block": {...}}   a missing block (n = -1 for the very top).
Blocks use the same JSON as above. Return {"edits":[]} if nothing printed is missing or wrong."#;

/// A correction the review asks for.
#[derive(Debug, Deserialize)]
pub struct Correction {
    #[serde(default)]
    pub replace: Option<usize>,
    #[serde(default)]
    pub insert_after: Option<i64>,
    pub block: Block,
}

/// Shows the model the scan beside the sheet as built and applies the printed content it finds
/// missing or wrong. Returns how many corrections were made.
pub fn review(scan: &Path, sheet: &mut Sheet, ask: Ask) -> Result<usize, String> {
    let docx = super::cli::tmp("review.docx");
    super::cli::build(sheet, &docx)?;
    let png = super::cli::render(&docx, &super::cli::tmp("review.png"), 0)?;
    let ours = image::open(&png).map_err(|e| e.to_string())?.to_rgb8();
    let page = image::open(scan).map_err(|e| format!("{}: {e}", scan.display()))?;
    let theirs = page.resize(ours.width() * 2, ours.height(), image::imageops::FilterType::Triangle).resize_exact(ours.width(), ours.height(), image::imageops::FilterType::Triangle).to_rgb8();
    let mut both = image::RgbImage::from_pixel(ours.width() * 2 + 20, ours.height(), image::Rgb([128, 128, 128]));
    image::imageops::replace(&mut both, &theirs, 0, 0);
    image::imageops::replace(&mut both, &ours, ours.width() as i64 + 20, 0);
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(both).write_to(&mut bytes, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes.into_inner());
    let blocks: String = sheet
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| format!("{i}: {}\n", serde_json::to_string(b).unwrap_or_default()))
        .collect();
    let reply = ask(&REVIEW.replace("{BLOCKS}", &blocks), &b64)?;
    let (a, b) = (reply.find('{').ok_or("The review held no answer.")?, reply.rfind('}').ok_or("The review held no answer.")?);
    #[derive(Deserialize)]
    struct Reply {
        #[serde(default)]
        edits: Vec<Correction>,
    }
    let r: Reply = serde_json::from_str(&reply[a..=b]).map_err(|e| format!("The review didn't parse: {e}"))?;
    Ok(apply_corrections(sheet, r.edits))
}

/// Applies review corrections, last place first so earlier numbers still point where they did.
pub fn apply_corrections(sheet: &mut Sheet, mut edits: Vec<Correction>) -> usize {
    let at = |c: &Correction| c.replace.map(|n| n as i64).or(c.insert_after).unwrap_or(-2);
    edits.sort_by_key(|c| std::cmp::Reverse(at(c)));
    let mut n = 0;
    for c in edits {
        if let Some(i) = c.replace.filter(|&i| i < sheet.blocks.len()) {
            sheet.blocks[i] = c.block;
            n += 1;
        } else if let Some(i) = c.insert_after.filter(|&i| i >= -1 && i < sheet.blocks.len() as i64) {
            sheet.blocks.insert((i + 1) as usize, c.block);
            n += 1;
        }
    }
    sheet.normalize();
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corrections_apply_from_the_bottom_up() {
        let mut s: Sheet = serde_json::from_str(r#"{"blocks":[{"type":"p","runs":[{"text":"a"}]},{"type":"p","runs":[{"text":"b"}]}]}"#).unwrap();
        let edits: Vec<Correction> = serde_json::from_str(
            r#"[{"insert_after":-1,"block":{"type":"p","runs":[{"text":"top"}]}},
                {"replace":1,"block":{"type":"p","runs":[{"text":"B"}]}},
                {"insert_after":1,"block":{"type":"p","runs":[{"text":"end"}]}}]"#,
        )
        .unwrap();
        assert_eq!(apply_corrections(&mut s, edits), 3);
        let text: Vec<String> = s
            .blocks
            .iter()
            .map(|b| match b {
                Block::P(p) => p.runs.iter().map(|r| if let Run::Text(t) = r { t.text.clone() } else { String::new() }).collect(),
                _ => String::new(),
            })
            .collect();
        assert_eq!(text, ["top", "a", "B", "end"]);
    }
}
