//! Draws a rebuilt page: each block at its place on a sheet of paper, printed text in the page
//! serif, the student's writing in blue, and LaTeX math typeset by equaSHion.
use crate::{
    model::{Block, Grade, Rebuild, Verdict, points},
    ui::{kit, theme, theme::icon},
};
use eframe::egui::{self, Color32, CornerRadius, FontId, Galley, Painter, Pos2, Rect, Stroke, StrokeKind, Vec2};
use equashion_layout::{DEFAULT_FAMILY, FontDb, Rgb, layout_equation};
use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};

/// Ink of printed text on the copy. The copy is always paper-white, in either theme.
pub const PRINTED: Color32 = Color32::from_rgb(0x1d, 0x1b, 0x18);
/// The student's writing.
pub const STUDENT: Color32 = Color32::from_rgb(0x1f, 0x4e, 0xc9);
/// Wash behind a block the model was unsure of.
const UNSURE_WASH: Color32 = Color32::from_rgba_premultiplied(64, 46, 0, 64);
const UNCLEAR: Color32 = Color32::from_rgb(0xb4, 0x5a, 0x00);
/// Marking pens.
const RIGHT: Color32 = Color32::from_rgb(0x1f, 0x8a, 0x4c);
const WRONG: Color32 = Color32::from_rgb(0xc6, 0x28, 0x28);
const PARTIAL: Color32 = Color32::from_rgb(0xc7, 0x77, 0x00);
const BLANK: Color32 = Color32::from_rgb(0x8a, 0x84, 0x7c);

fn fonts() -> &'static FontDb {
    static F: OnceLock<FontDb> = OnceLock::new();
    F.get_or_init(FontDb::bundled_only)
}

/// A piece of a block's text.
#[derive(Debug, PartialEq)]
pub enum Seg<'a> {
    Text(&'a str),
    /// LaTeX, and whether it is a display equation.
    Math(&'a str, bool),
    Break,
    /// `~~` toggles crossed-out writing.
    Strike,
    /// `**` toggles bold.
    Bold,
}

/// Splits text into plain runs, `$…$` / `$$…$$` / `\(…\)` / `\[…\]` math, line breaks, `~~` strike and `**` bold toggles.
/// An unmatched `$` stays as text.
pub fn segments(s: &str) -> Vec<Seg<'_>> {
    let mut out = vec![];
    let mut rest = s;
    while !rest.is_empty() {
        // `\(…\)` and `\[…\]`, which some models use instead of dollars.
        let delimited = [("\\(", "\\)", false), ("\\[", "\\]", true)]
            .into_iter()
            .find(|(o, ..)| rest.starts_with(o))
            .and_then(|(open, close, display)| rest[open.len()..].find(close).map(|e| (open.len(), e, close.len(), display)));
        if let Some((open, e, close, display)) = delimited {
            out.push(Seg::Math(rest[open..open + e].trim(), display));
            rest = &rest[open + e + close..];
            continue;
        }
        let next = rest.find(['$', '\n', '~', '\\', '*']).unwrap_or(rest.len());
        if next > 0 {
            out.push(Seg::Text(&rest[..next]));
            rest = &rest[next..];
            continue;
        }
        if let Some(r) = rest.strip_prefix('\n') {
            out.push(Seg::Break);
            rest = r;
        } else if let Some(r) = rest.strip_prefix("~~") {
            out.push(Seg::Strike);
            rest = r;
        } else if let Some(r) = rest.strip_prefix("**") {
            out.push(Seg::Bold);
            rest = r;
        } else if let Some(r) = rest.strip_prefix("$$") {
            match r.find("$$") {
                Some(e) => {
                    out.push(Seg::Math(r[..e].trim(), true));
                    rest = &r[e + 2..];
                }
                None => {
                    out.push(Seg::Text("$$"));
                    rest = r;
                }
            }
        } else if let Some(r) = rest.strip_prefix('$') {
            match r.find('$') {
                Some(e) if e > 0 => {
                    out.push(Seg::Math(r[..e].trim(), false));
                    rest = &r[e + 1..];
                }
                _ => {
                    out.push(Seg::Text("$"));
                    rest = r;
                }
            }
        } else {
            // A lone '~' or '*', or a backslash that doesn't open math.
            out.push(Seg::Text(&rest[..1]));
            rest = &rest[1..];
        }
    }
    out
}

/// A typeset equation at one size: its size and baseline in points, and its texture once drawn.
struct Eq {
    size: Vec2,
    baseline: f32,
    tex: Option<egui::TextureHandle>,
}

/// Typeset equations and fitted font sizes, kept between frames.
#[derive(Default)]
pub struct Cache {
    eqs: HashMap<(String, u32, bool, bool), Eq>,
    fits: HashMap<String, (f32, bool)>,
}
impl Cache {
    fn eq(&mut self, latex: &str, px: f32, display: bool, student: bool) -> &mut Eq {
        if self.eqs.len() > 4000 {
            self.eqs.clear();
        }
        let key = (latex.to_owned(), (px * 4.0).round() as u32, display, student);
        self.eqs.entry(key).or_insert_with(|| {
            let m = equashion_math::from_linear(latex);
            let c = if student { STUDENT } else { PRINTED };
            let l = layout_equation(fonts(), &m, DEFAULT_FAMILY, px as f64, display, Rgb(c.r(), c.g(), c.b()), 0.0);
            Eq {
                size: Vec2::new(l.width() as f32, l.height() as f32),
                baseline: l.baseline() as f32,
                tex: None,
            }
        })
    }
    /// The equation's texture, drawn at `ppp` pixels per point.
    fn texture(&mut self, ctx: &egui::Context, latex: &str, px: f32, display: bool, student: bool, ppp: f32) -> Option<egui::TextureHandle> {
        let e = self.eq(latex, px, display, student);
        if e.tex.is_none() {
            let m = equashion_math::from_linear(latex);
            let c = if student { STUDENT } else { PRINTED };
            let l = layout_equation(fonts(), &m, DEFAULT_FAMILY, px as f64, display, Rgb(c.r(), c.g(), c.b()), 0.0);
            let r = equashion_render::render_scene(&l.scene, fonts(), ppp as f64, None);
            let img = egui::ColorImage::from_rgba_unmultiplied([r.width as usize, r.height as usize], &r.to_straight());
            e.tex = Some(ctx.load_texture(format!("eq-{latex}-{px}"), img, egui::TextureOptions::LINEAR));
        }
        e.tex.clone()
    }
}

/// One placed item of a laid-out block, relative to the block's top-left corner.
enum Item {
    Text { at: Pos2, galley: Arc<Galley>, strike: bool },
    Math { at: Pos2, size: Vec2, latex: String, display: bool },
}

/// Lays a block out in `width` points at `px` font size; returns its items and height.
fn flow(painter: &Painter, cache: &mut Cache, b: &Block, px: f32, width: f32) -> (Vec<Item>, f32, f32) {
    let colour = if b.handwritten { STUDENT } else { PRINTED };
    let plain = FontId::new(px, theme::serif());
    let heavy = FontId::new(px, theme::serif_semibold());
    let space = px * 0.28;
    let line_gap = px * 0.25;
    // Pieces of a line: width, ascent above the baseline, descent below, and how to draw them.
    enum Piece {
        Word(Arc<Galley>, f32, bool),
        Eq(String, bool, Vec2, f32),
        Space,
        Break,
    }
    let mut pieces = vec![];
    let mut strike = false;
    let mut bold = false;
    for seg in segments(&b.text) {
        match seg {
            Seg::Text(t) => {
                let font = if bold { &heavy } else { &plain };
                for (i, word) in t.split(' ').enumerate() {
                    if i > 0 {
                        pieces.push(Piece::Space);
                    }
                    if word.is_empty() {
                        continue;
                    }
                    let mut start = 0;
                    // "[unclear]" stands out in amber, inside a run of text.
                    for (a, m) in word.match_indices("[unclear]") {
                        if a > start {
                            let g = painter.layout_no_wrap(word[start..a].to_owned(), font.clone(), colour);
                            let asc = ascent(&g);
                            pieces.push(Piece::Word(g, asc, strike));
                        }
                        let g = painter.layout_no_wrap(m.to_owned(), font.clone(), UNCLEAR);
                        let asc = ascent(&g);
                        pieces.push(Piece::Word(g, asc, strike));
                        start = a + m.len();
                    }
                    if start < word.len() {
                        let g = painter.layout_no_wrap(word[start..].to_owned(), font.clone(), colour);
                        let asc = ascent(&g);
                        pieces.push(Piece::Word(g, asc, strike));
                    }
                }
            }
            Seg::Math(latex, display) => {
                if latex.is_empty() {
                    continue;
                }
                let e = cache.eq(latex, px, display, b.handwritten);
                if display {
                    pieces.push(Piece::Break);
                }
                pieces.push(Piece::Eq(latex.to_owned(), display, e.size, e.baseline));
                if display {
                    pieces.push(Piece::Break);
                }
            }
            Seg::Break => pieces.push(Piece::Break),
            Seg::Strike => strike = !strike,
            Seg::Bold => bold = !bold,
        }
    }
    // Break into lines, then place each line on a shared baseline.
    let mut items = vec![];
    let mut lines: Vec<Vec<(f32, &Piece)>> = vec![vec![]];
    let mut x = 0.0f32;
    let mut widest = 0.0f32;
    for p in &pieces {
        let w = match p {
            Piece::Word(g, ..) => g.size().x,
            Piece::Eq(_, _, s, _) => s.x,
            Piece::Space => space,
            Piece::Break => {
                lines.push(vec![]);
                x = 0.0;
                continue;
            }
        };
        let line = lines.last_mut().unwrap();
        if matches!(p, Piece::Space) && line.is_empty() {
            continue;
        }
        if x + w > width && !line.is_empty() && !matches!(p, Piece::Space) {
            lines.push(vec![]);
            x = 0.0;
        }
        lines.last_mut().unwrap().push((x, p));
        x += w;
        widest = widest.max(x);
    }
    let mut y = 0.0f32;
    let text_asc = px * 0.95;
    let text_desc = px * 0.3;
    for line in lines.iter().filter(|l| !l.is_empty()) {
        let (mut asc, mut desc) = (0.0f32, 0.0f32);
        for (_, p) in line {
            match p {
                Piece::Word(..) | Piece::Space => {
                    asc = asc.max(text_asc);
                    desc = desc.max(text_desc);
                }
                Piece::Eq(_, _, s, base) => {
                    asc = asc.max(*base);
                    desc = desc.max(s.y - base);
                }
                Piece::Break => {}
            }
        }
        let baseline = y + asc;
        // A display equation on its own line is centred.
        let shift = match line.as_slice() {
            [(_, Piece::Eq(_, true, s, _))] => ((width - s.x) / 2.0).max(0.0),
            _ => 0.0,
        };
        for (x, p) in line {
            match p {
                Piece::Word(g, a, strike) => items.push(Item::Text {
                    at: Pos2::new(x + shift, baseline - a),
                    galley: g.clone(),
                    strike: *strike,
                }),
                Piece::Eq(latex, display, s, base) => items.push(Item::Math {
                    at: Pos2::new(x + shift, baseline - base),
                    size: *s,
                    latex: latex.clone(),
                    display: *display,
                }),
                _ => {}
            }
        }
        y = baseline + desc + line_gap;
    }
    (items, (y - line_gap).max(0.0), widest)
}

fn ascent(g: &Galley) -> f32 {
    g.rows
        .first()
        .and_then(|r| r.row.glyphs.first().map(|gl| r.pos.y + gl.pos.y))
        .unwrap_or(g.size().y * 0.8)
}

/// How far past its box a line may run, at most, into empty space. Typeset math is wider
/// than the scan's plain superscripts, and boxes hug the ink.
const OVERHANG: f32 = 1.6;

/// The width, in thousandths, a block's lines may use: its box, plus empty space to its right up
/// to the next block on the same row. Printed text runs under the student's writing (a name on
/// a blank); the student's writing stops at anything.
pub fn room(blocks: &[Block], i: usize) -> f32 {
    let [x, y, w, h] = blocks[i].bbox;
    let hand = blocks[i].handwritten;
    let next = blocks
        .iter()
        .enumerate()
        .filter(|&(j, o)| j != i && (hand || !o.handwritten) && o.bbox[0] >= x + w * 0.5 && o.bbox[1] < y + h && o.bbox[1] + o.bbox[3] > y)
        .map(|(_, o)| o.bbox[0] - x - 4.0)
        .fold(f32::MAX, f32::min);
    (w * OVERHANG).min(next).max(w).min(1000.0 - x)
}

/// A block's font size before sizes are evened out, and whether it wraps. Each line of text
/// gets one line of type in the box's height; a line too wide for `width` shrinks. A block the
/// model gave as one long line that the box shows as a paragraph wraps instead. Cached under `key`.
fn fit(painter: &Painter, cache: &mut Cache, b: &Block, r: Rect, width: f32, key: &str) -> (f32, bool) {
    if let Some(&f) = cache.fits.get(key) {
        return f;
    }
    let lines = (b.text.lines().count() + b.text.matches("$$").count() / 2).max(1) as f32;
    let tall = (r.height() / lines / 1.25).max(3.0);
    let natural = |cache: &mut Cache, px: f32| flow(painter, cache, b, px, f32::INFINITY).2;
    let search = |cache: &mut Cache, ok: &dyn Fn(&mut Cache, f32) -> bool| {
        let (mut lo, mut hi) = (3.0f32, tall);
        for _ in 0..9 {
            let mid = (lo + hi) / 2.0;
            if ok(cache, mid) { lo = mid } else { hi = mid }
        }
        lo
    };
    let n = natural(cache, tall);
    let fit = if lines <= 1.0 && n > width * 1.8 && r.height() > tall * 2.5 {
        (search(cache, &|c, px| flow(painter, c, b, px, width).1 <= r.height() * 1.15), true)
    } else if lines <= 1.0 && n > 0.0 {
        // One line: as long as the scan's line (box widths are steadier than heights, which
        // superscripts and descenders inflate), within reason of its height and its room.
        let by_width = tall * r.width() * 1.05 / n;
        (by_width.clamp(tall * 0.6, tall * 1.6).min(tall * width / n), false)
    } else if n > width {
        (search(cache, &|c, px| natural(c, px) <= width), false)
    } else {
        (tall, false)
    };
    if cache.fits.len() > 4000 {
        cache.fits.clear();
    }
    cache.fits.insert(key.to_owned(), fit);
    fit
}

/// Sizes within this factor of each other are taken to be the same type. Line boxes vary a lot
/// even after sizing by width; a title is usually far larger than body text.
const BAND: f32 = 1.25;

/// Evens out sizes: each block takes the median of the sizes within `BAND` of its own, among
/// blocks of the same kind (printed or the student's). Body text ends up one size; a title
/// stays larger. A block never grows past `max`, the size at which it still fits its room.
pub fn even(sizes: &[(bool, f32, f32)]) -> Vec<f32> {
    sizes
        .iter()
        .map(|&(hand, px, max)| {
            let mut near: Vec<f32> = sizes.iter().filter(|o| o.0 == hand && o.1 >= px / BAND && o.1 <= px * BAND).map(|o| o.1).collect();
            near.sort_by(f32::total_cmp);
            near[near.len() / 2].min(max.max(px))
        })
        .collect()
}

/// Identifies a rebuild's content, so cached font sizes never outlive it.
pub fn key(r: &Rebuild) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for b in &r.blocks {
        b.text.hash(&mut h);
        b.bbox.map(f32::to_bits).hash(&mut h);
        b.line.to_bits().hash(&mut h);
        b.handwritten.hash(&mut h);
    }
    format!("{:x}", h.finish())
}

/// The screen rectangle of a block on a page drawn in `page`.
pub fn block_rect(b: &Block, page: Rect) -> Rect {
    let s = page.size() / 1000.0;
    Rect::from_min_size(page.min + Vec2::new(b.bbox[0] * s.x, b.bbox[1] * s.y), Vec2::new(b.bbox[2] * s.x, b.bbox[3] * s.y))
}

/// Draws the copy into `page` (paper already painted). `key` identifies the rebuild for caching.
/// Returns the block under the pointer.
pub fn draw(ui: &egui::Ui, cache: &mut Cache, r: &Rebuild, page: Rect, key: &str) -> Option<usize> {
    let painter = ui.painter_at(page);
    let ppp = ui.ctx().pixels_per_point();
    let pointer = ui.ctx().pointer_hover_pos().filter(|p| page.contains(*p) && ui.rect_contains_pointer(page));
    let mut hovered = None;
    let scale = page.width() / 1000.0;
    let fitted: Vec<(f32, bool, f32)> = r
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let br = block_rect(b, page);
            let width = room(&r.blocks, i) * scale;
            let (px, wrap) = fit(&painter, cache, b, br, width, &format!("{key}-{i}-{:.0}x{:.0}", br.width(), br.height()));
            (px, wrap, width)
        })
        .collect();
    // The largest size at which each block still fits its room caps it when sizes are evened.
    let raw: Vec<(bool, f32, f32)> = r
        .blocks
        .iter()
        .zip(&fitted)
        .map(|(b, &(px, wrap, width))| {
            let natural = if wrap { 0.0 } else { flow(&painter, cache, b, px, f32::INFINITY).2 };
            (b.handwritten, px, if natural > 0.0 { px * width / natural * 0.98 } else { px })
        })
        .collect();
    let sizes = even(&raw);
    // Where each block's drawn text ends, for placing marks after it.
    let mut ends = vec![0.0; r.blocks.len()];
    for (i, b) in r.blocks.iter().enumerate() {
        let br = block_rect(b, page);
        if let Some(dotted) = rule(&b.text) {
            let (y, ink) = (br.center().y, if b.handwritten { STUDENT } else { PRINTED });
            let thick = (page.width() / 1000.0 * 0.9).max(0.6);
            if dotted {
                let mut x = br.left();
                while x < br.right() {
                    painter.circle_filled(Pos2::new(x, y), thick * 0.7, ink);
                    x += thick * 3.0;
                }
            } else {
                painter.line_segment([Pos2::new(br.left(), y), Pos2::new(br.right(), y)], Stroke::new(thick, ink));
            }
            ends[i] = br.right();
            if pointer.is_some_and(|p| br.expand2(Vec2::new(0.0, 3.0)).contains(p)) {
                hovered = Some(i);
            }
            continue;
        }
        if b.conf < Rebuild::UNSURE || b.text.contains("[unclear]") {
            painter.rect_filled(br.expand(2.0), CornerRadius::same(3), UNSURE_WASH);
        }
        let px = sizes[i];
        let (_, wrap, width) = fitted[i];
        let (items, _, widest) = flow(&painter, cache, b, px, if wrap { width } else { f32::INFINITY });
        ends[i] = br.right().max(br.left() + widest);
        for it in items {
            match it {
                Item::Text { at, galley, strike } => {
                    let at = br.min + at.to_vec2();
                    let size = galley.size();
                    painter.galley(at, galley, PRINTED);
                    if strike {
                        let y = at.y + size.y * 0.55;
                        painter.line_segment([Pos2::new(at.x, y), Pos2::new(at.x + size.x, y)], Stroke::new((px * 0.08).max(1.0), STUDENT));
                    }
                }
                Item::Math { at, size, latex, display } => {
                    if let Some(t) = cache.texture(ui.ctx(), &latex, px, display, b.handwritten, ppp) {
                        let at = br.min + at.to_vec2();
                        painter.image(t.id(), Rect::from_min_size(at, size), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                    }
                }
            }
        }
        if pointer.is_some_and(|p| br.contains(p)) {
            hovered = Some(i);
        }
    }
    if let Some(g) = &r.grade {
        draw_marks(&painter, g, r, page, &ends);
    }
    if let Some(i) = hovered {
        let br = block_rect(&r.blocks[i], page);
        painter.rect_stroke(br.expand(3.0), CornerRadius::same(3), Stroke::new(1.5, theme::accent()), StrokeKind::Outside);
    }
    hovered
}

/// A block that is a horizontal rule: "---" draws a line across its box, "..." a dotted one.
fn rule(text: &str) -> Option<bool> {
    let t = text.trim();
    let all = |set: &[char]| t.chars().count() >= 3 && t.chars().all(|c| set.contains(&c));
    if all(&['-', '—', '–']) {
        Some(false)
    } else if all(&['.', '·', '…']) {
        Some(true)
    } else {
        None
    }
}

/// A tick, cross or part mark after each answer, its points, and the score at the top right.
fn draw_marks(painter: &Painter, g: &Grade, r: &Rebuild, page: Rect, ends: &[f32]) {
    let rad = (page.width() / 1000.0 * 13.0).clamp(7.0, 30.0);
    for m in &g.marks {
        let Some(area) = m.blocks.iter().map(|&i| block_rect(&r.blocks[i], page)).reduce(|a, b| a.union(b)) else { continue };
        let end = m.blocks.iter().map(|&i| ends[i]).fold(area.right(), f32::max);
        let c = Pos2::new((end + rad * 1.8).min(page.right() - rad * 3.0), area.center().y);
        let (glyph, pen) = match m.verdict {
            Verdict::Right => (icon::CHECK, RIGHT),
            Verdict::Partial => (icon::CHECK, PARTIAL),
            Verdict::Wrong => (icon::X, WRONG),
            Verdict::Blank => (icon::X, BLANK),
        };
        if m.unsure() {
            painter.circle_filled(c, rad * 1.35, UNSURE_WASH);
        }
        kit::glyph(painter, c, glyph, rad * 2.0, pen);
        if m.verdict == Verdict::Partial || m.max != 1.0 {
            painter.text(
                Pos2::new(c.x + rad * 1.2, c.y),
                egui::Align2::LEFT_CENTER,
                format!("{}/{}", points(m.points), points(m.max)),
                FontId::new(rad * 1.15, theme::bold()),
                pen,
            );
        }
        if m.unsure() {
            painter.text(Pos2::new(c.x + rad, c.y - rad), egui::Align2::CENTER_CENTER, "?", FontId::new(rad * 1.1, theme::bold()), UNCLEAR);
        }
    }
    let (s, max) = g.score();
    let at = page.right_top() + Vec2::new(-page.width() * 0.04, page.width() * 0.035);
    let score = painter.text(at, egui::Align2::RIGHT_TOP, format!("{} / {}", points(s), points(max)), FontId::new(rad * 2.2, theme::serif_semibold()), WRONG);
    painter.line_segment([Pos2::new(score.left(), score.bottom() + 2.0), Pos2::new(score.right(), score.bottom() + 2.0)], Stroke::new((rad * 0.15).max(1.0), WRONG));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn text_splits_into_math_breaks_and_strikes() {
        use Seg::*;
        assert_eq!(
            segments("2. Solve $x^2=4$\n~~x=3~~ $$x=\\pm2$$ costs $5"),
            vec![Text("2. Solve "), Math("x^2=4", false), Break, Strike, Text("x=3"), Strike, Text(" "), Math("x=\\pm2", true), Text(" costs "), Text("$"), Text("5")]
        );
        assert_eq!(segments("**Quiz** 2*3"), vec![Bold, Text("Quiz"), Bold, Text(" 2"), Text("*"), Text("3")]);
        assert_eq!((rule(" --- "), rule("....."), rule("____"), rule("-5")), (Some(false), Some(true), None, None));
        assert_eq!(segments(r"1. Solve \(x^2=4\) \[y\] a\b"), vec![Text("1. Solve "), Math("x^2=4", false), Text(" "), Math("y", true), Text(" a"), Text("\\"), Text("b")]);
    }
    #[test]
    fn body_sizes_even_out_but_a_title_stays_large() {
        let s = even(&[(false, 30.0, 99.0), (false, 10.5, 99.0), (false, 12.0, 99.0), (false, 11.0, 99.0), (true, 15.0, 99.0), (false, 11.5, 11.0)]);
        assert_eq!(s[0], 30.0);
        assert!(s[1..4].iter().all(|v| *v == s[1]), "{s:?}");
        assert_eq!(s[4], 15.0);
        assert!(s[5] <= 11.5);
    }
    #[test]
    fn room_stops_at_the_next_block_on_the_row() {
        let b = |x: f32, y: f32, w: f32| Block { bbox: [x, y, w, 20.0], ..Default::default() };
        let mut blocks = [b(100.0, 100.0, 80.0), b(200.0, 105.0, 50.0), b(100.0, 300.0, 100.0), b(120.0, 300.0, 50.0)];
        blocks[3].handwritten = true;
        assert_eq!(room(&blocks, 0), 96.0);
        // Printed text ignores the writing on its blank.
        assert_eq!(room(&blocks, 2), 160.0);
    }
    #[test]
    fn latex_typesets_with_a_baseline() {
        let mut c = Cache::default();
        let e = c.eq(r"\frac{-b\pm\sqrt{b^2-4ac}}{2a}", 20.0, false, true);
        assert!(e.size.x > 40.0 && e.size.y > 20.0);
        assert!(e.baseline > 0.0 && e.baseline < e.size.y);
    }
}
