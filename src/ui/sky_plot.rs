use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::App;
use crate::constants::gnss_color;

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered().title(" Sky ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let w = inner.width as usize;
    let h = inner.height as usize;
    if w < 10 || h < 6 {
        return;
    }

    // Character, color, and bold buffers
    let mut buf = vec![vec![' '; w]; h];
    let mut col = vec![vec![Color::White; w]; h];
    let mut bld = vec![vec![false; w]; h];

    let cx = w / 2;
    let cy = h / 2;
    // Separate X/Y radii for aspect ratio (terminal chars are ~2:1)
    let max_rx = (cx as i32 - 4).max(8) as usize;
    let max_ry = (cy as i32 - 2).max(4) as usize;

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
            let x = cx as i32 + (rx as f64 * rad.sin()) as i32;
            let y = cy as i32 - (ry as f64 * rad.cos()) as i32;
            if x >= 0 && (x as usize) < w && y >= 0 && (y as usize) < h {
                let (xu, yu) = (x as usize, y as usize);
                if buf[yu][xu] == ' ' {
                    buf[yu][xu] = '.';
                    col[yu][xu] = Color::DarkGray;
                }
            }
        }
    }

    // Draw crosshairs
    let rx_i = max_rx as i32;
    let ry_i = max_ry as i32;
    for x in (cx as i32 - rx_i)..=(cx as i32 + rx_i) {
        if x >= 0 && (x as usize) < w && buf[cy][x as usize] == ' ' {
            buf[cy][x as usize] = '-';
            col[cy][x as usize] = Color::DarkGray;
        }
    }
    for y in (cy as i32 - ry_i)..=(cy as i32 + ry_i) {
        if y >= 0 && (y as usize) < h && buf[y as usize][cx] == ' ' {
            buf[y as usize][cx] = '|';
            col[y as usize][cx] = Color::DarkGray;
        }
    }

    // Center — zenith marker
    buf[cy][cx] = '+';
    col[cy][cx] = Color::White;

    // Cardinal direction labels (outside the rings)
    let labels: &[(char, i32, i32)] = &[
        ('N', cx as i32, cy as i32 - ry_i - 1),
        ('S', cx as i32, cy as i32 + ry_i + 1),
        ('E', cx as i32 + rx_i + 2, cy as i32),
        ('W', cx as i32 - rx_i - 2, cy as i32),
    ];
    for &(ch, lx, ly) in labels {
        if lx >= 0 && (lx as usize) < w && ly >= 0 && (ly as usize) < h {
            buf[ly as usize][lx as usize] = ch;
            col[ly as usize][lx as usize] = Color::White;
            bld[ly as usize][lx as usize] = true;
        }
    }

    // Elevation labels on the horizontal axis
    for elev_deg in [30, 60] {
        let r_frac = (90 - elev_deg) as f64 / 90.0;
        let lx = cx + (r_frac * max_rx as f64) as usize + 1;
        let label = format!("{}°", elev_deg);
        let label_y = cy.saturating_sub(1);
        for (i, ch) in label.chars().enumerate() {
            let x = lx + i;
            if x < w && label_y < h && buf[label_y][x] == ' ' {
                buf[label_y][x] = ch;
                col[label_y][x] = Color::DarkGray;
            }
        }
    }

    // Plot satellites
    for sat in &app.gps_data.satellites {
        if !sat.elevation.is_finite() || !sat.azimuth.is_finite() || sat.elevation < 0.0 {
            continue;
        }

        let r_frac = (90.0 - sat.elevation) / 90.0;
        let az_rad = sat.azimuth.to_radians();
        let sx = cx as i32 + (r_frac * max_rx as f64 * az_rad.sin()) as i32;
        let sy = cy as i32 - (r_frac * max_ry as f64 * az_rad.cos()) as i32;

        if sx < 0 || sx as usize >= w || sy < 0 || sy as usize >= h {
            continue;
        }
        let (sxu, syu) = (sx as usize, sy as usize);

        let color = gnss_color(sat.gnssid);
        if sat.used {
            buf[syu][sxu] = '#';
            col[syu][sxu] = color;
            bld[syu][sxu] = true;
        } else {
            buf[syu][sxu] = 'o';
            col[syu][sxu] = color;
        }

        // Label: svid number next to marker
        let label = sat.svid.to_string();
        for (i, ch) in label.chars().enumerate() {
            let lx = sxu + 1 + i;
            if lx < w.saturating_sub(1) && buf[syu][lx] == ' ' {
                buf[syu][lx] = ch;
                col[syu][lx] = color;
            }
        }
    }

    // Render to frame
    for row_idx in 0..h {
        let spans: Vec<Span> = buf[row_idx]
            .iter()
            .enumerate()
            .map(|(x, ch)| {
                let mut style = Style::default().fg(col[row_idx][x]);
                if bld[row_idx][x] {
                    style = style.bold();
                }
                Span::styled(ch.to_string(), style)
            })
            .collect();
        let y = inner.y + row_idx as u16;
        if y < inner.y + inner.height {
            f.render_widget(
                Paragraph::new(vec![Line::from(spans)]),
                Rect::new(inner.x, y, inner.width, 1),
            );
        }
    }
}
