//! What the guest hands back after a frame.

use egui::{
    epaint::{
        image::ImageDelta,
        textures::{TextureFilter, TextureOptions, TextureWrapMode, TexturesDelta},
        ColorImage, ImageData, Mesh, Vertex,
    },
    CursorIcon, Rect, TextureId,
};
use std::sync::Arc;

use crate::{read_header, write_header, DecodeError, Reader, Writer};

/// A tessellated frame and the platform output that goes with it.
///
/// `Primitive::Callback` has no meaning across the bridge, so [`encode_output`] takes the
/// meshes only.
#[derive(Clone, Default, PartialEq)]
pub struct FrameOutput {
    pub pixels_per_point: f32,

    /// Clip rect and mesh per tessellated primitive, in guest-local points.
    pub primitives: Vec<(Rect, Mesh)>,

    pub textures_delta: TexturesDelta,

    pub cursor_icon: CursorIcon,

    /// Text the guest asked to put on the clipboard.
    pub copied_text: Option<String>,

    /// A url the guest asked to open.
    pub open_url: Option<String>,

    /// How long the host may wait before running the next frame. `None` means no repaint was
    /// requested.
    pub repaint_after_ms: Option<u32>,
}

impl std::fmt::Debug for FrameOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let vertices: usize = self.primitives.iter().map(|(_, m)| m.vertices.len()).sum();
        f.debug_struct("FrameOutput")
            .field("pixels_per_point", &self.pixels_per_point)
            .field("primitives", &self.primitives.len())
            .field("vertices", &vertices)
            .field("textures_set", &self.textures_delta.set.len())
            .field("textures_free", &self.textures_delta.free.len())
            .field("cursor_icon", &self.cursor_icon)
            .field("copied_text", &self.copied_text)
            .field("open_url", &self.open_url)
            .field("repaint_after_ms", &self.repaint_after_ms)
            .finish()
    }
}

pub fn encode_output(output: &FrameOutput, out: &mut Vec<u8>) {
    let w = &mut Writer::new(out);
    write_header(w);
    w.f32(output.pixels_per_point);

    w.usize(output.primitives.len());
    for (clip_rect, mesh) in &output.primitives {
        w.rect(*clip_rect);
        write_mesh(w, mesh);
    }

    write_textures_delta(w, &output.textures_delta);

    let cursor = CursorIcon::ALL
        .iter()
        .position(|icon| *icon == output.cursor_icon)
        .unwrap_or(0);
    w.u8(cursor as u8);

    w.option(output.copied_text.as_deref(), |w, text| w.str(text));
    w.option(output.open_url.as_deref(), |w, url| w.str(url));
    w.option(output.repaint_after_ms, |w, ms| w.u32(ms));
}

pub fn decode_output(bytes: &[u8]) -> Result<FrameOutput, DecodeError> {
    let r = &mut Reader::new(bytes);
    read_header(r)?;
    let pixels_per_point = r.f32()?;

    let count = r.usize()?;
    let mut primitives = Vec::with_capacity(count.min(4096));
    for _ in 0..count {
        primitives.push((r.rect()?, read_mesh(r)?));
    }

    let textures_delta = read_textures_delta(r)?;

    let index = r.u8()? as usize;
    let cursor_icon = *CursorIcon::ALL
        .get(index)
        .ok_or(DecodeError::BadTag {
            what: "CursorIcon",
            tag: index as u32,
        })?;

    Ok(FrameOutput {
        pixels_per_point,
        primitives,
        textures_delta,
        cursor_icon,
        copied_text: r.option(|r| r.string())?,
        open_url: r.option(|r| r.string())?,
        repaint_after_ms: r.option(|r| r.u32())?,
    })
}

fn write_texture_id(w: &mut Writer<'_>, id: TextureId) {
    match id {
        TextureId::Managed(id) => {
            w.u8(0);
            w.u64(id);
        }
        TextureId::User(id) => {
            w.u8(1);
            w.u64(id);
        }
    }
}

fn read_texture_id(r: &mut Reader<'_>) -> Result<TextureId, DecodeError> {
    Ok(match r.u8()? {
        0 => TextureId::Managed(r.u64()?),
        1 => TextureId::User(r.u64()?),
        tag => {
            return Err(DecodeError::BadTag {
                what: "TextureId",
                tag: tag as u32,
            })
        }
    })
}

fn write_mesh(w: &mut Writer<'_>, mesh: &Mesh) {
    write_texture_id(w, mesh.texture_id);

    w.usize(mesh.indices.len());
    for index in &mesh.indices {
        w.u32(*index);
    }

    w.usize(mesh.vertices.len());
    for vertex in &mesh.vertices {
        w.pos2(vertex.pos);
        w.pos2(vertex.uv);
        w.color32(vertex.color);
    }
}

fn read_mesh(r: &mut Reader<'_>) -> Result<Mesh, DecodeError> {
    let texture_id = read_texture_id(r)?;

    let index_count = r.usize()?;
    let mut indices = Vec::with_capacity(index_count.min(1 << 20));
    for _ in 0..index_count {
        indices.push(r.u32()?);
    }

    let vertex_count = r.usize()?;
    let mut vertices = Vec::with_capacity(vertex_count.min(1 << 20));
    for _ in 0..vertex_count {
        vertices.push(Vertex {
            pos: r.pos2()?,
            uv: r.pos2()?,
            color: r.color32()?,
        });
    }

    Ok(Mesh {
        indices,
        vertices,
        texture_id,
    })
}

fn write_filter(w: &mut Writer<'_>, filter: TextureFilter) {
    w.u8(match filter {
        TextureFilter::Nearest => 0,
        TextureFilter::Linear => 1,
    });
}

fn read_filter(r: &mut Reader<'_>) -> Result<TextureFilter, DecodeError> {
    Ok(match r.u8()? {
        0 => TextureFilter::Nearest,
        1 => TextureFilter::Linear,
        tag => {
            return Err(DecodeError::BadTag {
                what: "TextureFilter",
                tag: tag as u32,
            })
        }
    })
}

fn write_texture_options(w: &mut Writer<'_>, options: TextureOptions) {
    write_filter(w, options.magnification);
    write_filter(w, options.minification);
    w.u8(match options.wrap_mode {
        TextureWrapMode::ClampToEdge => 0,
        TextureWrapMode::Repeat => 1,
        TextureWrapMode::MirroredRepeat => 2,
    });
    w.option(options.mipmap_mode, write_filter);
}

fn read_texture_options(r: &mut Reader<'_>) -> Result<TextureOptions, DecodeError> {
    let magnification = read_filter(r)?;
    let minification = read_filter(r)?;
    let wrap_mode = match r.u8()? {
        0 => TextureWrapMode::ClampToEdge,
        1 => TextureWrapMode::Repeat,
        2 => TextureWrapMode::MirroredRepeat,
        tag => {
            return Err(DecodeError::BadTag {
                what: "TextureWrapMode",
                tag: tag as u32,
            })
        }
    };
    Ok(TextureOptions {
        magnification,
        minification,
        wrap_mode,
        mipmap_mode: r.option(read_filter)?,
    })
}

fn write_image_delta(w: &mut Writer<'_>, delta: &ImageDelta) {
    let ImageData::Color(image) = &delta.image;
    w.usize(image.size[0]);
    w.usize(image.size[1]);
    w.vec2(image.source_size);
    w.usize(image.pixels.len());
    for pixel in &image.pixels {
        w.color32(*pixel);
    }

    write_texture_options(w, delta.options);
    w.option(delta.pos, |w, pos| {
        w.usize(pos[0]);
        w.usize(pos[1]);
    });
}

fn read_image_delta(r: &mut Reader<'_>) -> Result<ImageDelta, DecodeError> {
    let size = [r.usize()?, r.usize()?];
    let source_size = r.vec2()?;
    let pixel_count = r.usize()?;
    let mut pixels = Vec::with_capacity(pixel_count.min(1 << 24));
    for _ in 0..pixel_count {
        pixels.push(r.color32()?);
    }

    let image = ColorImage {
        size,
        source_size,
        pixels,
    };

    Ok(ImageDelta {
        image: ImageData::Color(Arc::new(image)),
        options: read_texture_options(r)?,
        pos: r.option(|r| Ok([r.usize()?, r.usize()?]))?,
    })
}

fn write_textures_delta(w: &mut Writer<'_>, delta: &TexturesDelta) {
    let mut set: Vec<_> = delta.set.iter().collect();
    set.sort_by_key(|(id, _)| texture_order(**id));
    w.usize(set.len());
    for (id, deltas) in set {
        write_texture_id(w, *id);
        w.usize(deltas.len());
        for delta in deltas {
            write_image_delta(w, delta);
        }
    }

    let mut free: Vec<_> = delta.free.iter().copied().collect();
    free.sort_by_key(|id| texture_order(*id));
    w.usize(free.len());
    for id in free {
        write_texture_id(w, id);
    }
}

fn read_textures_delta(r: &mut Reader<'_>) -> Result<TexturesDelta, DecodeError> {
    let mut delta = TexturesDelta::default();

    let set_count = r.usize()?;
    for _ in 0..set_count {
        let id = read_texture_id(r)?;
        let count = r.usize()?;
        let entry = delta.set.entry(id).or_default();
        for _ in 0..count {
            entry.push(read_image_delta(r)?);
        }
    }

    let free_count = r.usize()?;
    for _ in 0..free_count {
        delta.free.insert(read_texture_id(r)?);
    }

    Ok(delta)
}

fn texture_order(id: TextureId) -> (u8, u64) {
    match id {
        TextureId::Managed(id) => (0, id),
        TextureId::User(id) => (1, id),
    }
}
