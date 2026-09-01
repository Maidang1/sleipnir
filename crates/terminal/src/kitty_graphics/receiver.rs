use std::collections::HashMap;

use super::protocol::{GraphicsCommand, PixelFormat};

#[derive(Debug)]
struct PendingTransmission {
    cmd: GraphicsCommand,
    chunks: Vec<Vec<u8>>,
}

pub struct ChunkReceiver {
    pending: HashMap<u32, PendingTransmission>,
    next_anon_id: u32,
}

impl ChunkReceiver {
    pub fn new() -> Self {
        Self {
            pending: HashMap::new(),
            next_anon_id: 0xF000_0000,
        }
    }

    pub fn receive(
        &mut self,
        cmd: GraphicsCommand,
    ) -> Option<(GraphicsCommand, Vec<u8>)> {
        let id = self.resolve_id(&cmd);

        if cmd.more_chunks {
            let entry = self.pending.entry(id).or_insert_with(|| PendingTransmission {
                cmd: cmd.clone(),
                chunks: Vec::new(),
            });
            entry.chunks.push(cmd.payload.clone());
            return None;
        }

        if let Some(mut pending) = self.pending.remove(&id) {
            pending.chunks.push(cmd.payload);
            let assembled: Vec<u8> = pending.chunks.into_iter().flatten().collect();
            let mut final_cmd = pending.cmd;
            final_cmd.more_chunks = false;
            final_cmd.payload = Vec::new();
            if final_cmd.image_id == 0 && cmd.image_id != 0 {
                final_cmd.image_id = cmd.image_id;
            }
            if final_cmd.placement_id == 0 && cmd.placement_id != 0 {
                final_cmd.placement_id = cmd.placement_id;
            }
            Some((final_cmd, assembled))
        } else {
            let payload = cmd.payload.clone();
            Some((cmd, payload))
        }
    }

    fn resolve_id(&mut self, cmd: &GraphicsCommand) -> u32 {
        if cmd.image_id > 0 {
            return cmd.image_id;
        }
        if cmd.image_number > 0 {
            return 0x8000_0000 | cmd.image_number;
        }
        let id = self.next_anon_id;
        self.next_anon_id = self.next_anon_id.wrapping_add(1);
        id
    }

    pub fn clear(&mut self) {
        self.pending.clear();
    }
}

pub fn decode_payload(
    cmd: &GraphicsCommand,
    raw_payload: &[u8],
) -> Result<Vec<u8>, String> {
    use base64::Engine;
    let engine = base64::engine::general_purpose::STANDARD;

    let decoded = engine
        .decode(raw_payload)
        .map_err(|e| format!("base64 decode failed: {e}"))?;

    if cmd.compression {
        decode_zlib(&decoded)
    } else {
        Ok(decoded)
    }
}

fn decode_zlib(data: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut decoder = flate2::read::ZlibDecoder::new(data);
    let mut decompressed = Vec::new();
    decoder
        .read_to_end(&mut decompressed)
        .map_err(|e| format!("zlib decompression failed: {e}"))?;
    Ok(decompressed)
}

pub fn rgba_from_pixels(
    format: PixelFormat,
    data: &[u8],
    width: u32,
    height: u32,
) -> Result<Vec<u8>, String> {
    match format {
        PixelFormat::Rgba => {
            let expected = (width as usize) * (height as usize) * 4;
            if data.len() < expected {
                return Err(format!(
                    "RGBA data too short: got {} bytes, expected {expected}",
                    data.len()
                ));
            }
            Ok(data[..expected].to_vec())
        }
        PixelFormat::Rgb => {
            let pixel_count = (width as usize) * (height as usize);
            let expected = pixel_count * 3;
            if data.len() < expected {
                return Err(format!(
                    "RGB data too short: got {} bytes, expected {expected}",
                    data.len()
                ));
            }
            let mut rgba = Vec::with_capacity(pixel_count * 4);
            for chunk in data[..expected].chunks_exact(3) {
                rgba.push(chunk[0]);
                rgba.push(chunk[1]);
                rgba.push(chunk[2]);
                rgba.push(255);
            }
            Ok(rgba)
        }
        PixelFormat::Png => decode_png(data),
    }
}

fn decode_png(data: &[u8]) -> Result<Vec<u8>, String> {
    let decoder = png::Decoder::new(data);
    let mut reader = decoder
        .read_info()
        .map_err(|e| format!("PNG decode error: {e}"))?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| format!("PNG frame error: {e}"))?;
    buf.truncate(info.buffer_size());

    match info.color_type {
        png::ColorType::Rgba => Ok(buf),
        png::ColorType::Rgb => {
            let pixel_count = buf.len() / 3;
            let mut rgba = Vec::with_capacity(pixel_count * 4);
            for chunk in buf.chunks_exact(3) {
                rgba.push(chunk[0]);
                rgba.push(chunk[1]);
                rgba.push(chunk[2]);
                rgba.push(255);
            }
            Ok(rgba)
        }
        png::ColorType::GrayscaleAlpha => {
            let pixel_count = buf.len() / 2;
            let mut rgba = Vec::with_capacity(pixel_count * 4);
            for chunk in buf.chunks_exact(2) {
                rgba.push(chunk[0]);
                rgba.push(chunk[0]);
                rgba.push(chunk[0]);
                rgba.push(chunk[1]);
            }
            Ok(rgba)
        }
        png::ColorType::Grayscale => {
            let mut rgba = Vec::with_capacity(buf.len() * 4);
            for &g in &buf {
                rgba.push(g);
                rgba.push(g);
                rgba.push(g);
                rgba.push(255);
            }
            Ok(rgba)
        }
        png::ColorType::Indexed => Err("Indexed PNG not supported".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_shot_passthrough() {
        let mut recv = ChunkReceiver::new();
        let mut cmd = GraphicsCommand::default();
        cmd.image_id = 1;
        cmd.payload = b"AAAA".to_vec();
        let result = recv.receive(cmd);
        assert!(result.is_some());
        let (c, p) = result.unwrap();
        assert_eq!(c.image_id, 1);
        assert_eq!(p, b"AAAA");
    }

    #[test]
    fn multipart_assembly() {
        let mut recv = ChunkReceiver::new();

        let mut c1 = GraphicsCommand::default();
        c1.image_id = 5;
        c1.more_chunks = true;
        c1.payload = b"AB".to_vec();
        assert!(recv.receive(c1).is_none());

        let mut c2 = GraphicsCommand::default();
        c2.image_id = 5;
        c2.more_chunks = true;
        c2.payload = b"CD".to_vec();
        assert!(recv.receive(c2).is_none());

        let mut c3 = GraphicsCommand::default();
        c3.image_id = 5;
        c3.more_chunks = false;
        c3.payload = b"EF".to_vec();
        let result = recv.receive(c3);
        assert!(result.is_some());
        let (cmd, payload) = result.unwrap();
        assert_eq!(cmd.image_id, 5);
        assert_eq!(payload, b"ABCDEF");
    }

    #[test]
    fn rgba_from_rgb_data() {
        let rgb = vec![255, 0, 0, 0, 255, 0];
        let rgba = rgba_from_pixels(PixelFormat::Rgb, &rgb, 2, 1).unwrap();
        assert_eq!(rgba, vec![255, 0, 0, 255, 0, 255, 0, 255]);
    }

    #[test]
    fn rgba_passthrough() {
        let data = vec![10, 20, 30, 40, 50, 60, 70, 80];
        let rgba = rgba_from_pixels(PixelFormat::Rgba, &data, 2, 1).unwrap();
        assert_eq!(rgba, data);
    }
}
