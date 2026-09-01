use std::collections::HashMap;
use std::sync::Arc;

use super::protocol::{
    Action, CursorMovement, DeleteTarget, GraphicsCommand, PixelFormat,
    format_error_response, format_ok_response,
};
use super::receiver::{ChunkReceiver, decode_payload, rgba_from_pixels};

const MAX_IMAGES: usize = 320;
const MAX_IMAGE_PIXELS: u64 = 100_000_000;
const MAX_TOTAL_BYTES: usize = 512 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct StoredImage {
    pub id: u32,
    pub width: u32,
    pub height: u32,
    pub data: Arc<Vec<u8>>,
}

#[derive(Clone, Debug)]
pub struct Placement {
    pub image_id: u32,
    pub placement_id: u32,
    pub anchor_line: i32,
    pub anchor_col: usize,
    pub display_cols: u32,
    pub display_rows: u32,
    pub src_x: u32,
    pub src_y: u32,
    pub src_width: u32,
    pub src_height: u32,
    pub x_offset: u32,
    pub y_offset: u32,
    pub z_index: i32,
    pub cursor_movement: CursorMovement,
}

#[derive(Clone, Debug)]
pub struct VisiblePlacement {
    pub image_id: u32,
    pub placement_id: u32,
    pub display_line: i32,
    pub display_col: usize,
    pub display_cols: u32,
    pub display_rows: u32,
    pub src_x: u32,
    pub src_y: u32,
    pub src_width: u32,
    pub src_height: u32,
    pub x_offset: u32,
    pub y_offset: u32,
    pub z_index: i32,
    pub image: Arc<Vec<u8>>,
    pub image_width: u32,
    pub image_height: u32,
}

pub struct ImageStore {
    images: HashMap<u32, StoredImage>,
    placements: HashMap<(u32, u32), Placement>,
    receiver: ChunkReceiver,
    total_bytes: usize,
    next_image_id: u32,
}

impl ImageStore {
    pub fn new() -> Self {
        Self {
            images: HashMap::new(),
            placements: HashMap::new(),
            receiver: ChunkReceiver::new(),
            total_bytes: 0,
            next_image_id: 1,
        }
    }

    pub fn process_command(
        &mut self,
        cmd: GraphicsCommand,
        cursor_line: i32,
        cursor_col: usize,
    ) -> Option<Vec<u8>> {
        match cmd.action {
            Action::Query => Some(self.handle_query(&cmd)),
            Action::Transmit => self.handle_transmit(cmd, cursor_line, cursor_col),
            Action::TransmitAndDisplay => {
                self.handle_transmit_and_display(cmd, cursor_line, cursor_col)
            }
            Action::Display => self.handle_display(cmd, cursor_line, cursor_col),
            Action::Delete => {
                self.handle_delete(&cmd);
                None
            }
            _ => None,
        }
    }

    fn handle_query(&self, cmd: &GraphicsCommand) -> Vec<u8> {
        format_ok_response(cmd)
    }

    fn handle_transmit(
        &mut self,
        cmd: GraphicsCommand,
        _cursor_line: i32,
        _cursor_col: usize,
    ) -> Option<Vec<u8>> {
        let quiet = cmd.quiet;
        let Some((final_cmd, raw_payload)) = self.receiver.receive(cmd) else {
            return None;
        };

        match self.store_image(&final_cmd, &raw_payload) {
            Ok(_) => {
                if quiet < 1 {
                    Some(format_ok_response(&final_cmd))
                } else {
                    None
                }
            }
            Err(msg) => {
                if quiet < 2 {
                    Some(format_error_response(&final_cmd, &msg))
                } else {
                    None
                }
            }
        }
    }

    fn handle_transmit_and_display(
        &mut self,
        cmd: GraphicsCommand,
        cursor_line: i32,
        cursor_col: usize,
    ) -> Option<Vec<u8>> {
        let quiet = cmd.quiet;
        let display_cols = cmd.display_cols;
        let display_rows = cmd.display_rows;
        let src_x = cmd.src_x;
        let src_y = cmd.src_y;
        let src_width = cmd.src_width;
        let src_height = cmd.src_height;
        let x_offset = cmd.x_offset;
        let y_offset = cmd.y_offset;
        let z_index = cmd.z_index;
        let placement_id = cmd.placement_id;
        let cursor_movement = cmd.cursor_movement;
        let Some((final_cmd, raw_payload)) = self.receiver.receive(cmd) else {
            return None;
        };

        match self.store_image(&final_cmd, &raw_payload) {
            Ok(image_id) => {
                let placement = Placement {
                    image_id,
                    placement_id,
                    anchor_line: cursor_line,
                    anchor_col: cursor_col,
                    display_cols,
                    display_rows,
                    src_x,
                    src_y,
                    src_width,
                    src_height,
                    x_offset,
                    y_offset,
                    z_index,
                    cursor_movement,
                };
                self.placements
                    .insert((image_id, placement_id), placement);
                if quiet < 1 {
                    Some(format_ok_response(&final_cmd))
                } else {
                    None
                }
            }
            Err(msg) => {
                if quiet < 2 {
                    Some(format_error_response(&final_cmd, &msg))
                } else {
                    None
                }
            }
        }
    }

    fn handle_display(
        &mut self,
        cmd: GraphicsCommand,
        cursor_line: i32,
        cursor_col: usize,
    ) -> Option<Vec<u8>> {
        let image_id = cmd.image_id;
        if !self.images.contains_key(&image_id) {
            if cmd.quiet < 2 {
                return Some(format_error_response(&cmd, "ENOENT:image not found"));
            }
            return None;
        }

        let placement = Placement {
            image_id,
            placement_id: cmd.placement_id,
            anchor_line: cursor_line,
            anchor_col: cursor_col,
            display_cols: cmd.display_cols,
            display_rows: cmd.display_rows,
            src_x: cmd.src_x,
            src_y: cmd.src_y,
            src_width: cmd.src_width,
            src_height: cmd.src_height,
            x_offset: cmd.x_offset,
            y_offset: cmd.y_offset,
            z_index: cmd.z_index,
            cursor_movement: cmd.cursor_movement,
        };
        self.placements
            .insert((image_id, cmd.placement_id), placement);
        if cmd.quiet < 1 {
            Some(format_ok_response(&cmd))
        } else {
            None
        }
    }

    fn handle_delete(&mut self, cmd: &GraphicsCommand) {
        let target = cmd.delete_target.unwrap_or(DeleteTarget::All);
        match target {
            DeleteTarget::All => {
                for img in self.images.values() {
                    self.total_bytes -= img.data.len();
                }
                self.images.clear();
                self.placements.clear();
            }
            DeleteTarget::ById(id) => {
                let id = if id > 0 { id } else { cmd.image_id };
                if let Some(img) = self.images.remove(&id) {
                    self.total_bytes -= img.data.len();
                }
                self.placements
                    .retain(|&(img_id, _), _| img_id != id);
            }
            DeleteTarget::ByPos | DeleteTarget::ByCellRange => {
                self.placements.retain(|_, p| {
                    p.anchor_line != cmd.src_y as i32
                        || p.anchor_col != cmd.src_x as usize
                });
            }
            DeleteTarget::ByColumn => {
                let col = cmd.src_x as usize;
                self.placements.retain(|_, p| p.anchor_col != col);
            }
            DeleteTarget::ByRow => {
                let row = cmd.src_y as i32;
                self.placements.retain(|_, p| p.anchor_line != row);
            }
            DeleteTarget::ByZIndex => {
                let z = cmd.z_index;
                self.placements.retain(|_, p| p.z_index != z);
            }
        }
        self.gc_unreferenced_images();
    }

    fn store_image(
        &mut self,
        cmd: &GraphicsCommand,
        raw_payload: &[u8],
    ) -> Result<u32, String> {
        if cmd.transmission != super::protocol::Transmission::Direct {
            return Err("ENOTSUP:only direct transmission supported".to_string());
        }

        let decoded = decode_payload(cmd, raw_payload)?;

        let (width, height) = if cmd.format == PixelFormat::Png {
            let (w, h) = png_dimensions(&decoded)?;
            (
                if cmd.width > 0 { cmd.width } else { w },
                if cmd.height > 0 { cmd.height } else { h },
            )
        } else {
            if cmd.width == 0 || cmd.height == 0 {
                return Err("EINVAL:width and height required for raw pixel data".to_string());
            }
            (cmd.width, cmd.height)
        };

        let pixel_count = width as u64 * height as u64;
        if pixel_count > MAX_IMAGE_PIXELS {
            return Err("EINVAL:image too large".to_string());
        }

        let rgba = rgba_from_pixels(cmd.format, &decoded, width, height)?;

        if self.total_bytes + rgba.len() > MAX_TOTAL_BYTES {
            self.evict_oldest();
            if self.total_bytes + rgba.len() > MAX_TOTAL_BYTES {
                return Err("ENOMEM:image store quota exceeded".to_string());
            }
        }

        while self.images.len() >= MAX_IMAGES {
            self.evict_oldest();
        }

        let image_id = if cmd.image_id > 0 {
            cmd.image_id
        } else {
            let id = self.next_image_id;
            self.next_image_id = self.next_image_id.wrapping_add(1);
            if self.next_image_id == 0 {
                self.next_image_id = 1;
            }
            id
        };

        if let Some(old) = self.images.remove(&image_id) {
            self.total_bytes -= old.data.len();
        }

        self.total_bytes += rgba.len();
        self.images.insert(
            image_id,
            StoredImage {
                id: image_id,
                width,
                height,
                data: Arc::new(rgba),
            },
        );

        Ok(image_id)
    }

    fn evict_oldest(&mut self) {
        if let Some(&id) = self.images.keys().next() {
            if let Some(img) = self.images.remove(&id) {
                self.total_bytes -= img.data.len();
            }
            self.placements.retain(|&(img_id, _), _| img_id != id);
        }
    }

    fn gc_unreferenced_images(&mut self) {
        let referenced: std::collections::HashSet<u32> =
            self.placements.keys().map(|&(id, _)| id).collect();
        let orphaned: Vec<u32> = self
            .images
            .keys()
            .filter(|id| !referenced.contains(id))
            .copied()
            .collect();
        for id in orphaned {
            if let Some(img) = self.images.remove(&id) {
                self.total_bytes -= img.data.len();
            }
        }
    }

    pub fn visible_placements(
        &self,
        display_offset: usize,
        history_size: usize,
        screen_lines: usize,
    ) -> Vec<VisiblePlacement> {
        let top_abs = history_size as i32 - display_offset as i32;
        let bottom_abs = top_abs + screen_lines as i32;

        let mut result = Vec::new();
        for placement in self.placements.values() {
            let Some(image) = self.images.get(&placement.image_id) else {
                continue;
            };
            let abs_line = placement.anchor_line;
            if abs_line < top_abs || abs_line >= bottom_abs {
                continue;
            }
            let display_line = abs_line - top_abs;
            let src_width = if placement.src_width > 0 {
                placement.src_width
            } else {
                image.width
            };
            let src_height = if placement.src_height > 0 {
                placement.src_height
            } else {
                image.height
            };

            result.push(VisiblePlacement {
                image_id: placement.image_id,
                placement_id: placement.placement_id,
                display_line,
                display_col: placement.anchor_col,
                display_cols: placement.display_cols,
                display_rows: placement.display_rows,
                src_x: placement.src_x,
                src_y: placement.src_y,
                src_width,
                src_height,
                x_offset: placement.x_offset,
                y_offset: placement.y_offset,
                z_index: placement.z_index,
                image: image.data.clone(),
                image_width: image.width,
                image_height: image.height,
            });
        }
        result.sort_by_key(|p| p.z_index);
        result
    }

    pub fn image_count(&self) -> usize {
        self.images.len()
    }

    pub fn placement_count(&self) -> usize {
        self.placements.len()
    }

    pub fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    pub fn clear_all(&mut self) {
        self.images.clear();
        self.placements.clear();
        self.receiver.clear();
        self.total_bytes = 0;
    }

    pub fn rebase_after_history_shrink(&mut self, removed: i32) {
        for placement in self.placements.values_mut() {
            placement.anchor_line -= removed;
        }
        self.placements
            .retain(|_, p| p.anchor_line >= 0);
    }
}

fn png_dimensions(data: &[u8]) -> Result<(u32, u32), String> {
    let decoder = png::Decoder::new(data);
    let reader = decoder
        .read_info()
        .map_err(|e| format!("PNG info error: {e}"))?;
    let info = reader.info();
    Ok((info.width, info.height))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_rgba(w: u32, h: u32) -> Vec<u8> {
        vec![128u8; (w as usize) * (h as usize) * 4]
    }

    fn base64_encode(data: &[u8]) -> Vec<u8> {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .encode(data)
            .into_bytes()
    }

    #[test]
    fn transmit_and_display_stores_image() {
        let mut store = ImageStore::new();
        let rgba = make_test_rgba(2, 2);
        let payload = base64_encode(&rgba);

        let mut cmd = GraphicsCommand::default();
        cmd.action = Action::TransmitAndDisplay;
        cmd.image_id = 1;
        cmd.width = 2;
        cmd.height = 2;
        cmd.format = PixelFormat::Rgba;
        cmd.payload = payload;

        let resp = store.process_command(cmd, 5, 0);
        assert!(resp.is_some());
        assert_eq!(store.image_count(), 1);
        assert_eq!(store.placement_count(), 1);
    }

    #[test]
    fn query_returns_ok() {
        let mut store = ImageStore::new();
        let mut cmd = GraphicsCommand::default();
        cmd.action = Action::Query;
        cmd.image_id = 42;
        let resp = store.process_command(cmd, 0, 0);
        assert!(resp.is_some());
        let resp = resp.unwrap();
        assert!(resp.starts_with(b"\x1b_G"));
        assert!(resp.windows(2).any(|w| w == b"OK"));
    }

    #[test]
    fn delete_all_clears() {
        let mut store = ImageStore::new();
        let rgba = make_test_rgba(1, 1);
        let payload = base64_encode(&rgba);

        let mut cmd = GraphicsCommand::default();
        cmd.action = Action::TransmitAndDisplay;
        cmd.image_id = 10;
        cmd.width = 1;
        cmd.height = 1;
        cmd.format = PixelFormat::Rgba;
        cmd.payload = payload;
        store.process_command(cmd, 0, 0);
        assert_eq!(store.image_count(), 1);

        let mut del = GraphicsCommand::default();
        del.action = Action::Delete;
        del.delete_target = Some(DeleteTarget::All);
        store.process_command(del, 0, 0);
        assert_eq!(store.image_count(), 0);
        assert_eq!(store.placement_count(), 0);
    }

    #[test]
    fn visible_placements_filters_by_viewport() {
        let mut store = ImageStore::new();
        let rgba = make_test_rgba(1, 1);
        let payload = base64_encode(&rgba);

        // Place images at absolute lines 0, 10, 20
        for i in 0..3 {
            let mut cmd = GraphicsCommand::default();
            cmd.action = Action::TransmitAndDisplay;
            cmd.image_id = i + 1;
            cmd.width = 1;
            cmd.height = 1;
            cmd.format = PixelFormat::Rgba;
            cmd.payload = payload.clone();
            store.process_command(cmd, i as i32 * 10, 0);
        }

        // history_size=20, display_offset=0 => viewport covers abs 20..30
        // Only the image at abs line 20 should be visible
        let visible = store.visible_placements(0, 20, 10);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].image_id, 3);

        // Scroll to top: display_offset=20 => viewport covers abs 0..10
        // Only the image at abs line 0 should be visible
        let visible = store.visible_placements(20, 20, 10);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].image_id, 1);
    }
}
