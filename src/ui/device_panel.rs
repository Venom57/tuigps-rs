use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::App;
use crate::formatting::panel_title;

fn or_dash(s: &str) -> &str {
    if s.is_empty() { "---" } else { s }
}

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let dev = &app.gps_data.device;
    let ver = &app.gps_data.version;

    // Most important first: lower lines are dropped when the panel is short.
    let lines = vec![
        Line::from(format!("Path:    {}", or_dash(&dev.path))),
        Line::from(format!("Driver:  {}", or_dash(&dev.driver))),
        Line::from(format!(
            "Baud:    {}",
            if dev.bps > 0 {
                dev.bps.to_string()
            } else {
                "---".to_string()
            }
        )),
        Line::from(format!(
            "Cycle:   {}",
            if dev.cycle.is_finite() && dev.cycle > 0.0 {
                format!("{:.2}s", dev.cycle)
            } else {
                "---".to_string()
            }
        )),
        Line::from(format!("Subtype: {}", or_dash(&dev.subtype))),
        Line::from(format!("gpsd:    {}", or_dash(&ver.release))),
    ];

    let block = Block::bordered().title(panel_title("Device", lines.len(), area.height));
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(Paragraph::new(lines), inner);
}
