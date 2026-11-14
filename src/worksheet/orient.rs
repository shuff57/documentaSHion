//! Which way up a scanned page is. Sheets go through a feeder upside down or sideways; the
//! model and the fitter both want the page upright.
use image::DynamicImage;
use std::process::Command;

/// Quarter turns clockwise that make `img` upright: Tesseract reads a small copy at each turn
/// and the turn with the most confidently read text wins. 0 when Tesseract isn't installed or
/// nothing is readable.
pub fn upright(img: &DynamicImage) -> u8 {
    let small = img.resize(1100, 1100, image::imageops::FilterType::Triangle);
    let mut best = (0.0f32, 0u8);
    for turns in 0..4u8 {
        let turned = match turns {
            1 => small.rotate90(),
            2 => small.rotate180(),
            3 => small.rotate270(),
            _ => small.clone(),
        };
        let score = readable(&turned).unwrap_or(0.0);
        if score > best.0 {
            best = (score, turns);
        }
    }
    best.1
}

/// The image turned by `turns` quarter turns clockwise.
pub fn turn(img: DynamicImage, turns: u8) -> DynamicImage {
    match turns % 4 {
        1 => img.rotate90(),
        2 => img.rotate180(),
        3 => img.rotate270(),
        _ => img,
    }
}

/// How much text Tesseract reads with confidence: letters of words read at 70% or better. The
/// page is read as one block (`--psm 6`): page segmentation would read sideways text blocks
/// the right way round by itself and blur the difference between turns.
fn readable(img: &DynamicImage) -> Option<f32> {
    let path = super::cli::tmp(&format!("orient-{}.png", std::process::id()));
    img.to_luma8().save(&path).ok()?;
    let out = Command::new(crate::services::tesseract()).arg(&path).arg("-").args(["--psm", "6", "tsv"]).output().ok()?;
    let tsv = String::from_utf8_lossy(&out.stdout);
    Some(score_tsv(&tsv))
}

/// The score [`readable`] gives Tesseract's TSV: letters (alphanumeric characters) in words of
/// three or more characters read at 70% confidence or better.
fn score_tsv(tsv: &str) -> f32 {
    tsv.lines()
        .skip(1)
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            let (conf, text) = (f.get(10)?.parse::<f32>().ok()?, f.get(11)?.trim());
            let letters = text.chars().filter(|c| c.is_alphanumeric()).count();
            (conf >= 70.0 && letters >= 3).then_some(letters as f32)
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_confident_words_count() {
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
                   5\t1\t1\t1\t1\t1\t0\t0\t10\t10\t91.5\tSolving\n\
                   5\t1\t1\t1\t1\t2\t0\t0\t10\t10\t30.0\tqzxv\n\
                   5\t1\t1\t1\t1\t3\t0\t0\t10\t10\t95.0\t(1)\n";
        assert_eq!(score_tsv(tsv), 7.0);
    }
}
