use ratatui::prelude::*;
use ratatui::widgets::*;

use super::constellation_counts;
use crate::app::App;
use crate::constants::{gnss_color, gnss_letter};

/// Three-letter constellation names for the legend.
fn legend_name(gnssid: u8) -> &'static str {
    match gnssid {
        0 => "GPS",
        1 => "SBS",
        2 => "GAL",
        3 => "BDS",
        4 => "IMS",
        5 => "QZS",
        6 => "GLO",
        7 => "IRN",
        _ => "???",
    }
}

/// Draws directly into the frame buffer within `area`, using coordinates
/// relative to the area's top-left corner. Writes outside the area are
/// ignored.
struct Canvas<'a> {
    buf: &'a mut Buffer,
    area: Rect,
}

impl Canvas<'_> {
    fn cell(&mut self, x: i32, y: i32) -> Option<&mut ratatui::buffer::Cell> {
        if x < 0 || y < 0 || x >= self.area.width as i32 || y >= self.area.height as i32 {
            return None;
        }
        self.buf
            .cell_mut((self.area.x + x as u16, self.area.y + y as u16))
    }

    fn is_blank(&mut self, x: i32, y: i32) -> bool {
        self.cell(x, y).is_some_and(|c| c.symbol() == " ")
    }

    /// Blank, or only ring/crosshair grid that a satellite label may cover.
    fn is_free(&mut self, x: i32, y: i32) -> bool {
        self.cell(x, y).is_some_and(|c| {
            matches!(c.symbol(), " ")
                || (c.fg == Color::DarkGray && matches!(c.symbol(), "." | "-" | "|"))
        })
    }

    fn put(&mut self, x: i32, y: i32, ch: char, style: Style) {
        if let Some(cell) = self.cell(x, y) {
            cell.set_char(ch).set_style(style);
        }
    }

    fn put_if_blank(&mut self, x: i32, y: i32, ch: char, style: Style) {
        if self.is_blank(x, y) {
            self.put(x, y, ch, style);
        }
    }
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered().title(" Sky ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    draw_plot(f.buffer_mut(), inner, app);

    super::mark_data_state(f, area, app);
}

fn draw_plot(buf: &mut Buffer, inner: Rect, app: &App) {
    let w = inner.width as usize;
    let h = inner.height as usize;
    if w < 8 || h < 4 {
        return;
    }
    let mut c = Canvas { buf, area: inner };

    let grid = Style::default().fg(Color::DarkGray);
    let emphasis = Style::default().bold();

    let cx = (w / 2) as i32;
    let cy = (h / 2) as i32;
    // Separate X/Y radii for aspect ratio (terminal chars are ~2:1)
    // Reserve space for cardinal labels if there's enough room
    let label_margin_x: i32 = if w > 20 { 3 } else { 1 };
    let label_margin_y: i32 = if h > 10 { 1 } else { 0 };
    let max_rx = (cx - label_margin_x).max(2);
    let max_ry = (cy - label_margin_y).max(2);

    // Draw concentric elevation rings at 0°, 30°, 60°
    for elev_deg in [0, 30, 60] {
        let r_frac = (90 - elev_deg) as f64 / 90.0;
        let rx = (r_frac * max_rx as f64) as i32;
        let ry = (r_frac * max_ry as f64) as i32;
        if rx < 1 || ry < 1 {
            continue;
        }
        for angle_deg in (0..360).step_by(2) {
            let rad = (angle_deg as f64).to_radians();
            let x = cx + (rx as f64 * rad.sin()) as i32;
            let y = cy - (ry as f64 * rad.cos()) as i32;
            c.put_if_blank(x, y, '.', grid);
        }
    }

    // Draw crosshairs
    for x in (cx - max_rx)..=(cx + max_rx) {
        c.put_if_blank(x, cy, '-', grid);
    }
    for y in (cy - max_ry)..=(cy + max_ry) {
        c.put_if_blank(cx, y, '|', grid);
    }

    // Center — zenith marker
    c.put(cx, cy, '+', Style::default());

    // Cardinal direction labels (outside the rings)
    for (ch, lx, ly) in [
        ('N', cx, cy - max_ry - 1),
        ('S', cx, cy + max_ry + 1),
        ('E', cx + max_rx + 2, cy),
        ('W', cx - max_rx - 2, cy),
    ] {
        c.put(lx, ly, ch, emphasis);
    }

    // Elevation labels on the horizontal axis
    for (elev_deg, label) in [(30, "30°"), (60, "60°")] {
        let r_frac = (90 - elev_deg) as f64 / 90.0;
        let lx = cx + (r_frac * max_rx as f64) as i32 + 1;
        let label_y = (cy - 1).max(0);
        for (i, ch) in label.chars().enumerate() {
            c.put_if_blank(lx + i as i32, label_y, ch, grid);
        }
    }

    // Plot satellites. The marker shape encodes used (#) vs. visible (o);
    // the label is prefixed with a constellation letter (G/R/E/C/J/S...) so
    // constellation does not depend on color alone.
    for sat in &app.gps_data.satellites {
        if !sat.elevation.is_finite() || !sat.azimuth.is_finite() || sat.elevation < 0.0 {
            continue;
        }

        let r_frac = ((90.0 - sat.elevation) / 90.0).max(0.0);
        let az_rad = sat.azimuth.to_radians();
        let sx = cx + (r_frac * max_rx as f64 * az_rad.sin()) as i32;
        let sy = cy - (r_frac * max_ry as f64 * az_rad.cos()) as i32;

        if sx < 0 || sx as usize >= w || sy < 0 || sy as usize >= h {
            continue;
        }

        let color = gnss_color(sat.gnssid);
        let style = Style::default().fg(color);
        if sat.used {
            c.put(sx, sy, '#', style.bold());
        } else {
            c.put(sx, sy, 'o', style);
        }

        // Labels may cover grid lines (so satellites on an axis still get
        // one) but never another satellite's marker or label.
        let label = format!("{}{}", gnss_letter(sat.gnssid), sat.svid);
        for (i, ch) in label.chars().enumerate() {
            let lx = sx + 1 + i as i32;
            if (lx as usize) >= w.saturating_sub(1) || !c.is_free(lx, sy) {
                break; // truncate rather than leave a gap in the label
            }
            c.put(lx, sy, ch, style);
        }
    }

    draw_legend(&mut c, app, w);
}

/// Compact legend in the top-left corner (outside the horizon ring), one
/// constellation per row. Drawn only into free cells; stops at the first
/// row that doesn't fit.
fn draw_legend(c: &mut Canvas, app: &App, w: usize) {
    if w < 30 {
        return;
    }
    let counts = constellation_counts(&app.gps_data.satellites);
    let mut row = 0i32;
    let mut text = [' '; 6];
    let present = counts
        .iter()
        .enumerate()
        .filter(|(_, (visible, _))| *visible > 0)
        .map(|(id, _)| id as u8);

    for gnssid in present {
        text[0] = gnss_letter(gnssid);
        for (slot, ch) in text[2..5].iter_mut().zip(legend_name(gnssid).chars()) {
            *slot = ch;
        }
        if !legend_row(c, row, &text[..5], Style::default().fg(gnss_color(gnssid))) {
            return;
        }
        row += 1;
    }
    if row > 0 {
        let used_key = ['#', ' ', 'u', 's', 'e', 'd'];
        legend_row(c, row, &used_key, Style::default().fg(Color::DarkGray));
    }
}

fn legend_row(c: &mut Canvas, row: i32, text: &[char], style: Style) -> bool {
    // Require one free column of padding to the right as well.
    let fits = (0..=text.len() as i32).all(|x| c.is_blank(x, row));
    if fits {
        for (x, ch) in text.iter().enumerate() {
            c.put(x as i32, row, *ch, style);
        }
    }
    fits
}
