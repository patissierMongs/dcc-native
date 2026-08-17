//! A character grid — the whole UI is drawn into this and nothing else.
//!
//! Working in cells rather than pixels is what makes the thing feel like a
//! terminal instead of a web page pretending to be one: every glyph lands on an
//! integer column, box-drawing joins line up, and layout is arithmetic.

#[derive(Clone, Copy, PartialEq)]
pub struct Cell {
    pub ch: char,
    pub fg: u32,
}

/// Marks the second cell of a full-width glyph. Never drawn; the painter
/// skips it and the glyph before it is positioned on its own.
pub const CONT: char = '\0';

/// Full-width (two-cell) characters: CJK, Hangul, full-width forms. A rough
/// East-Asian-Width table is all a status wall needs.
pub fn is_wide(ch: char) -> bool {
    matches!(ch as u32,
        0x1100..=0x115F        // Hangul Jamo
        | 0x2E80..=0x303E      // CJK radicals, punctuation
        | 0x3041..=0x33FF      // kana, CJK symbols
        | 0x3400..=0x4DBF      // CJK ext A
        | 0x4E00..=0x9FFF      // CJK unified
        | 0xA000..=0xA4CF      // Yi
        | 0xAC00..=0xD7A3      // Hangul syllables
        | 0xF900..=0xFAFF      // CJK compat
        | 0xFE30..=0xFE4F      // CJK compat forms
        | 0xFF00..=0xFF60      // full-width forms
        | 0xFFE0..=0xFFE6
        | 0x20000..=0x3FFFD)
}

fn char_width(ch: char) -> usize {
    if is_wide(ch) { 2 } else { 1 }
}

/// Display width in cells — what layout math should use instead of
/// `chars().count()` once CJK text is in play.
pub fn str_width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

/// Truncate a string to `width` display cells (leading '…' marks the cut) —
/// for right-aligned text that must not overflow leftward.
pub fn clip_str(s: &str, width: usize) -> String {
    if str_width(s) <= width {
        return s.to_string();
    }
    let mut out: Vec<char> = Vec::new();
    let mut used = 1; // the ellipsis
    for ch in s.chars().rev() {
        let w = char_width(ch);
        if used + w > width {
            break;
        }
        out.push(ch);
        used += w;
    }
    let mut r = String::from("…");
    r.extend(out.iter().rev());
    r
}

pub struct Grid {
    pub cols: usize,
    pub rows: usize,
    pub cells: Vec<Cell>,
    blank: Cell,
}

const BLOCKS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

impl Grid {
    pub fn new(cols: usize, rows: usize, fg: u32) -> Self {
        let blank = Cell { ch: ' ', fg };
        Grid { cols, rows, cells: vec![blank; cols * rows], blank }
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.cols = cols;
        self.rows = rows;
        self.cells = vec![self.blank; cols * rows];
    }

    pub fn clear(&mut self) {
        for c in self.cells.iter_mut() {
            *c = self.blank;
        }
    }

    #[inline]
    pub fn put(&mut self, x: usize, y: usize, ch: char, fg: u32) {
        if x < self.cols && y < self.rows {
            self.cells[y * self.cols + x] = Cell { ch, fg };
        }
    }

    /// Draw `s` starting at (x, y); returns the column just past the end.
    /// Full-width characters take two cells: the glyph plus a CONT marker.
    pub fn text(&mut self, x: usize, y: usize, s: &str, fg: u32) -> usize {
        let mut cx = x;
        for ch in s.chars() {
            if cx >= self.cols {
                break;
            }
            if is_wide(ch) {
                if cx + 1 >= self.cols {
                    break;
                }
                self.put(cx, y, ch, fg);
                self.put(cx + 1, y, CONT, fg);
                cx += 2;
            } else {
                self.put(cx, y, ch, fg);
                cx += 1;
            }
        }
        cx
    }

    /// Right-align `s` so it ends at column `right` (exclusive).
    pub fn text_right(&mut self, right: usize, y: usize, s: &str, fg: u32) {
        let len = str_width(s);
        let x = right.saturating_sub(len);
        self.text(x, y, s, fg);
    }

    /// Truncate to `width` display cells, marking the cut with a middle dot.
    pub fn text_clip(&mut self, x: usize, y: usize, s: &str, width: usize, fg: u32) {
        if width == 0 {
            return;
        }
        if str_width(s) <= width {
            self.text(x, y, s, fg);
        } else {
            let mut cut = String::new();
            let mut used = 0;
            for ch in s.chars() {
                let w = if is_wide(ch) { 2 } else { 1 };
                if used + w > width.saturating_sub(1) {
                    break;
                }
                cut.push(ch);
                used += w;
            }
            let end = self.text(x, y, &cut, fg);
            self.put(end, y, '·', fg);
        }
    }

    /// Like `text_clip`, but bounded by an absolute right column (exclusive).
    /// Returns the column just past what was drawn.
    pub fn text_clip_end(&mut self, x: usize, y: usize, s: &str, right: usize, fg: u32) -> usize {
        if right <= x {
            return x;
        }
        let width = right - x;
        if str_width(s) <= width {
            self.text(x, y, s, fg)
        } else {
            self.text_clip(x, y, s, width, fg);
            right
        }
    }

    pub fn hline(&mut self, x: usize, y: usize, len: usize, ch: char, fg: u32) {
        for i in 0..len {
            self.put(x + i, y, ch, fg);
        }
    }

    pub fn vline(&mut self, x: usize, y: usize, len: usize, ch: char, fg: u32) {
        for i in 0..len {
            self.put(x, y + i, ch, fg);
        }
    }

    /// A titled box: ┌─[ title ]────────┐
    pub fn panel(&mut self, x: usize, y: usize, w: usize, h: usize, title: &str, frame: u32, title_fg: u32) {
        if w < 4 || h < 2 {
            return;
        }
        self.put(x, y, '┌', frame);
        self.put(x + w - 1, y, '┐', frame);
        self.put(x, y + h - 1, '└', frame);
        self.put(x + w - 1, y + h - 1, '┘', frame);
        self.hline(x + 1, y, w - 2, '─', frame);
        self.hline(x + 1, y + h - 1, w - 2, '─', frame);
        self.vline(x, y + 1, h - 2, '│', frame);
        self.vline(x + w - 1, y + 1, h - 2, '│', frame);

        if !title.is_empty() {
            let mut cx = x + 2;
            cx = self.text(cx, y, "┤ ", frame);
            cx = self.text(cx, y, title, title_fg);
            self.text(cx, y, " ├", frame);
        }
    }

    /// `[||||||||........]` — an honest terminal meter, not a rounded div.
    pub fn meter(&mut self, x: usize, y: usize, w: usize, frac: f64, fg: u32, empty_fg: u32) {
        if w < 3 {
            return;
        }
        let inner = w - 2;
        let filled = ((frac.clamp(0.0, 1.0)) * inner as f64).round() as usize;
        self.put(x, y, '[', empty_fg);
        for i in 0..inner {
            let (ch, c) = if i < filled { ('|', fg) } else { ('.', empty_fg) };
            self.put(x + 1 + i, y, ch, c);
        }
        self.put(x + w - 1, y, ']', empty_fg);
    }

    /// One-row sparkline from block glyphs, scaled to the series maximum.
    pub fn spark(&mut self, x: usize, y: usize, data: &[f64], fg: u32, zero_fg: u32) {
        let max = data.iter().cloned().fold(0.0_f64, f64::max);
        for (i, v) in data.iter().enumerate() {
            if *v <= 0.0 {
                self.put(x + i, y, '·', zero_fg);
                continue;
            }
            let lvl = if max > 0.0 {
                ((v / max) * 8.0).ceil().clamp(1.0, 8.0) as usize
            } else {
                1
            };
            self.put(x + i, y, BLOCKS[lvl], fg);
        }
    }

    /// Fill a rectangle with a shade glyph — used for the CRT texture bands.
    pub fn shade(&mut self, x: usize, y: usize, w: usize, h: usize, ch: char, fg: u32) {
        for r in 0..h {
            self.hline(x, y + r, w, ch, fg);
        }
    }

    /// Multi-row filled area graph: the last `w` samples of `data`, scaled to
    /// `max`, drawn bottom-up with eighth-block tips. The tall sibling of
    /// `spark` — history you can actually read levels off.
    pub fn graph(&mut self, x: usize, y: usize, w: usize, h: usize, data: &[f64], max: f64, fg: u32, dim: u32) {
        if h == 0 || w == 0 {
            return;
        }
        let n = data.len().min(w);
        let start = data.len() - n;
        for (i, v) in data[start..].iter().enumerate() {
            let cx = x + i;
            let frac = if max > 0.0 { (v / max).clamp(0.0, 1.0) } else { 0.0 };
            let eighths = (frac * (h * 8) as f64).round() as usize;
            if eighths == 0 {
                self.put(cx, y + h - 1, '·', dim);
                continue;
            }
            let full = eighths / 8;
            let rem = eighths % 8;
            for r in 0..full {
                self.put(cx, y + h - 1 - r, '█', fg);
            }
            if rem > 0 && full < h {
                self.put(cx, y + h - 1 - full, BLOCKS[rem], fg);
            }
        }
    }
}
