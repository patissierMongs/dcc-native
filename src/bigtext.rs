//! A 3x5 block font, doubled horizontally so digits keep their proportions on
//! a grid whose cells are roughly twice as tall as they are wide.

use crate::grid::Grid;

fn glyph(c: char) -> Option<[u8; 5]> {
    // Each row is three bits, most significant bit on the left.
    Some(match c {
        '0' => [0b111, 0b101, 0b101, 0b101, 0b111],
        '1' => [0b010, 0b110, 0b010, 0b010, 0b111],
        '2' => [0b111, 0b001, 0b111, 0b100, 0b111],
        '3' => [0b111, 0b001, 0b111, 0b001, 0b111],
        '4' => [0b101, 0b101, 0b111, 0b001, 0b001],
        '5' => [0b111, 0b100, 0b111, 0b001, 0b111],
        '6' => [0b111, 0b100, 0b111, 0b101, 0b111],
        '7' => [0b111, 0b001, 0b001, 0b010, 0b010],
        '8' => [0b111, 0b101, 0b111, 0b101, 0b111],
        '9' => [0b111, 0b101, 0b111, 0b001, 0b111],
        ':' => [0b000, 0b010, 0b000, 0b010, 0b000],
        ' ' => [0, 0, 0, 0, 0],
        _ => return None,
    })
}

fn width_of(c: char) -> usize {
    if c == ':' {
        3
    } else {
        7
    }
}

pub fn measure(s: &str) -> usize {
    s.chars().map(width_of).sum::<usize>().saturating_sub(1)
}

/// Draw `s` with its top-left at (x, y). Lit pixels are two cells wide.
pub fn draw(g: &mut Grid, x: usize, y: usize, s: &str, fg: u32, dim: u32) {
    let mut cx = x;
    for ch in s.chars() {
        let Some(rows) = glyph(ch) else { continue };
        let cols = if ch == ':' { 1 } else { 3 };
        let start = if ch == ':' { 1 } else { 0 };
        for (r, bits) in rows.iter().enumerate() {
            for c in 0..cols {
                let bit = (bits >> (2 - (c + start))) & 1;
                let (glyph_ch, colour) = if bit == 1 { ('█', fg) } else { ('·', dim) };
                if bit == 1 || ch != ' ' {
                    g.put(cx + c * 2, y + r, glyph_ch, colour);
                    g.put(cx + c * 2 + 1, y + r, glyph_ch, colour);
                }
            }
        }
        cx += width_of(ch);
    }
}
