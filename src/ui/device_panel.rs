use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::App;

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered().title(" Device ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let dev = &app.gps_data.device;
    let ver = &app.gps_data.version;

    let lines = vec![
        Line::from(format!("Path:    {}", if dev.path.is_empty() { "---" } else { &dev.path })),
        Line::from(format!("Driver:  {}", if dev.driver.is_empty() { "---" } else { &dev.driver })),
        Line::from(format!("Subtype: {}", if dev.subtype.is_empty() { "---" } else { &dev.subtype })),
        Line::from(format!("Baud:    {}", if dev.bps > 0 { dev.bps.to_string() } else { "---".to_string() })),
        Line::from(format!("Cycle:   {:.1}s", dev.cycle)),
        Line::from(format!("gpsd:    {}", if ver.release.is_empty() { "---" } else { &ver.release })),
    ];

    f.render_widget(Paragraph::new(lines), inner);
}
