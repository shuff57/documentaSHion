//! A first placement from the words on the scan. The model's spacing is read by eye; on a
//! dense page it can be off by more than a line, and the fit then settles on a neighbouring
//! line that looks alike. Here Tesseract reads the scan once, each block's first line as drawn
//! (from the PDF's words) is found among the scan's words by its own words, and the blocks are
//! moved there before the fit polishes them.
use super::*;
use std::collections::HashMap;

/// A word and where it stands: its letters and digits in lower case, its left edge and its
/// baseline, points.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Word {
    pub text: String,
    pub x: f32,
    pub base: f32,
}

/// A word's letters and digits, lower case: what OCR and the PDF agree on.
pub(super) fn norm(s: &str) -> String {
    let s = s.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'");
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// The words in one page of `pdftotext -bbox` output, markers left out. A PDF word's box is its
/// font's, so the baseline is about a fifth of the way up from its bottom.
pub(super) fn pdf_words(xml: &str) -> Vec<Word> {
    let attr = |line: &str, name: &str| -> Option<f32> {
        let i = line.find(&format!("{name}=\""))? + name.len() + 2;
        line[i..].split('"').next()?.parse().ok()
    };
    xml.lines()
        .filter_map(|line| {
            let (x, y0, y1) = (attr(line, "xMin")?, attr(line, "yMin")?, attr(line, "yMax")?);
            let text = norm(line.split('>').nth(1)?.split('<').next()?);
            (y1 - y0 >= 2.5 && !text.is_empty()).then(|| Word { text, x, base: y1 - 0.19 * (y1 - y0) })
        })
        .collect()
}

/// The words Tesseract reads on a scan page at 150 dpi, read at twice the size in sparse-text
/// mode (a worksheet's text is scattered, and its order doesn't matter here). A word's box is
/// its ink, so the bottom is the baseline unless a letter hangs below it.
pub(super) fn scan_words(page: &GrayImage) -> Vec<Word> {
    let big = image::imageops::resize(page, page.width() * 2, page.height() * 2, image::imageops::FilterType::Triangle);
    let path = super::super::cli::tmp("seed-ocr.png");
    if big.save(&path).is_err() {
        return vec![];
    }
    let Ok(out) = Command::new(crate::services::tesseract()).arg(&path).arg("-").args(["--psm", "11", "--dpi", "300", "tsv"]).output() else {
        return vec![];
    };
    tsv_words(&String::from_utf8_lossy(&out.stdout), 2.0 * K)
}

/// The words of Tesseract TSV read at 50% confidence or better; `k` pixels per point.
fn tsv_words(tsv: &str, k: f32) -> Vec<Word> {
    tsv.lines()
        .skip(1)
        .filter_map(|row| {
            let f: Vec<&str> = row.splitn(12, '\t').collect();
            if f.len() < 12 || f[0] != "5" || f[10].trim().parse::<f32>().ok()? < 50.0 {
                return None;
            }
            let n = |i: usize| f[i].trim().parse::<f32>().ok();
            let (left, top, h) = (n(6)? / k, n(7)? / k, n(9)? / k);
            let raw = f[11].trim();
            let text = norm(raw);
            let hangs = raw.chars().any(|c| "gjpqy,;()[]{}|/".contains(c));
            (!text.is_empty()).then(|| Word { text, x: left, base: top + h - if hangs { 0.22 * h } else { 0.0 } })
        })
        .collect()
}

/// How much one matching word counts: by its length, and a number as much as a long word,
/// since the numbers are what tell a worksheet's look-alike lines apart.
fn weight(w: &Word) -> f32 {
    let n = w.text.chars().count() as f32;
    if w.text.chars().all(|c| c.is_ascii_digit()) {
        (n + 1.5).min(6.0)
    } else if n == 1.0 {
        0.5
    } else {
        n.min(6.0)
    }
}

/// A place a line might be on the scan: points down from where it was drawn, and how much of
/// it agrees there (summed word weights).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Cand {
    dy: f32,
    support: f32,
}

/// The least support a place needs: one longer word, or a few short ones.
const LEAST: f32 = 3.0;

/// Where on the scan the line of words `line` (as drawn) may be: groups of its words found in
/// the scan at the same offset, best first.
fn places(line: &[&Word], scan: &[Word]) -> Vec<Cand> {
    // Every pairing of a line word with the same word on the scan, near enough across.
    let pairs: Vec<(usize, f32, f32)> = line
        .iter()
        .enumerate()
        .flat_map(|(li, rw)| scan.iter().filter(move |sw| sw.text == rw.text && (sw.x - rw.x).abs() <= 72.0).map(move |sw| (li, sw.base - rw.base, sw.x - rw.x)))
        .collect();
    let mut found: Vec<(f32, f32)> = pairs
        .iter()
        .map(|&(_, dy, dx)| {
            // The words that agree with this pairing, each counted once.
            let mut best: HashMap<usize, (f32, f32)> = HashMap::new();
            for &(lj, ey, ex) in &pairs {
                if (ey - dy).abs() <= 3.0 && (ex - dx).abs() <= 10.0 {
                    let w = weight(line[lj]);
                    let e = best.entry(lj).or_insert((0.0, ey));
                    if w > e.0 || (w == e.0 && (ey - dy).abs() < (e.1 - dy).abs()) {
                        *e = (w, ey);
                    }
                }
            }
            let support: f32 = best.values().map(|v| v.0).sum();
            let mean = best.values().map(|v| v.0 * v.1).sum::<f32>() / support.max(1e-6);
            (support, mean)
        })
        .collect();
    found.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut out: Vec<Cand> = vec![];
    for (support, dy) in found {
        if support >= LEAST && out.iter().all(|c| (c.dy - dy).abs() > 5.0) {
            out.push(Cand { dy, support });
            if out.len() == 6 {
                break;
            }
        }
    }
    out
}

/// Points of support a change of offset between consecutive lines costs, per point: answer
/// space a size off moves the lines below it together, so offsets change seldom.
const JUMP: f32 = 0.03;

/// The best consistent choice of places for a list's lines, in order down the page (`y` as
/// drawn): what is chosen keeps its order on the scan, and the offset changes as little as it
/// can. `start` is the offset the list begins with.
fn choose(lines: &[(f32, Vec<Cand>)], start: f32) -> Vec<Option<Cand>> {
    let jump = |d: f32| JUMP * (d.abs() - 2.0).max(0.0);
    // best[i][j]: the most support of a choice ending with line i at its place j.
    let mut best: Vec<Vec<(f32, Option<(usize, usize)>)>> = vec![];
    for (i, (y, cands)) in lines.iter().enumerate() {
        let row = cands
            .iter()
            .map(|c| {
                let at = y + c.dy;
                let mut b = (c.support - jump(c.dy - start), None);
                for (pi, (py, pc)) in lines[..i].iter().enumerate() {
                    for (pj, p) in pc.iter().enumerate() {
                        if at > py + p.dy + 2.0 {
                            let v = best[pi][pj].0 + c.support - jump(c.dy - p.dy);
                            if v > b.0 {
                                b = (v, Some((pi, pj)));
                            }
                        }
                    }
                }
                b
            })
            .collect();
        best.push(row);
    }
    let mut out = vec![None; lines.len()];
    let mut at = best.iter().enumerate().flat_map(|(i, r)| r.iter().enumerate().map(move |(j, v)| (v.0, i, j))).max_by(|a, b| a.0.total_cmp(&b.0)).map(|v| (v.1, v.2));
    while let Some((i, j)) = at {
        out[i] = Some(lines[i].1[j]);
        at = best[i][j].1;
    }
    out
}

/// Each block whose first line is found among the scan's words: its path, how far down to
/// move it, where it was drawn (as [`place`] takes them) and how much of it agrees there.
pub(super) fn offsets(p: &Probe, sheet: &Sheet, scan: &[Word]) -> HashMap<super::super::Path, (f32, f32, f32)> {
    let anchors = super::super::anchors(&sheet.blocks);
    // Each paragraph with words on its first line: its anchor, where it was drawn, and places.
    let mut lines: Vec<(usize, f32, Vec<Cand>)> = vec![];
    for m in p.marks.iter().filter(|m| m.part == 'B') {
        let Some(path) = anchors.get(m.index) else { continue };
        if !matches!(super::super::block_at(&sheet.blocks, path), Some(Block::P(_))) {
            continue;
        }
        let right = if path.len() > 1 { cell_width(&sheet.blocks, path).map(|w| m.x + w).unwrap_or(sheet.page.width) } else { sheet.page.width };
        let words: Vec<&Word> = p.words.iter().filter(|w| (w.base - m.y).abs() <= 2.5 && w.x >= m.x - 1.0 && w.x < right).collect();
        let cands = places(&words, scan);
        if !cands.is_empty() {
            lines.push((m.index, m.y, cands));
        }
    }
    let list_of = |i: usize| run_of(&anchors[i]);
    let mut lists: Vec<Vec<usize>> = lines.iter().map(|l| list_of(l.0)).collect();
    lists.sort_by(|a, b| (a.len(), a).cmp(&(b.len(), b)));
    lists.dedup();
    let mut chosen: HashMap<usize, (f32, f32, f32)> = HashMap::new();
    for list in lists {
        let members: Vec<&(usize, f32, Vec<Cand>)> = lines.iter().filter(|l| list_of(l.0) == list).collect();
        // A cell starts from the nearest line already placed above it.
        let first = members[0].0;
        let start = chosen.iter().filter(|(i, _)| **i < first).max_by_key(|(i, _)| **i).map(|(_, v)| v.0).unwrap_or(0.0);
        let input: Vec<(f32, Vec<Cand>)> = members.iter().map(|l| (l.1, l.2.clone())).collect();
        // A line whose words match about as well somewhere else (a question's parts repeated in
        // the next question), or that only a word or two match, is only kept close to the line
        // before it: a jump has to be sure.
        let mut last = start;
        for (l, c) in members.iter().zip(choose(&input, start)) {
            let Some(c) = c else { continue };
            let second = l.2.iter().filter(|o| *o != &c).map(|o| o.support).fold(0.0, f32::max);
            if (c.support < 1.5 * second || c.support < super::PINNED) && (c.dy - last).abs() > 10.0 {
                continue;
            }
            last = c.dy;
            chosen.insert(l.0, (c.dy, l.1, c.support));
        }
    }
    if std::env::var_os("DOCUMENTASHION_FIT_DEBUG").is_some() {
        for (i, y, c) in &lines {
            let words: Vec<&str> = p.words.iter().filter(|w| (w.base - y).abs() <= 2.5).map(|w| w.text.as_str()).take(6).collect();
            eprintln!("  seed anchor {i:>2} {:?} at y {y:6.1} {words:?}: chose {:?} of {:?}", anchors[*i], chosen.get(i).map(|v| v.0), c.iter().map(|c| (c.dy, c.support)).collect::<Vec<_>>());
        }
    }
    chosen.into_iter().map(|(i, v)| (anchors[i].clone(), v)).collect()
}

/// Moves the blocks of `sheet` to where their words are on `page` (the prepared scan), a few
/// times over since a move changes what wraps where; keeps whichever state puts the most of the
/// print on the scan's, the sheet as it came included.
pub(super) fn seed(page: &GrayImage, sheet: &mut Sheet, scan: &[Word]) -> Result<(), String> {
    if scan.len() < 10 {
        return Ok(());
    }
    let mut best: Option<(f32, Sheet)> = None;
    let mut first_pages = None;
    for round in 0..5 {
        let p = probe(sheet)?;
        let scan_page = if page.dimensions() == p.img.dimensions() {
            page.clone()
        } else {
            image::imageops::resize(page, p.img.width(), p.img.height(), image::imageops::FilterType::Triangle)
        };
        let agree = agreement(&p.img, &scan_page);
        // Not a state that runs onto more pages than the sheet did as it came.
        let pages = *first_pages.get_or_insert(p.pages);
        if p.pages <= pages && best.as_ref().is_none_or(|b| agree > b.0) {
            best = Some((agree, sheet.clone()));
        }
        let offs: HashMap<super::super::Path, (f32, f32)> = offsets(&p, sheet, scan).into_iter().map(|(k, v)| (k, (v.0, v.1))).collect();
        let lines = p.marks.iter().filter(|m| m.part == 'B').count();
        let off = offs.values().map(|v| v.0.abs()).fold(0.0, f32::max);
        eprintln!("seed {}: {} of {lines} lines found by their words, worst {off:.1} pt off; {:.0}% of the print on the scan's", round + 1, offs.len(), agree * 100.0);
        if round == 4 || off < 1.5 || place(sheet, &offs) == 0 {
            break;
        }
    }
    if let Some((_, s)) = best {
        *sheet = s;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(text: &str, x: f32, base: f32) -> Word {
        Word { text: text.into(), x, base }
    }

    #[test]
    fn a_line_is_found_where_most_of_its_words_agree() {
        // "Find the 3rd term" drawn at 100; on the scan 40 pt lower, and a stray "the" and
        // "find" elsewhere.
        let drawn = [w("find", 72.0, 100.0), w("the", 100.0, 100.0), w("3rd", 120.0, 100.0), w("term", 140.0, 100.0)];
        let line: Vec<&Word> = drawn.iter().collect();
        let scan = [w("find", 73.0, 140.5), w("the", 101.0, 140.0), w("3rd", 121.0, 140.5), w("term", 141.0, 140.0), w("the", 300.0, 400.0), w("find", 75.0, 600.0)];
        let p = places(&line, &scan);
        assert!((p[0].dy - 40.25).abs() < 0.5, "{p:?}");
        assert!(p[0].support > p.get(1).map(|c| c.support).unwrap_or(0.0) * 2.0, "{p:?}");
    }

    #[test]
    fn look_alike_lines_are_told_apart_by_their_numbers() {
        let drawn = [w("1", 72.0, 100.0), w("find", 90.0, 100.0), w("next", 120.0, 100.0)];
        let line: Vec<&Word> = drawn.iter().collect();
        let scan = [w("1", 72.0, 120.0), w("find", 90.0, 120.0), w("next", 120.0, 120.0), w("2", 72.0, 180.0), w("find", 90.0, 180.0), w("next", 120.0, 180.0)];
        let p = places(&line, &scan);
        assert!((p[0].dy - 20.0).abs() < 0.1 && p[0].support > p[1].support, "{p:?}");
    }

    #[test]
    fn the_choice_keeps_order_and_steady_offsets() {
        let c = |dy: f32, support: f32| Cand { dy, support };
        // Line 2 matches a far place a little better; order and steadiness keep it with the
        // others, 30 pt down.
        let lines = vec![(100.0, vec![c(30.0, 8.0)]), (150.0, vec![c(-80.0, 9.0), c(30.0, 8.0)]), (200.0, vec![c(30.0, 8.0)])];
        assert_eq!(choose(&lines, 0.0).iter().map(|c| c.map(|c| c.dy)).collect::<Vec<_>>(), vec![Some(30.0); 3]);
    }

    #[test]
    fn pdf_words_leave_markers_out() {
        let xml = r#"<word xMin="72.0" yMin="60.6" xMax="72.4" yMax="61.5">Q</word>
<word xMin="80.0" yMin="50.0" xMax="120.0" yMax="62.0">Building</word>"#;
        let words = pdf_words(xml);
        assert_eq!(words.len(), 1);
        assert_eq!(words[0].text, "building");
        assert!((words[0].base - 59.72).abs() < 0.01);
    }

    #[test]
    fn tsv_words_sit_on_their_baseline() {
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
5\t1\t1\t1\t1\t1\t100\t200\t80\t40\t90\tTerm\n\
5\t1\t1\t1\t1\t2\t200\t200\t80\t50\t90\tgap\n\
5\t1\t1\t1\t1\t3\t300\t200\t80\t40\t20\tzzz\n";
        let words = tsv_words(tsv, 2.0);
        assert_eq!(words.len(), 2);
        assert_eq!(words[0], w("term", 50.0, 120.0));
        assert!((words[1].base - (125.0 - 0.22 * 25.0)).abs() < 0.01);
    }
}
