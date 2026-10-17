use crate::app::App;
use crate::ui::*;

/// Top row height (including borders) that shows every line of Position
/// (6), Fix (7), Velocity (5) and Hold (7).
const TOP_FULL: u16 = 9;
/// Bottom row height showing the core lines: Errors EPH..SEP, all of Time,
/// Device path/driver/baud/cycle/subtype.
const BOTTOM_CORE: u16 = 7;
/// Bottom row height showing every Errors line.
const BOTTOM_FULL: u16 = error_panel::LINES + 2;
/// Text rows are not squeezed below this (3 lines of content).
const ROW_FLOOR: u16 = 5;
/// Smallest sky plot that still draws (inner height 4).
const SKY_TINY: u16 = 6;
/// Small but readable sky plot (inner height 6); at 80x24 the bottom row
/// gives way first (3 lines each) rather than squashing the plot further.
const SKY_SMALL: u16 = 8;
/// Sky plot height to reach before the bottom row grows past its core.
const SKY_COMFORT: u16 = 12;

pub fn render_dashboard(f: &mut Frame, area: Rect, app: &App) {
    let (top, bottom) = row_heights(area.height);

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
    // Holding position implies standing still, so the hold statistics take
    // the Velocity panel's place while hold is active.
    if app.position_hold.is_some() {
        position::render_hold(f, row1[2], app);
    } else {
        velocity::render(f, row1[2], app);
    }
    sky_plot::render(f, row2[0], app);
    signal_chart::render(f, row2[1], app);
    error_panel::render(f, row3[0], app);
    device_panel::render(f, row3[1], app);
    time_panel::render(f, row3[2], app, false);
}

/// Heights for the top and bottom rows; the middle (sky/signal) row gets
/// the rest. Space is handed out in priority order, each step raising one
/// row to a target height:
///
/// 1. minimal top, drawable sky, minimal bottom;
/// 2. full top row, then a slightly larger sky, then the bottom core;
/// 3. a comfortable sky, then the full bottom row; the sky takes the rest.
///
/// Panels that end up short drop their least important lines and show
/// `…` in their title.
pub fn row_heights(total: u16) -> (u16, u16) {
    let (mut top, mut sky, mut bottom) = (0u16, 0u16, 0u16);
    let mut left = total;
    let mut grow = |row: &mut u16, target: u16| {
        let add = target.saturating_sub(*row).min(left);
        *row += add;
        left -= add;
    };

    grow(&mut top, ROW_FLOOR);
    grow(&mut sky, SKY_TINY);
    grow(&mut bottom, ROW_FLOOR);
    grow(&mut top, TOP_FULL);
    grow(&mut sky, SKY_SMALL);
    grow(&mut bottom, BOTTOM_CORE);
    grow(&mut sky, SKY_COMFORT);
    grow(&mut bottom, BOTTOM_FULL);
    (top, bottom)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sky(total: u16) -> u16 {
        let (t, b) = row_heights(total);
        total - t - b
    }

    #[test]
    fn full_rows_when_tall() {
        // 120x40: content area is 38 rows.
        assert_eq!(row_heights(38), (TOP_FULL, BOTTOM_FULL));
        assert_eq!(sky(38), 18);
    }

    #[test]
    fn at_80x24_top_full_sky_small_bottom_floor() {
        // 24 rows minus tab bar and status bar.
        assert_eq!(row_heights(22), (TOP_FULL, ROW_FLOOR));
        assert_eq!(sky(22), SKY_SMALL);
    }

    #[test]
    fn at_100x30_bottom_core_sky_comfort() {
        assert_eq!(row_heights(28), (TOP_FULL, BOTTOM_CORE));
        assert_eq!(sky(28), SKY_COMFORT);
    }

    #[test]
    fn minimum_terminal_keeps_drawable_sky() {
        // MIN_HEIGHT 16 minus tab and status bar.
        let (top, bottom) = row_heights(14);
        assert_eq!(top, ROW_FLOOR);
        assert_eq!(sky(14), SKY_TINY);
        assert_eq!(bottom, 3);
    }

    #[test]
    fn monotonic_and_never_overflows() {
        let mut prev = (0, 0, 0);
        for total in 0..80 {
            let (top, bottom) = row_heights(total);
            assert!(top + bottom <= total, "{total}");
            let cur = (top, sky(total), bottom);
            assert!(cur.0 >= prev.0 && cur.1 >= prev.1 && cur.2 >= prev.2);
            prev = cur;
        }
    }
}
