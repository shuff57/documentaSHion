//! Word-level comparison of two texts: what a correction changed, and which OCR word box each
//! word of the accepted text belongs to.
use crate::model::Word;

/// A word and whether a line break precedes it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Token<'a> {
    pub text: &'a str,
    /// Number of line breaks before the word (0, 1 or 2 for a paragraph break).
    pub breaks: u8,
}

pub fn tokens(s: &str) -> Vec<Token<'_>> {
    let mut out = vec![];
    let mut breaks = 0u8;
    let mut start = None;
    for (i, c) in s.char_indices() {
        if c.is_whitespace() {
            if let Some(st) = start.take() {
                out.push(Token {
                    text: &s[st..i],
                    breaks,
                });
                breaks = 0;
            }
            if c == '\n' {
                breaks = (breaks + 1).min(2);
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(st) = start {
        out.push(Token {
            text: &s[st..],
            breaks,
        });
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    /// Index in the old text, index in the new text.
    Same(usize, usize),
    Removed(usize),
    Added(usize),
}

/// Longest-common-subsequence diff. Removals come before additions within a changed run.
pub fn diff<T: PartialEq>(a: &[T], b: &[T]) -> Vec<Op> {
    let prefix = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (am, bm) = (&a[prefix..a.len() - suffix], &b[prefix..b.len() - suffix]);
    let mut ops: Vec<Op> = (0..prefix).map(|i| Op::Same(i, i)).collect();
    let (n, m) = (am.len(), bm.len());
    if n * m > 4_000_000 {
        // Too different to compare word by word in reasonable memory: show a full replacement.
        ops.extend((0..n).map(|i| Op::Removed(prefix + i)));
        ops.extend((0..m).map(|j| Op::Added(prefix + j)));
    } else {
        // lcs[i][j]: common words between am[i..] and bm[j..].
        let mut lcs = vec![0u32; (n + 1) * (m + 1)];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                lcs[i * (m + 1) + j] = if am[i] == bm[j] {
                    lcs[(i + 1) * (m + 1) + j + 1] + 1
                } else {
                    lcs[(i + 1) * (m + 1) + j].max(lcs[i * (m + 1) + j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        let mut added = vec![];
        while i < n || j < m {
            if i < n && j < m && am[i] == bm[j] {
                ops.extend(added.drain(..));
                ops.push(Op::Same(prefix + i, prefix + j));
                i += 1;
                j += 1;
            } else if j < m && (i == n || lcs[i * (m + 1) + j + 1] >= lcs[(i + 1) * (m + 1) + j]) {
                added.push(Op::Added(prefix + j));
                j += 1;
            } else {
                ops.push(Op::Removed(prefix + i));
                i += 1;
            }
        }
        ops.extend(added);
    }
    let (a_end, b_end) = (a.len() - suffix, b.len() - suffix);
    ops.extend((0..suffix).map(|k| Op::Same(a_end + k, b_end + k)));
    ops
}

/// A word of the accepted text, with the OCR box it sits in and the OCR confidence if the
/// word is unchanged from OCR.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed<'a> {
    pub text: &'a str,
    pub bbox: Option<[u32; 4]>,
    pub conf: Option<f32>,
}

/// Matches the accepted text against the OCR words. Unchanged words keep their box. Corrected
/// words share the boxes of the OCR words they replaced, split by length. Inserted words
/// borrow a neighbour's box. Without OCR words, nothing gets a box.
pub fn place<'a>(words: &[Word], text: &'a str) -> Vec<Placed<'a>> {
    let new = tokens(text);
    let mut out: Vec<Placed> = new
        .iter()
        .map(|t| Placed {
            text: t.text,
            bbox: None,
            conf: None,
        })
        .collect();
    if words.is_empty() {
        return out;
    }
    let old: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
    let newt: Vec<&str> = new.iter().map(|t| t.text).collect();
    let ops = diff(&old, &newt);
    let mut k = 0;
    while k < ops.len() {
        match ops[k] {
            Op::Same(i, j) => {
                out[j].bbox = Some(words[i].bbox);
                out[j].conf = Some(words[i].conf);
                k += 1;
            }
            _ => {
                let start = k;
                while k < ops.len() && !matches!(ops[k], Op::Same(..)) {
                    k += 1;
                }
                let removed: Vec<usize> = ops[start..k]
                    .iter()
                    .filter_map(|o| if let Op::Removed(i) = o { Some(*i) } else { None })
                    .collect();
                let added: Vec<usize> = ops[start..k]
                    .iter()
                    .filter_map(|o| if let Op::Added(j) = o { Some(*j) } else { None })
                    .collect();
                if added.is_empty() {
                    continue;
                }
                if removed.is_empty() {
                    let neighbour = ops[..start]
                        .iter()
                        .rev()
                        .chain(ops[k..].iter())
                        .find_map(|o| if let Op::Same(i, _) = o { Some(words[*i].bbox) } else { None });
                    for j in added {
                        out[j].bbox = neighbour;
                    }
                    continue;
                }
                split_boxes(&removed.iter().map(|&i| words[i].bbox).collect::<Vec<_>>(), &added, &mut out);
            }
        }
    }
    out
}

/// Spreads replacement words across the replaced boxes, line by line, in proportion to length.
fn split_boxes(boxes: &[[u32; 4]], added: &[usize], out: &mut [Placed]) {
    // Group the replaced boxes into lines so a correction spanning a line break stays on its lines.
    let mut lines: Vec<[u32; 4]> = vec![];
    for b in boxes {
        match lines.last_mut() {
            Some(l) if b[1] < l[1] + l[3] && b[1] + b[3] > l[1] && b[0] >= l[0] => {
                let right = (l[0] + l[2]).max(b[0] + b[2]);
                let bottom = (l[1] + l[3]).max(b[1] + b[3]);
                l[1] = l[1].min(b[1]);
                l[2] = right - l[0];
                l[3] = bottom - l[1];
            }
            _ => lines.push(*b),
        }
    }
    let total_chars: usize = added.iter().map(|&j| out[j].text.chars().count() + 1).sum();
    let total_width: u32 = lines.iter().map(|l| l[2]).sum();
    let mut used = 0.0f64;
    for &j in added {
        let share = (out[j].text.chars().count() + 1) as f64 / total_chars as f64;
        let (mut start, end) = (used * total_width as f64, (used + share) * total_width as f64);
        used += share;
        // Find the line where this word starts.
        let mut acc = 0.0;
        for l in &lines {
            if start < acc + l[2] as f64 || std::ptr::eq(l, lines.last().unwrap()) {
                start -= acc;
                let width = (end - acc - start).min(l[2] as f64 - start).max(1.0);
                out[j].bbox = Some([l[0] + start as u32, l[1], width as u32, l[3]]);
                break;
            }
            acc += l[2] as f64;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn w(text: &str, x: u32) -> Word {
        Word {
            text: text.into(),
            conf: 90.0,
            bbox: [x, 100, 50, 20],
        }
    }
    #[test]
    fn tokens_keep_line_structure() {
        let t = tokens("Name: ___\n\n1. Solve  x\n");
        let words: Vec<_> = t.iter().map(|t| (t.text, t.breaks)).collect();
        assert_eq!(words, [("Name:", 0), ("___", 0), ("1.", 2), ("Solve", 0), ("x", 0)]);
    }
    #[test]
    fn diff_finds_word_changes() {
        let a = ["Solve", "for", "x", "when", "y", "=", "3"];
        let b = ["Solve", "for", "x", "if", "y", "=", "8"];
        let ops = diff(&a, &b);
        assert!(ops.contains(&Op::Removed(3)) && ops.contains(&Op::Added(3)));
        assert!(ops.contains(&Op::Removed(6)) && ops.contains(&Op::Added(6)));
        assert_eq!(ops.iter().filter(|o| matches!(o, Op::Same(..))).count(), 5);
    }
    #[test]
    fn corrected_words_land_in_the_boxes_they_replace() {
        let ocr = [w("Tbe", 0), w("cat", 60), w("sat", 120)];
        let placed = place(&ocr, "The cat sat down");
        assert_eq!(placed[0].bbox, Some([0, 100, 50, 20]));
        assert_eq!(placed[0].conf, None);
        assert_eq!(placed[1].bbox, Some([60, 100, 50, 20]));
        assert_eq!(placed[1].conf, Some(90.0));
        assert_eq!(placed[3].bbox, Some([120, 100, 50, 20])); // inserted: borrows a neighbour
    }
    #[test]
    fn a_split_word_shares_its_box() {
        let placed = place(&[w("homework", 0)], "home work");
        let (a, b) = (placed[0].bbox.unwrap(), placed[1].bbox.unwrap());
        assert_eq!(a[0], 0);
        assert!(b[0] >= 20 && b[0] + b[2] <= 51, "{a:?} {b:?}");
    }
    #[test]
    fn without_ocr_words_nothing_is_placed() {
        assert!(place(&[], "free text").iter().all(|p| p.bbox.is_none()));
    }
}
