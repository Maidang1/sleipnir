//! Render the panel to stdout, exactly as the plugin would send it.
//!
//! Development aid: the widget tree is host-rendered, so this is the only way
//! to eyeball the image without launching the terminal.
use plugin_protocol::v2::{Widget, measure};
use sleipnir_plugin_disk3d::{View, render, scan::scan};

fn flatten(w: &Widget, out: &mut Vec<String>) {
    match w {
        Widget::Text { s, .. } => out.push(s.clone()),
        Widget::Badge { s, .. } => out.push(format!("[{s}]")),
        Widget::Btn { s, .. } => out.push(format!("<{s}>")),
        Widget::Bar { v } => {
            let n = (v * 20.0).round().max(0.0) as usize;
            out.push(format!("[{}{}]", "#".repeat(n.min(20)), "-".repeat(20 - n.min(20))));
        }
        Widget::Sep => out.push("─".repeat(60)),
        Widget::Col { children, .. } => children.iter().for_each(|c| flatten(c, out)),
        Widget::Row { children, .. } => {
            let mut parts = Vec::new();
            children.iter().for_each(|c| flatten(c, &mut parts));
            out.push(parts.join("  "));
        }
        _ => {}
    }
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let mut view = View::new(scan(std::path::Path::new(&dir)));
    for (label, yaw, pitch) in [("angle A", 0.7, 0.42), ("angle B", 2.1, 0.75)] {
        view.camera.yaw = yaw;
        view.camera.pitch = pitch;
        let tree = render(&view, 78, 26);
        let mut lines = Vec::new();
        flatten(&tree, &mut lines);
        println!("=== {label} (nodes {}) ===", measure(&tree).nodes);
        println!("{}\n", lines.join("\n"));
    }
}
