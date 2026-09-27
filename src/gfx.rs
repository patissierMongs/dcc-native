//! Direct2D + DirectWrite rendering of the character grid.
//!
//! An `ID2D1HwndRenderTarget` is the simplest hardware-accelerated path on
//! Windows: no swap chain, no device management, and it repaints a full screen
//! of text in well under a millisecond. DPI is pinned to 96 so one DIP is one
//! physical pixel and the grid stays crisp on any monitor.
//!
//! The same painter also drives an offscreen WIC target, so the exact frame
//! that goes to the desktop can be written out as a PNG — which is how the
//! layout gets checked without disturbing whatever is on screen.

use std::collections::HashMap;

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::*;

use crate::grid::Grid;

fn color_of(rgb: u32, alpha: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: ((rgb >> 16) & 0xFF) as f32 / 255.0,
        g: ((rgb >> 8) & 0xFF) as f32 / 255.0,
        b: (rgb & 0xFF) as f32 / 255.0,
        a: alpha,
    }
}

/// Scale a colour towards black — the night-dim knob for OLED and eyes.
fn dim_rgb(rgb: u32, f: f32) -> u32 {
    if f >= 0.999 {
        return rgb;
    }
    let r = (((rgb >> 16) & 0xFF) as f32 * f) as u32;
    let g = (((rgb >> 8) & 0xFF) as f32 * f) as u32;
    let b = ((rgb & 0xFF) as f32 * f) as u32;
    (r << 16) | (g << 8) | b
}

/// UTF-16 for DrawText, which takes a counted slice — no terminator wanted.
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// UTF-16 for PCWSTR, which is a bare pointer: NUL-terminated, and the buffer
/// must outlive the call that reads it.
fn wide_nul(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Owns the font and the per-colour brush cache, and knows how to stamp a
/// `Grid` onto any Direct2D render target.
pub struct Painter {
    format: IDWriteTextFormat,
    brushes: HashMap<u32, ID2D1SolidColorBrush>,
    brush_owner: *mut std::ffi::c_void,
    pub cell_w: f32,
    pub cell_h: f32,
    pub pt: f32,
}

fn make_format(dwrite: &IDWriteFactory, font: &str, pt: f32) -> Result<IDWriteTextFormat> {
    unsafe {
        let font_w = wide_nul(font);
        let locale_w = wide_nul("en-us");
        let f = dwrite.CreateTextFormat(
            PCWSTR(font_w.as_ptr()),
            None,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            pt,
            PCWSTR(locale_w.as_ptr()),
        )?;
        f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        Ok(f)
    }
}

/// Advance width and line height of one cell, measured from a run of glyphs.
fn measure(dwrite: &IDWriteFactory, format: &IDWriteTextFormat) -> Result<(f32, f32)> {
    unsafe {
        let probe = wide("MMMMMMMMMMMMMMMMMMMM");
        let layout = dwrite.CreateTextLayout(&probe, format, 100000.0, 1000.0)?;
        let mut m = DWRITE_TEXT_METRICS::default();
        layout.GetMetrics(&mut m)?;
        Ok((m.widthIncludingTrailingWhitespace / probe.len() as f32, m.height))
    }
}

impl Painter {
    /// Pick a font size so the surface comes out near `target_cols` columns.
    /// A 5120px-wide monitor and a 1920px one then show the same layout at
    /// the same apparent density instead of one being unreadable.
    pub fn fitted(dwrite: &IDWriteFactory, font: &str, width_px: f32, target_cols: f32) -> Result<Self> {
        let probe_pt = 16.0f32;
        let probe = make_format(dwrite, font, probe_pt)?;
        let (w0, _) = measure(dwrite, &probe)?;
        let want_cell = width_px / target_cols;
        let pt = (probe_pt * (want_cell / w0)).round().clamp(8.0, 96.0);

        let format = make_format(dwrite, font, pt)?;
        let (cell_w, cell_h) = measure(dwrite, &format)?;
        Ok(Painter {
            format,
            brushes: HashMap::new(),
            brush_owner: std::ptr::null_mut(),
            cell_w,
            cell_h,
            pt,
        })
    }

    pub fn cols_rows(&self, size: (u32, u32)) -> (usize, usize) {
        (
            (size.0 as f32 / self.cell_w).floor().max(1.0) as usize,
            (size.1 as f32 / self.cell_h).floor().max(1.0) as usize,
        )
    }

    fn brush(&mut self, target: &ID2D1RenderTarget, rgb: u32) -> Result<ID2D1SolidColorBrush> {
        let owner = target.as_raw();
        if owner != self.brush_owner {
            self.brushes.clear();
            self.brush_owner = owner;
        }
        if let Some(b) = self.brushes.get(&rgb) {
            return Ok(b.clone());
        }
        let b = unsafe { target.CreateSolidColorBrush(&color_of(rgb, 1.0), None)? };
        self.brushes.insert(rgb, b.clone());
        Ok(b)
    }

    /// Stamp the grid onto `target`. Cells are coalesced into runs of equal
    /// colour, so a full screen costs a few hundred DrawText calls rather than
    /// one per character. `offset` drifts everything by whole pixels (OLED
    /// care) and `dim` scales every colour towards black (night hours).
    pub fn paint(
        &mut self,
        target: &ID2D1RenderTarget,
        grid: &Grid,
        bg: u32,
        width_px: f32,
        offset: (f32, f32),
        dim: f32,
    ) -> Result<()> {
        let mut runs: Vec<(usize, usize, u32, String)> = Vec::new();
        for y in 0..grid.rows {
            let row = &grid.cells[y * grid.cols..(y + 1) * grid.cols];
            let is_wide_at = |i: usize| i + 1 < row.len() && row[i + 1].ch == crate::grid::CONT;
            // Braille glyphs come from a fallback font whose advance may not
            // match the cell, so each one is pinned to its own cell too.
            let is_braille = |ch: char| (0x2800..=0x28FF).contains(&(ch as u32));
            let mut x = 0usize;
            while x < row.len() {
                let ch = row[x].ch;
                if ch == ' ' || ch == crate::grid::CONT {
                    x += 1;
                    continue;
                }
                // A full-width glyph gets its own run pinned to its cell:
                // the fallback font's advance need not be exactly two cells,
                // so letting CJK text flow inside a long run would drift off
                // the grid.
                if is_wide_at(x) {
                    runs.push((x, y, row[x].fg, ch.to_string()));
                    x += 2;
                    continue;
                }
                if is_braille(ch) {
                    runs.push((x, y, row[x].fg, ch.to_string()));
                    x += 1;
                    continue;
                }
                let fg = row[x].fg;
                let start = x;
                let mut s = String::new();
                while x < row.len()
                    && row[x].fg == fg
                    && row[x].ch != ' '
                    && row[x].ch != crate::grid::CONT
                    && !is_wide_at(x)
                    && !is_braille(row[x].ch)
                {
                    s.push(row[x].ch);
                    x += 1;
                }
                runs.push((start, y, fg, s));
            }
        }

        unsafe {
            target.BeginDraw();
            target.Clear(Some(&color_of(dim_rgb(bg, dim), 1.0)));
            for (x, y, fg, s) in &runs {
                let brush = self.brush(target, dim_rgb(*fg, dim))?;
                let rect = D2D_RECT_F {
                    left: *x as f32 * self.cell_w + offset.0,
                    top: *y as f32 * self.cell_h + offset.1,
                    right: width_px + self.cell_w + offset.0,
                    bottom: (*y as f32 + 2.0) * self.cell_h + offset.1,
                };
                let text = wide(s);
                target.DrawText(
                    &text,
                    &self.format,
                    &rect,
                    &brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                    DWRITE_MEASURING_MODE_NATURAL,
                );
            }
            target.EndDraw(None, None)
        }
    }
}

pub struct Renderer {
    d2d: ID2D1Factory,
    /// Kept for the renderer's lifetime alongside the formats it made.
    _dwrite: IDWriteFactory,
    pub painter: Painter,
    target: Option<ID2D1HwndRenderTarget>,
    hwnd: HWND,
    size: (u32, u32),
    last_cells: Option<Vec<crate::grid::Cell>>,
    last_offset: (f32, f32),
    last_dim: f32,
}

fn rt_props() -> D2D1_RENDER_TARGET_PROPERTIES {
    rt_props_alpha(D2D1_ALPHA_MODE_IGNORE)
}

/// A WIC target backed by 32bppPBGRA rejects ALPHA_MODE_IGNORE, so the
/// offscreen path asks for premultiplied instead.
fn rt_props_alpha(alpha: D2D1_ALPHA_MODE) -> D2D1_RENDER_TARGET_PROPERTIES {
    D2D1_RENDER_TARGET_PROPERTIES {
        r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: alpha,
        },
        dpiX: 96.0, // 1 DIP == 1 physical pixel
        dpiY: 96.0,
        usage: D2D1_RENDER_TARGET_USAGE_NONE,
        minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
    }
}

pub fn dwrite_factory() -> Result<IDWriteFactory> {
    unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED) }
}

impl Renderer {
    pub fn new(hwnd: HWND, size: (u32, u32), font: &str, target_cols: f32) -> Result<Self> {
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dwrite = dwrite_factory()?;
            let painter = Painter::fitted(&dwrite, font, size.0 as f32, target_cols)?;
            let mut me = Renderer {
                d2d,
                _dwrite: dwrite,
                painter,
                target: None,
                hwnd,
                size,
                last_cells: None,
                last_offset: (0.0, 0.0),
                last_dim: 1.0,
            };
            me.recreate()?;
            Ok(me)
        }
    }

    fn recreate(&mut self) -> Result<()> {
        unsafe {
            let hwnd_props = D2D1_HWND_RENDER_TARGET_PROPERTIES {
                hwnd: self.hwnd,
                pixelSize: D2D_SIZE_U { width: self.size.0, height: self.size.1 },
                presentOptions: D2D1_PRESENT_OPTIONS_NONE,
            };
            let target = self.d2d.CreateHwndRenderTarget(&rt_props(), &hwnd_props)?;
            target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_CLEARTYPE);
            self.target = Some(target);
            Ok(())
        }
    }

    pub fn cols_rows(&self) -> (usize, usize) {
        self.painter.cols_rows(self.size)
    }

    /// Repaint only when something actually changed — cell content, the OLED
    /// drift offset, or the dim level.
    pub fn draw_if_changed(&mut self, grid: &Grid, bg: u32, offset: (f32, f32), dim: f32) -> Result<bool> {
        if self.target.is_some()
            && self.last_cells.as_deref() == Some(&grid.cells[..])
            && self.last_offset == offset
            && self.last_dim == dim
        {
            return Ok(false);
        }
        self.draw(grid, bg, offset, dim)?;
        self.last_cells = Some(grid.cells.clone());
        self.last_offset = offset;
        self.last_dim = dim;
        Ok(true)
    }

    pub fn draw(&mut self, grid: &Grid, bg: u32, offset: (f32, f32), dim: f32) -> Result<()> {
        if self.target.is_none() {
            self.recreate()?;
        }
        let target = self.target.clone().unwrap();
        let rt: ID2D1RenderTarget = target.cast()?;
        match self.painter.paint(&rt, grid, bg, self.size.0 as f32, offset, dim) {
            Ok(()) => Ok(()),
            Err(e) if e.code() == D2DERR_RECREATE_TARGET => {
                self.target = None;
                Ok(())
            }
            Err(e) => Err(e),
        }
    }
}

/// Render one frame to a PNG through WIC. Same painter, same grid, same
/// glyphs — so what lands in the file is what lands on the desktop.
pub fn render_png(path: &str, size: (u32, u32), font: &str, target_cols: f32, bg: u32,
                  build: &mut dyn FnMut(usize, usize) -> Grid) -> Result<()> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
        let dwrite = dwrite_factory()?;
        let mut painter = Painter::fitted(&dwrite, font, size.0 as f32, target_cols)?;
        let (cols, rows) = painter.cols_rows(size);
        let grid = build(cols, rows);

        let wic: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let bitmap = wic.CreateBitmap(
            size.0,
            size.1,
            &GUID_WICPixelFormat32bppPBGRA,
            WICBitmapCacheOnLoad,
        )?;

        let target =
            d2d.CreateWicBitmapRenderTarget(&bitmap, &rt_props_alpha(D2D1_ALPHA_MODE_PREMULTIPLIED))?;
        target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        let rt: ID2D1RenderTarget = target.cast()?;
        painter.paint(&rt, &grid, bg, size.0 as f32, (0.0, 0.0), 1.0)?;

        let stream = wic.CreateStream()?;
        let path_w = wide_nul(path);
        stream.InitializeFromFilename(PCWSTR(path_w.as_ptr()), GENERIC_WRITE.0)?;

        let encoder = wic.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;

        let mut frame: Option<IWICBitmapFrameEncode> = None;
        let mut opts: Option<windows::Win32::System::Com::StructuredStorage::IPropertyBag2> = None;
        encoder.CreateNewFrame(&mut frame, &mut opts)?;
        let frame = frame.unwrap();
        frame.Initialize(None)?;
        frame.SetSize(size.0, size.1)?;
        let mut fmt = GUID_WICPixelFormat32bppPBGRA;
        frame.SetPixelFormat(&mut fmt)?;
        frame.WriteSource(&bitmap, std::ptr::null())?;
        frame.Commit()?;
        encoder.Commit()?;

        println!("[dcc] wrote {} ({}x{} px, {}x{} cells, {:.0}pt)", path, size.0, size.1, cols, rows, painter.pt);
        Ok(())
    }
}
