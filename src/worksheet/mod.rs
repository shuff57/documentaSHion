//! Rebuilding a scanned worksheet as an editable Word document.
//!
//! A [`Sheet`] is the worksheet's printed content as a model reads it from the scan: paragraphs of
//! styled runs (math as LaTeX), tables, rules and figures, with their sizes and spacing in points.
//! [`docx::write`] turns it into a `.docx` that WordCraft (OfficeCraft) and Word open; equations
//! become native Word math through equaSHion, and figures are drawn by verSHion's figure
//! generators. Only the printed worksheet is rebuilt here; the student's writing is not part of a
//! `Sheet`.
pub mod cli;
pub mod docx;
pub mod extract;
pub mod fit;
pub mod graphs;
pub mod orient;

use serde::{Deserialize, Serialize};

/// One printed page.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Sheet {
    #[serde(default)]
    pub page: PageSetup,
    /// Body font and size (pt) for runs that give none.
    #[serde(default = "default_font")]
    pub font: String,
    #[serde(default = "default_size")]
    pub size: f32,
    pub blocks: Vec<Block>,
    /// Printed at the top and bottom of every page, inside the header and footer distances.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub header: Vec<Block>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub footer: Vec<Block>,
}

fn default_font() -> String {
    "Arial".into()
}
fn default_size() -> f32 {
    11.0
}

/// Paper and margins, in points.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PageSetup {
    #[serde(default = "letter_w")]
    pub width: f32,
    #[serde(default = "letter_h")]
    pub height: f32,
    /// Top, right, bottom, left.
    #[serde(default = "inch_margins")]
    pub margins: [f32; 4],
    #[serde(default)]
    pub landscape: bool,
    /// Distance of the header's top and the footer's bottom from the paper's edge, points.
    #[serde(default = "half_inch")]
    pub header_at: f32,
    #[serde(default = "half_inch")]
    pub footer_at: f32,
    /// Graph paper printed behind everything.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<GridPaper>,
}

/// A grid of lines across the whole sheet.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GridPaper {
    /// Distance between lines, points.
    pub spacing: f32,
    /// Line colour, `RRGGBB`.
    #[serde(default = "grid_color")]
    pub color: String,
    /// Line thickness, points.
    #[serde(default = "grid_line")]
    pub width: f32,
    /// Where the first vertical and horizontal lines fall, points from the left and top edges.
    #[serde(default)]
    pub offset: [f32; 2],
}
fn grid_color() -> String {
    "D5DCE6".into()
}
fn grid_line() -> f32 {
    0.5
}
fn half_inch() -> f32 {
    36.0
}
fn letter_w() -> f32 {
    612.0
}
fn letter_h() -> f32 {
    792.0
}
fn inch_margins() -> [f32; 4] {
    [72.0; 4]
}
impl Default for PageSetup {
    fn default() -> Self {
        PageSetup {
            width: letter_w(),
            height: letter_h(),
            margins: inch_margins(),
            landscape: false,
            header_at: half_inch(),
            footer_at: half_inch(),
            background: None,
        }
    }
}
impl PageSetup {
    /// Width of the text column, in points.
    pub fn text_width(&self) -> f32 {
        let w = if self.landscape { self.height.max(self.width) } else { self.width };
        w - self.margins[1] - self.margins[3]
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Block {
    P(Para),
    Table(Table),
    Figure(Figure),
    /// Vertical space, in points.
    Space { height: f32 },
    /// A page break.
    Break,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Para {
    #[serde(default)]
    pub align: Align,
    /// Space before and after, points.
    #[serde(default)]
    pub before: f32,
    #[serde(default)]
    pub after: f32,
    /// Exact line height in points; `None` is single spacing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<f32>,
    /// Left indent and first-line hanging indent, points.
    #[serde(default)]
    pub indent: f32,
    #[serde(default)]
    pub hanging: f32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tabs: Vec<Tab>,
    /// A rule under the paragraph (a horizontal line across the column).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_below: Option<Rule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_above: Option<Rule>,
    pub runs: Vec<Run>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Tab {
    /// From the left margin, points.
    pub pos: f32,
    #[serde(default)]
    pub align: TabAlign,
    /// A leader the tab fills with: an answer blank is a tab with an `underscore` leader.
    #[serde(default)]
    pub leader: Leader,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TabAlign {
    #[default]
    Left,
    Center,
    Right,
    Decimal,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Leader {
    #[default]
    None,
    Dot,
    Underscore,
    Hyphen,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Rule {
    /// Thickness, points.
    #[serde(default = "half_pt")]
    pub width: f32,
    #[serde(default = "black")]
    pub color: String,
    #[serde(default)]
    pub style: RuleStyle,
}
fn half_pt() -> f32 {
    0.75
}
fn black() -> String {
    "000000".into()
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RuleStyle {
    #[default]
    Solid,
    Dashed,
    Dotted,
    Double,
}

/// A piece of a paragraph.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Run {
    /// An inline equation, LaTeX; `display` sets it on its own line.
    Math {
        math: String,
        #[serde(default)]
        display: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        size: Option<f32>,
    },
    /// A tab to the paragraph's next tab stop.
    Tab { tab: bool },
    /// A line break inside the paragraph.
    Br { br: bool },
    Text(Text),
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Text {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub bold: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub italic: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub underline: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub sup: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub sub: bool,
    /// Hex colour, `RRGGBB`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}
fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Table {
    /// Column widths, points.
    pub widths: Vec<f32>,
    #[serde(default)]
    pub borders: Borders,
    /// Horizontal alignment of the whole table.
    #[serde(default)]
    pub align: Align,
    pub rows: Vec<Row>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Borders {
    /// No lines: a layout grid, e.g. two columns of questions side by side.
    #[default]
    None,
    /// Every cell boxed.
    All,
    /// Only the outer box.
    Outer,
    /// Only the lines between rows and columns.
    Inner,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Row {
    /// Minimum height, points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<f32>,
    pub cells: Vec<Cell>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Cell {
    #[serde(default)]
    pub blocks: Vec<Block>,
    /// Columns this cell spans.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub span: u32,
    /// Background, `RRGGBB`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shade: Option<String>,
    #[serde(default)]
    pub valign: VAlign,
}
fn one() -> u32 {
    1
}
fn is_one(n: &u32) -> bool {
    *n == 1
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VAlign {
    #[default]
    Top,
    Center,
    Bottom,
}

/// A drawing: one of verSHion's figure generators with its settings, or an image file.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Figure {
    /// verSHion figure id, e.g. `coordinate-grid`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vershion: Option<String>,
    /// The figure's settings, by verSHion parameter name.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub params: serde_json::Map<String, serde_json::Value>,
    /// An image file instead of a generated figure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Size on the page, points.
    pub width: f32,
    pub height: f32,
    #[serde(default)]
    pub align: Align,
    /// Space above it and its left indent, points.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub before: f32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub indent: f32,
    /// What it shows, for screen readers.
    #[serde(default)]
    pub alt: String,
}
fn is_zero(v: &f32) -> bool {
    *v == 0.0
}

/// Where a block is in a sheet: its index in the body, then, for each table it sits in, the
/// row, the cell and its index in that cell.
pub type Path = Vec<usize>;

/// The paragraphs, spacers and figures of `blocks`, tables included, in document order: the
/// order [`docx::write_as`] numbers its probe markers in.
pub fn anchors(blocks: &[Block]) -> Vec<Path> {
    fn walk(blocks: &[Block], prefix: &mut Path, out: &mut Vec<Path>) {
        for (i, b) in blocks.iter().enumerate() {
            prefix.push(i);
            match b {
                Block::P(_) | Block::Space { .. } | Block::Figure(_) => out.push(prefix.clone()),
                Block::Table(t) => {
                    for (r, row) in t.rows.iter().enumerate() {
                        for (c, cell) in row.cells.iter().enumerate() {
                            prefix.extend([r, c]);
                            walk(&cell.blocks, prefix, out);
                            prefix.truncate(prefix.len() - 2);
                        }
                    }
                }
                Block::Break => {}
            }
            prefix.pop();
        }
    }
    let mut out = vec![];
    walk(blocks, &mut vec![], &mut out);
    out
}

/// The block at `path`.
pub fn block_at<'a>(blocks: &'a [Block], path: &[usize]) -> Option<&'a Block> {
    let b = blocks.get(*path.first()?)?;
    match (b, &path[1..]) {
        (_, []) => Some(b),
        (Block::Table(t), [r, c, rest @ ..]) => block_at(&t.rows.get(*r)?.cells.get(*c)?.blocks, rest),
        _ => None,
    }
}

/// The block at `path`, to change.
pub fn block_at_mut<'a>(blocks: &'a mut [Block], path: &[usize]) -> Option<&'a mut Block> {
    let b = blocks.get_mut(*path.first()?)?;
    match (b, &path[1..]) {
        (b, []) => Some(b),
        (Block::Table(t), [r, c, rest @ ..]) => block_at_mut(&mut t.rows.get_mut(*r)?.cells.get_mut(*c)?.blocks, rest),
        _ => None,
    }
}

/// The list of blocks a path's last index is in: the body, or a table cell's blocks.
pub fn list_at<'a>(blocks: &'a mut Vec<Block>, path: &[usize]) -> Option<&'a mut Vec<Block>> {
    match path {
        [] | [_] => Some(blocks),
        [i, r, c, rest @ ..] => match blocks.get_mut(*i)? {
            Block::Table(t) => list_at(&mut t.rows.get_mut(*r)?.cells.get_mut(*c)?.blocks, rest),
            _ => None,
        },
        _ => None,
    }
}

/// The table at `path`, to change.
pub fn table_at_mut<'a>(blocks: &'a mut [Block], path: &[usize]) -> Option<&'a mut Table> {
    match block_at_mut(blocks, path)? {
        Block::Table(t) => Some(t),
        _ => None,
    }
}

impl Sheet {
    /// Splits tabs and line breaks typed inside text into tab and break runs, everywhere, so each
    /// column of a line is its own runs; and makes the space between paragraphs one-sided (see
    /// [`one_sided`]).
    pub fn normalize(&mut self) {
        for b in self.blocks.iter_mut().chain(self.header.iter_mut()).chain(self.footer.iter_mut()) {
            b.normalize();
        }
        one_sided(&mut self.blocks);
        one_sided(&mut self.header);
        one_sided(&mut self.footer);
    }
}

/// Moves a paragraph's space after into the next paragraph's space before. Word adds the two
/// where they meet but WordCraft (like HTML) keeps only the larger, so a gap set on one side
/// only is the same in both.
pub fn one_sided(blocks: &mut [Block]) {
    for i in 1..blocks.len() {
        let (head, tail) = blocks.split_at_mut(i);
        if let (Block::P(a), Block::P(b)) = (&mut head[i - 1], &mut tail[0]) {
            b.before += a.after;
            a.after = 0.0;
        }
    }
}

impl Block {
    fn normalize(&mut self) {
        match self {
            Block::P(p) => {
                let mut runs = Vec::with_capacity(p.runs.len());
                for r in p.runs.drain(..) {
                    let Run::Text(t) = r else {
                        runs.push(r);
                        continue;
                    };
                    if !t.text.contains(['\t', '\n']) {
                        runs.push(Run::Text(t));
                        continue;
                    }
                    for (i, line) in t.text.split('\n').enumerate() {
                        if i > 0 {
                            runs.push(Run::Br { br: true });
                        }
                        for (j, piece) in line.split('\t').enumerate() {
                            if j > 0 {
                                runs.push(Run::Tab { tab: true });
                            }
                            if !piece.is_empty() {
                                runs.push(Run::Text(Text { text: piece.to_string(), ..t.clone() }));
                            }
                        }
                    }
                }
                p.runs = runs;
            }
            Block::Table(t) => {
                for c in t.rows.iter_mut().flat_map(|r| r.cells.iter_mut()) {
                    for b in &mut c.blocks {
                        b.normalize();
                    }
                    one_sided(&mut c.blocks);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchors_walk_into_tables_in_document_order() {
        let s: Sheet = serde_json::from_str(
            r#"{"blocks":[{"type":"p","runs":[]},{"type":"table","widths":[10,10],"rows":[{"cells":[
                {"blocks":[{"type":"p","runs":[]},{"type":"figure","path":"x.png","width":1,"height":1}]},
                {"blocks":[{"type":"p","runs":[]}]}]}]},{"type":"break"},{"type":"space","height":3}]}"#,
        )
        .unwrap();
        let a = anchors(&s.blocks);
        assert_eq!(a, vec![vec![0], vec![1, 0, 0, 0], vec![1, 0, 0, 1], vec![1, 0, 1, 0], vec![3]]);
        assert!(matches!(block_at(&s.blocks, &a[2]), Some(Block::Figure(_))));
        let mut blocks = s.blocks.clone();
        assert_eq!(list_at(&mut blocks, &a[2]).unwrap().len(), 2);
        assert_eq!(list_at(&mut blocks, &a[4]).unwrap().len(), 4);
    }

    #[test]
    fn tabs_typed_in_text_become_tab_runs() {
        let mut s: Sheet = serde_json::from_str(r#"{"blocks":[{"type":"p","runs":[{"text":"(1) 5\t","bold":true},{"text":"a\nb"}]}]}"#).unwrap();
        s.normalize();
        let Block::P(p) = &s.blocks[0] else { panic!() };
        let kinds: Vec<&str> = p
            .runs
            .iter()
            .map(|r| match r {
                Run::Text(t) if t.bold => "bold",
                Run::Text(_) => "text",
                Run::Tab { .. } => "tab",
                Run::Br { .. } => "br",
                Run::Math { .. } => "math",
            })
            .collect();
        assert_eq!(kinds, ["bold", "tab", "text", "br", "text"]);
    }

    #[test]
    fn space_between_paragraphs_ends_up_before_the_second() {
        let mut s: Sheet = serde_json::from_str(r#"{"blocks":[{"type":"p","after":20,"runs":[]},{"type":"p","before":5,"after":7,"runs":[]},{"type":"space","height":3}]}"#).unwrap();
        s.normalize();
        let (Block::P(a), Block::P(b)) = (&s.blocks[0], &s.blocks[1]) else { panic!() };
        // The last paragraph keeps its space after: a spacer, not a paragraph, follows it.
        assert_eq!((a.before, a.after, b.before, b.after), (0.0, 0.0, 25.0, 7.0));
    }
}
