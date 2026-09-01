use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    Transmit,
    TransmitAndDisplay,
    Query,
    Display,
    Delete,
    TransmitFrame,
    ControlAnimation,
    ComposeFrame,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PixelFormat {
    Rgba,
    Rgb,
    Png,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Transmission {
    Direct,
    File,
    TempFile,
    SharedMemory,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CursorMovement {
    After,
    None,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeleteTarget {
    All,
    ById(u32),
    ByPos,
    ByCellRange,
    ByColumn,
    ByRow,
    ByZIndex,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompositionMode {
    AlphaBlend,
    Overwrite,
}

#[derive(Clone, Debug)]
pub struct GraphicsCommand {
    pub action: Action,
    pub image_id: u32,
    pub image_number: u32,
    pub placement_id: u32,
    pub format: PixelFormat,
    pub transmission: Transmission,
    pub compression: bool,
    pub more_chunks: bool,
    pub quiet: u8,
    pub cursor_movement: CursorMovement,
    pub width: u32,
    pub height: u32,
    pub src_x: u32,
    pub src_y: u32,
    pub src_width: u32,
    pub src_height: u32,
    pub display_cols: u32,
    pub display_rows: u32,
    pub x_offset: u32,
    pub y_offset: u32,
    pub z_index: i32,
    pub delete_target: Option<DeleteTarget>,
    pub composition_mode: CompositionMode,
    pub unicode_placeholder: bool,
    pub frame_number: u32,
    pub frame_gap: u32,
    pub background_frame: u32,
    pub loop_count: i32,
    pub payload: Vec<u8>,
}

impl Default for GraphicsCommand {
    fn default() -> Self {
        Self {
            action: Action::TransmitAndDisplay,
            image_id: 0,
            image_number: 0,
            placement_id: 0,
            format: PixelFormat::Rgba,
            transmission: Transmission::Direct,
            compression: false,
            more_chunks: false,
            quiet: 0,
            cursor_movement: CursorMovement::After,
            width: 0,
            height: 0,
            src_x: 0,
            src_y: 0,
            src_width: 0,
            src_height: 0,
            display_cols: 0,
            display_rows: 0,
            x_offset: 0,
            y_offset: 0,
            z_index: 0,
            delete_target: None,
            composition_mode: CompositionMode::AlphaBlend,
            unicode_placeholder: false,
            frame_number: 0,
            frame_gap: 0,
            background_frame: 0,
            loop_count: 0,
            payload: Vec::new(),
        }
    }
}

pub fn parse_graphics_command(data: &[u8]) -> Option<GraphicsCommand> {
    if data.is_empty() {
        return None;
    }

    let (control, payload) = match data.iter().position(|&b| b == b';') {
        Some(pos) => (&data[..pos], &data[pos + 1..]),
        None => (data, &[] as &[u8]),
    };

    let kvs = parse_control_keys(control);
    let mut cmd = GraphicsCommand::default();

    for (key, value) in &kvs {
        match key.as_str() {
            "a" => {
                cmd.action = match value.as_str() {
                    "t" => Action::Transmit,
                    "T" => Action::TransmitAndDisplay,
                    "q" => Action::Query,
                    "p" => Action::Display,
                    "d" => Action::Delete,
                    "f" => Action::TransmitFrame,
                    "a" => Action::ControlAnimation,
                    "c" => Action::ComposeFrame,
                    _ => Action::TransmitAndDisplay,
                };
            }
            "i" => cmd.image_id = value.parse().unwrap_or(0),
            "I" => cmd.image_number = value.parse().unwrap_or(0),
            "p" => cmd.placement_id = value.parse().unwrap_or(0),
            "f" => {
                cmd.format = match value.as_str() {
                    "24" => PixelFormat::Rgb,
                    "32" => PixelFormat::Rgba,
                    "100" => PixelFormat::Png,
                    _ => PixelFormat::Rgba,
                };
            }
            "t" => {
                cmd.transmission = match value.as_str() {
                    "d" => Transmission::Direct,
                    "f" => Transmission::File,
                    "t" => Transmission::TempFile,
                    "s" => Transmission::SharedMemory,
                    _ => Transmission::Direct,
                };
            }
            "o" => cmd.compression = value == "z",
            "m" => cmd.more_chunks = value == "1",
            "q" => cmd.quiet = value.parse().unwrap_or(0),
            "C" => {
                cmd.cursor_movement = if value == "1" {
                    CursorMovement::None
                } else {
                    CursorMovement::After
                };
            }
            "s" => cmd.width = value.parse().unwrap_or(0),
            "v" => cmd.height = value.parse().unwrap_or(0),
            "x" => cmd.src_x = value.parse().unwrap_or(0),
            "y" => cmd.src_y = value.parse().unwrap_or(0),
            "w" => cmd.src_width = value.parse().unwrap_or(0),
            "h" => cmd.src_height = value.parse().unwrap_or(0),
            "c" => cmd.display_cols = value.parse().unwrap_or(0),
            "r" => cmd.display_rows = value.parse().unwrap_or(0),
            "X" => cmd.x_offset = value.parse().unwrap_or(0),
            "Y" => cmd.y_offset = value.parse().unwrap_or(0),
            "z" => cmd.z_index = value.parse().unwrap_or(0),
            "O" => {
                cmd.composition_mode = if value == "1" {
                    CompositionMode::Overwrite
                } else {
                    CompositionMode::AlphaBlend
                };
            }
            "d" => {
                cmd.delete_target = Some(match value.as_str() {
                    "a" | "A" => DeleteTarget::All,
                    "i" | "I" => DeleteTarget::ById(cmd.image_id),
                    "p" | "P" => DeleteTarget::ByPos,
                    "n" | "N" => DeleteTarget::ByCellRange,
                    "x" | "X" => DeleteTarget::ByColumn,
                    "y" | "Y" => DeleteTarget::ByRow,
                    "z" | "Z" => DeleteTarget::ByZIndex,
                    _ => DeleteTarget::All,
                });
            }
            "U" => cmd.unicode_placeholder = value == "1",
            "S" => cmd.loop_count = value.parse().unwrap_or(0),
            _ => {}
        }
    }

    // Context-dependent keys: v/z/r have different meanings for animation actions.
    match cmd.action {
        Action::TransmitFrame => {
            if let Some(v) = kvs.get("v") {
                cmd.frame_gap = v.parse().unwrap_or(0);
            }
            if let Some(z) = kvs.get("z") {
                cmd.frame_number = z.parse().unwrap_or(0);
            }
            if let Some(r) = kvs.get("r") {
                cmd.background_frame = r.parse().unwrap_or(0);
            }
        }
        Action::ControlAnimation => {
            if let Some(v) = kvs.get("v") {
                cmd.frame_gap = v.parse().unwrap_or(0);
            }
            if let Some(z) = kvs.get("z") {
                cmd.frame_number = z.parse().unwrap_or(0);
            }
        }
        Action::ComposeFrame => {
            if let Some(z) = kvs.get("z") {
                cmd.frame_number = z.parse().unwrap_or(0);
            }
        }
        _ => {}
    }

    if cmd.action == Action::Delete {
        if let Some(d_val) = kvs.get("d") {
            cmd.delete_target = Some(match d_val.as_str() {
                "a" | "A" => DeleteTarget::All,
                "i" | "I" => DeleteTarget::ById(cmd.image_id),
                "p" | "P" => DeleteTarget::ByPos,
                "n" | "N" => DeleteTarget::ByCellRange,
                "x" | "X" => DeleteTarget::ByColumn,
                "y" | "Y" => DeleteTarget::ByRow,
                "z" | "Z" => DeleteTarget::ByZIndex,
                _ => DeleteTarget::All,
            });
        } else {
            cmd.delete_target.get_or_insert(DeleteTarget::All);
        }
    }

    cmd.payload = payload.to_vec();

    Some(cmd)
}

fn parse_control_keys(control: &[u8]) -> HashMap<String, String> {
    let control_str = std::str::from_utf8(control).unwrap_or("");
    let mut map = HashMap::new();
    for pair in control_str.split(',') {
        if let Some(eq_pos) = pair.find('=') {
            let key = &pair[..eq_pos];
            let value = &pair[eq_pos + 1..];
            map.insert(key.to_string(), value.to_string());
        }
    }
    map
}

pub fn format_ok_response(cmd: &GraphicsCommand) -> Vec<u8> {
    let mut response = Vec::with_capacity(64);
    response.extend_from_slice(b"\x1b_Gi=");
    response.extend_from_slice(cmd.image_id.to_string().as_bytes());
    if cmd.image_number > 0 {
        response.extend_from_slice(b",I=");
        response.extend_from_slice(cmd.image_number.to_string().as_bytes());
    }
    if cmd.placement_id > 0 {
        response.extend_from_slice(b",p=");
        response.extend_from_slice(cmd.placement_id.to_string().as_bytes());
    }
    response.extend_from_slice(b";OK\x1b\\");
    response
}

pub fn format_error_response(cmd: &GraphicsCommand, message: &str) -> Vec<u8> {
    let mut response = Vec::with_capacity(64 + message.len());
    response.extend_from_slice(b"\x1b_Gi=");
    response.extend_from_slice(cmd.image_id.to_string().as_bytes());
    if cmd.image_number > 0 {
        response.extend_from_slice(b",I=");
        response.extend_from_slice(cmd.image_number.to_string().as_bytes());
    }
    if cmd.placement_id > 0 {
        response.extend_from_slice(b",p=");
        response.extend_from_slice(cmd.placement_id.to_string().as_bytes());
    }
    response.extend_from_slice(b";E");
    response.extend_from_slice(message.as_bytes());
    response.extend_from_slice(b"\x1b\\");
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_transmit_and_display() {
        let data = b"a=T,f=32,s=100,v=50,i=1;base64data";
        let cmd = parse_graphics_command(data).unwrap();
        assert_eq!(cmd.action, Action::TransmitAndDisplay);
        assert_eq!(cmd.format, PixelFormat::Rgba);
        assert_eq!(cmd.width, 100);
        assert_eq!(cmd.height, 50);
        assert_eq!(cmd.image_id, 1);
        assert_eq!(cmd.payload, b"base64data");
    }

    #[test]
    fn parse_query_action() {
        let data = b"a=q,i=42,f=100;";
        let cmd = parse_graphics_command(data).unwrap();
        assert_eq!(cmd.action, Action::Query);
        assert_eq!(cmd.image_id, 42);
        assert_eq!(cmd.format, PixelFormat::Png);
    }

    #[test]
    fn parse_delete_action() {
        let data = b"a=d,d=i,i=5";
        let cmd = parse_graphics_command(data).unwrap();
        assert_eq!(cmd.action, Action::Delete);
        assert_eq!(cmd.delete_target, Some(DeleteTarget::ById(5)));
    }

    #[test]
    fn parse_multipart() {
        let data = b"m=1,i=3;chunk1";
        let cmd = parse_graphics_command(data).unwrap();
        assert!(cmd.more_chunks);
        assert_eq!(cmd.image_id, 3);
        assert_eq!(cmd.payload, b"chunk1");
    }

    #[test]
    fn parse_no_payload() {
        let data = b"a=p,i=7,p=2,c=10,r=5";
        let cmd = parse_graphics_command(data).unwrap();
        assert_eq!(cmd.action, Action::Display);
        assert_eq!(cmd.image_id, 7);
        assert_eq!(cmd.placement_id, 2);
        assert_eq!(cmd.display_cols, 10);
        assert_eq!(cmd.display_rows, 5);
        assert!(cmd.payload.is_empty());
    }

    #[test]
    fn format_ok_response_basic() {
        let mut cmd = GraphicsCommand::default();
        cmd.image_id = 42;
        let resp = format_ok_response(&cmd);
        assert_eq!(resp, b"\x1b_Gi=42;OK\x1b\\");
    }

    #[test]
    fn format_error_response_basic() {
        let mut cmd = GraphicsCommand::default();
        cmd.image_id = 7;
        let resp = format_error_response(&cmd, "ENOENT");
        assert_eq!(resp, b"\x1b_Gi=7;EENOENT\x1b\\");
    }

    #[test]
    fn parse_unicode_placeholder_flag() {
        let data = b"a=T,i=1,U=1,f=32,s=10,v=10;payload";
        let cmd = parse_graphics_command(data).unwrap();
        assert!(cmd.unicode_placeholder);
        assert_eq!(cmd.action, Action::TransmitAndDisplay);
    }

    #[test]
    fn parse_transmit_frame_context_keys() {
        let data = b"a=f,i=1,z=3,v=40,r=1,O=1;framedata";
        let cmd = parse_graphics_command(data).unwrap();
        assert_eq!(cmd.action, Action::TransmitFrame);
        assert_eq!(cmd.frame_number, 3);
        assert_eq!(cmd.frame_gap, 40);
        assert_eq!(cmd.background_frame, 1);
        assert_eq!(cmd.composition_mode, CompositionMode::Overwrite);
    }

    #[test]
    fn parse_control_animation_action() {
        let data = b"a=a,i=1,S=3,v=50,z=2";
        let cmd = parse_graphics_command(data).unwrap();
        assert_eq!(cmd.action, Action::ControlAnimation);
        assert_eq!(cmd.loop_count, 3);
        assert_eq!(cmd.frame_gap, 50);
        assert_eq!(cmd.frame_number, 2);
    }

    #[test]
    fn parse_compose_frame_action() {
        let data = b"a=c,i=1,z=5;composedata";
        let cmd = parse_graphics_command(data).unwrap();
        assert_eq!(cmd.action, Action::ComposeFrame);
        assert_eq!(cmd.frame_number, 5);
    }
}
