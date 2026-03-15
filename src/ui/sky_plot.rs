use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::App;
use crate::constants::gnss_color;

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered().title(" Sky ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.width < 4 || inner.height < 4 {
        return;
    }

    let w = inner.width as f64;
    let h = inner.height as f64;
    let cx = w / 2.0;
    let cy = h / 2.0;
    let radius = cx.min(cy * 2.0) - 1.0;

    let mut canvas: Vec<Vec<(char, Style)>> =
        vec![vec![(' ', Style::default()); inner.width as usize]; inner.height as usize];

    // Draw elevation rings
    draw_ring(&mut canvas, cx, cy, radius, 1.0, '·');
    draw_ring(&mut canvas, cx, cy, radius, 2.0 / 3.0, '·');
    draw_ring(&mut canvas, cx, cy, radius, 1.0 / 3.0, '·');

    // Draw crosshairs
    for x in 0..inner.width as usize {
        plot_char_raw(&mut canvas, x, cy as usize, '·', Style::default().fg(Color::DarkGray));
    }
    for y in 0..inner.height as usize {
        plot_char_raw(&mut canvas, cx as usize, y, '·', Style::default().fg(Color::DarkGray));
    }

    // Cardinal labels
    plot_char(&mut canvas, cx, 0.0, 'N', Style::default().fg(Color::White).bold());
    plot_char(&mut canvas, cx, h - 1.0, 'S', Style::default().fg(Color::White).bold());
    plot_char(&mut canvas, 0.0, cy, 'W', Style::default().fg(Color::White).bold());
    plot_char(&mut canvas, w - 1.0, cy, 'E', Style::default().fg(Color::White).bold());

    // Plot satellites
    for sat in &app.gps_data.satellites {
        if !sat.elevation.is_finite() || !sat.azimuth.is_finite() {
            continue;
        }
        let r = (90.0 - sat.elevation) / 90.0 * radius;
        let az_rad = sat.azimuth.to_radians();
        let x = cx + r * az_rad.sin() / 2.0;
        let y = cy - r * az_rad.cos();

        let color = gnss_color(sat.gnssid);
        let (ch, style) = if sat.used {
            ('#', Style::default().fg(color).bold())
        } else {
            ('o', Style::default().fg(color))
        };
        plot_char(&mut canvas, x, y, ch, style);
    }

    // Render canvas
    for (row_idx, row) in canvas.iter().enumerate() {
        let spans: Vec<Span> = row
            .iter()
            .map(|(ch, style)| Span::styled(ch.to_string(), *style))
            .collect();
        let line = Line::from(spans);
        let y = inner.y + row_idx as u16;
        if y < inner.y + inner.height {
            f.render_widget(
                Paragraph::new(vec![line]),
                Rect::new(inner.x, y, inner.width, 1),
            );
        }
    }
}

fn plot_char(canvas: &mut [Vec<(char, Style)>], x: f64, y: f64, ch: char, style: Style) {
    let xi = x.round() as usize;
    let yi = y.round() as usize;
    plot_char_raw(canvas, xi, yi, ch, style);
}

fn plot_char_raw(canvas: &mut [Vec<(char, Style)>], x: usize, y: usize, ch: char, style: Style) {
    if y < canvas.len() && x < canvas[0].len() {
        canvas[y][x] = (ch, style);
    }
}

fn draw_ring(
    canvas: &mut [Vec<(char, Style)>],
    cx: f64,
    cy: f64,
    radius: f64,
    scale: f64,
    ch: char,
) {
    let r = radius * scale;
    let style = Style::default().fg(Color::DarkGray);
    let steps = (r * 4.0) as usize;
    if steps == 0 {
        return;
    }
    for i in 0..steps {
        let angle = 2.0 * std::f64::consts::PI * i as f64 / steps as f64;
        let x = cx + r * angle.cos() / 2.0;
        let y = cy + r * angle.sin();
        plot_char(canvas, x, y, ch, style);
    }
}
