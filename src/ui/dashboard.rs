use crate::app::App;
use crate::ui::*;

/// Rows (including borders) the top row wants: Fix shows Mode/Status/Sats
/// + 4 DOPs; Position grows by 3 when hold CEP is shown.
const TOP_ROW: u16 = 9;
const TOP_ROW_WITH_HOLD: u16 = 10;
/// Bottom row: Errors panel has 7 lines.
const BOTTOM_ROW: u16 = 9;
/// Smallest useful sky plot (inner height 6).
const SKY_MIN: u16 = 8;
/// Panels are never squeezed below this before the sky plot gives way.
const ROW_FLOOR: u16 = 5;

pub fn render_dashboard(f: &mut Frame, area: Rect, app: &App) {
    let top_want = if app.position_hold.is_some() {
        TOP_ROW_WITH_HOLD
    } else {
        TOP_ROW
    };
    let (top, bottom) = row_heights(area.height, top_want, BOTTOM_ROW);

    let rows = Layout::vertical([
        Constraint::Length(top),
        Constraint::Min(0),
        Constraint::Length(bottom),
    ])
    .split(area);

    let thirds = [
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
    ];
    let row1 = Layout::horizontal(thirds).split(rows[0]);
    let row2 =
        Layout::horizontal([Constraint::Ratio(2, 3), Constraint::Ratio(1, 3)]).split(rows[1]);
    let row3 = Layout::horizontal(thirds).split(rows[2]);

    position::render(f, row1[0], app);
    fix::render(f, row1[1], app);
    velocity::render(f, row1[2], app);
    sky_plot::render(f, row2[0], app);
    signal_chart::render(f, row2[1], app);
    error_panel::render(f, row3[0], app);
    device_panel::render(f, row3[1], app);
    time_panel::render(f, row3[2], app, false);
}

/// Heights for the top and bottom rows; the middle (sky/signal) row gets
/// the rest. Text rows get their full content height when possible; when
/// space is short the bottom row shrinks first, then the top row, down to
/// a floor, so the sky plot keeps a usable minimum.
fn row_heights(total: u16, top_want: u16, bottom_want: u16) -> (u16, u16) {
    let mut top = top_want;
    let mut bottom = bottom_want;
    let mut deficit = (top + bottom + SKY_MIN).saturating_sub(total);

    let cut = deficit.min(bottom.saturating_sub(ROW_FLOOR));
    bottom -= cut;
    deficit -= cut;

    let cut = deficit.min(top.saturating_sub(ROW_FLOOR));
    top -= cut;
    deficit -= cut;

    if deficit > 0 {
        // Very short: split proportionally, sky gets the remainder.
        top = total * 3 / 10;
        bottom = total * 2 / 10;
    }
    (top, bottom)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_rows_when_tall() {
        assert_eq!(row_heights(40, TOP_ROW, BOTTOM_ROW), (9, 9));
    }

    #[test]
    fn bottom_shrinks_first_at_80x24() {
        // 24 rows minus tab bar and status bar.
        let (top, bottom) = row_heights(22, TOP_ROW, BOTTOM_ROW);
        assert_eq!(top, TOP_ROW);
        assert_eq!(bottom, 5);
        assert!(22 - top - bottom >= SKY_MIN);
    }

    #[test]
    fn tiny_does_not_overflow() {
        for total in 0..30 {
            let (top, bottom) = row_heights(total, TOP_ROW_WITH_HOLD, BOTTOM_ROW);
            assert!(top + bottom <= total, "{total}");
        }
    }
}
