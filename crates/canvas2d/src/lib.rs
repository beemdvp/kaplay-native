mod color;
mod fonts;

use std::borrow::Cow;
use std::collections::HashMap;

pub use color::Rgba;
pub use fonts::{Baseline, FontStore, StrokeSpec, TextAlign, TextMetrics};

pub struct ImageSource<'a> {
    pub width: u32,
    pub height: u32,
    pub data: &'a [u8],
    pub premultiplied: bool,
}

struct Canvas {
    pixmap: tiny_skia::Pixmap,
}

#[derive(Debug, Clone)]
struct Context2d {
    canvas: u32,
    fill_style: Rgba,
    stroke_style: Rgba,
    line_width: f32,
    line_join: tiny_skia::LineJoin,
    font: String,
    text_baseline: Baseline,
    text_align: TextAlign,
}

pub struct Canvas2d {
    canvases: HashMap<u32, Canvas>,
    contexts: HashMap<u32, Context2d>,
    next_handle: u32,
    pub fonts: FontStore,
}

impl Canvas2d {
    pub fn new(default_font: Vec<u8>) -> Self {
        Self {
            canvases: HashMap::new(),
            contexts: HashMap::new(),
            next_handle: 1,
            fonts: FontStore::new(default_font),
        }
    }

    pub fn create_canvas(&mut self, width: u32, height: u32) -> u32 {
        let pixmap = tiny_skia::Pixmap::new(width.max(1), height.max(1)).unwrap();
        let handle = self.next_handle;
        self.next_handle += 1;
        self.canvases.insert(handle, Canvas { pixmap });
        handle
    }

    pub fn resize_canvas(&mut self, handle: u32, width: u32, height: u32) {
        if let Some(canvas) = self.canvases.get_mut(&handle) {
            if canvas.pixmap.width() == width.max(1) && canvas.pixmap.height() == height.max(1) {
                return;
            }
            if let Some(pixmap) = tiny_skia::Pixmap::new(width.max(1), height.max(1)) {
                canvas.pixmap = pixmap;
            }
        }
    }

    pub fn canvas_size(&self, handle: u32) -> Option<(u32, u32)> {
        self.canvases
            .get(&handle)
            .map(|canvas| (canvas.pixmap.width(), canvas.pixmap.height()))
    }

    pub fn canvas_bytes(&self, handle: u32) -> Option<&[u8]> {
        self.canvases
            .get(&handle)
            .map(|canvas| canvas.pixmap.data())
    }

    pub fn destroy_canvas(&mut self, handle: u32) {
        self.canvases.remove(&handle);
        self.contexts.retain(|_, context| context.canvas != handle);
    }

    pub fn create_context(&mut self, canvas: u32) -> Option<u32> {
        if !self.canvases.contains_key(&canvas) {
            return None;
        }
        let handle = self.next_handle;
        self.next_handle += 1;
        self.contexts.insert(
            handle,
            Context2d {
                canvas,
                fill_style: Rgba::BLACK,
                stroke_style: Rgba::BLACK,
                line_width: 1.0,
                line_join: tiny_skia::LineJoin::Miter,
                font: "10px sans-serif".into(),
                text_baseline: Baseline::Alphabetic,
                text_align: TextAlign::Left,
            },
        );
        Some(handle)
    }

    pub fn set_fill_style(&mut self, context: u32, value: &str) {
        if let (Some(context), Some(color)) =
            (self.contexts.get_mut(&context), Rgba::from_css(value))
        {
            context.fill_style = color;
        }
    }

    pub fn set_stroke_style(&mut self, context: u32, value: &str) {
        if let (Some(context), Some(color)) =
            (self.contexts.get_mut(&context), Rgba::from_css(value))
        {
            context.stroke_style = color;
        }
    }

    pub fn set_font(&mut self, context: u32, value: &str) {
        if let Some(context) = self.contexts.get_mut(&context) {
            context.font = value.to_string();
        }
    }

    pub fn set_line_width(&mut self, context: u32, value: f32) {
        if let Some(context) = self.contexts.get_mut(&context) {
            context.line_width = value;
        }
    }

    pub fn set_line_join(&mut self, context: u32, value: &str) {
        if let Some(context) = self.contexts.get_mut(&context) {
            context.line_join = match value {
                "round" => tiny_skia::LineJoin::Round,
                "bevel" => tiny_skia::LineJoin::Bevel,
                _ => tiny_skia::LineJoin::Miter,
            };
        }
    }

    pub fn set_text_baseline(&mut self, context: u32, value: &str) {
        if let Some(context) = self.contexts.get_mut(&context) {
            context.text_baseline = Baseline::from_css(value);
        }
    }

    pub fn set_text_align(&mut self, context: u32, value: &str) {
        if let Some(context) = self.contexts.get_mut(&context) {
            context.text_align = TextAlign::from_css(value);
        }
    }

    pub fn register_font(&mut self, family: &str, data: Vec<u8>) {
        self.fonts.register(family, data);
    }

    pub fn has_font(&self, family: &str) -> bool {
        self.fonts.has_family(family)
    }

    pub fn clear_rect(&mut self, context: u32, x: f32, y: f32, width: f32, height: f32) {
        let Some(context) = self.contexts.get(&context) else {
            return;
        };
        let Some(canvas) = self.canvases.get_mut(&context.canvas) else {
            return;
        };
        let Some(rect) = tiny_skia::Rect::from_xywh(x, y, width, height) else {
            return;
        };
        let paint = tiny_skia::Paint {
            shader: tiny_skia::Shader::SolidColor(tiny_skia::Color::TRANSPARENT),
            blend_mode: tiny_skia::BlendMode::Clear,
            ..Default::default()
        };
        canvas
            .pixmap
            .fill_rect(rect, &paint, tiny_skia::Transform::identity(), None);
    }

    pub fn fill_text(&mut self, context: u32, text: &str, x: f32, y: f32) {
        self.draw_text(context, text, x, y, false);
    }

    pub fn stroke_text(&mut self, context: u32, text: &str, x: f32, y: f32) {
        self.draw_text(context, text, x, y, true);
    }

    fn draw_text(&mut self, context: u32, text: &str, x: f32, y: f32, stroke: bool) {
        let Some(context) = self.contexts.get(&context) else {
            return;
        };
        let Some(canvas) = self.canvases.get_mut(&context.canvas) else {
            return;
        };
        if stroke {
            self.fonts.draw(
                &mut canvas.pixmap,
                &context.font,
                text,
                x,
                y,
                context.text_baseline,
                context.text_align,
                context.stroke_style,
                Some(StrokeSpec {
                    width: context.line_width,
                    line_join: context.line_join,
                }),
            );
        } else {
            self.fonts.draw(
                &mut canvas.pixmap,
                &context.font,
                text,
                x,
                y,
                context.text_baseline,
                context.text_align,
                context.fill_style,
                None,
            );
        }
    }

    pub fn measure_text(&self, context: u32, text: &str) -> TextMetrics {
        self.contexts
            .get(&context)
            .map(|context| self.fonts.measure(&context.font, text))
            .unwrap_or_default()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw_image_from_canvas(
        &mut self,
        context: u32,
        source: u32,
        source_x: f32,
        source_y: f32,
        source_width: f32,
        source_height: f32,
        dest_x: f32,
        dest_y: f32,
        dest_width: f32,
        dest_height: f32,
    ) {
        let Some(destination) = self.contexts.get(&context).map(|context| context.canvas) else {
            return;
        };
        if destination == source {
            let Some((width, height)) = self.canvas_size(source) else {
                return;
            };
            let Some(data) = self.canvas_bytes(source).map(<[u8]>::to_vec) else {
                return;
            };
            self.draw_image(
                context,
                ImageSource {
                    width,
                    height,
                    data: &data,
                    premultiplied: true,
                },
                source_x,
                source_y,
                source_width,
                source_height,
                dest_x,
                dest_y,
                dest_width,
                dest_height,
            );
            return;
        }
        let [destination, source_canvas] = self.canvases.get_disjoint_mut([&destination, &source]);
        let (Some(destination), Some(source_canvas)) = (destination, source_canvas) else {
            return;
        };
        draw_image_into(
            destination,
            ImageSource {
                width: source_canvas.pixmap.width(),
                height: source_canvas.pixmap.height(),
                data: source_canvas.pixmap.data(),
                premultiplied: true,
            },
            source_x,
            source_y,
            source_width,
            source_height,
            dest_x,
            dest_y,
            dest_width,
            dest_height,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw_image(
        &mut self,
        context: u32,
        image: ImageSource<'_>,
        source_x: f32,
        source_y: f32,
        source_width: f32,
        source_height: f32,
        dest_x: f32,
        dest_y: f32,
        dest_width: f32,
        dest_height: f32,
    ) {
        let Some(context) = self.contexts.get(&context) else {
            return;
        };
        let Some(canvas) = self.canvases.get_mut(&context.canvas) else {
            return;
        };
        draw_image_into(
            canvas,
            image,
            source_x,
            source_y,
            source_width,
            source_height,
            dest_x,
            dest_y,
            dest_width,
            dest_height,
        );
    }

    pub fn put_image_data(
        &mut self,
        context: u32,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        data: &[u8],
    ) {
        let Some(context) = self.contexts.get(&context) else {
            return;
        };
        let Some(canvas) = self.canvases.get_mut(&context.canvas) else {
            return;
        };
        let premultiplied = premultiply(data, width as usize, height as usize);
        let canvas_width = canvas.pixmap.width() as i64;
        let canvas_height = canvas.pixmap.height() as i64;
        let stride = canvas.pixmap.width() as usize * 4;

        for row in 0..height as i64 {
            let dest_y = y as i64 + row;
            if dest_y < 0 || dest_y >= canvas_height {
                continue;
            }
            let start_x = x.max(0) as usize;
            let end_x = (x as i64 + width as i64).min(canvas_width).max(0);
            if end_x <= start_x as i64 {
                continue;
            }
            let source_offset =
                row as usize * width as usize * 4 + (start_x - x.max(0) as usize) * 4;
            let dest_offset = dest_y as usize * stride + start_x * 4;
            let length = (end_x as usize - start_x) * 4;
            canvas.pixmap.data_mut()[dest_offset..dest_offset + length]
                .copy_from_slice(&premultiplied[source_offset..source_offset + length]);
        }
    }

    pub fn get_image_data(&self, context: u32, x: i32, y: i32, width: u32, height: u32) -> Vec<u8> {
        let mut output = vec![0u8; width as usize * height as usize * 4];
        let Some(context) = self.contexts.get(&context) else {
            return output;
        };
        let Some(canvas) = self.canvases.get(&context.canvas) else {
            return output;
        };
        let canvas_width = canvas.pixmap.width() as i64;
        let canvas_height = canvas.pixmap.height() as i64;
        let stride = canvas.pixmap.width() as usize * 4;

        for row in 0..height as i64 {
            let source_y = y as i64 + row;
            if source_y < 0 || source_y >= canvas_height {
                continue;
            }
            let start_x = x.max(0) as usize;
            let end_x = (x as i64 + width as i64).min(canvas_width).max(0);
            if end_x <= start_x as i64 {
                continue;
            }
            let dest_offset = row as usize * width as usize * 4 + (start_x - x.max(0) as usize) * 4;
            let source_offset = source_y as usize * stride + start_x * 4;
            let length = (end_x as usize - start_x) * 4;
            output[dest_offset..dest_offset + length]
                .copy_from_slice(&canvas.pixmap.data()[source_offset..source_offset + length]);
        }

        unpremultiply(&mut output);
        output
    }

    pub fn to_png(&self, canvas: u32) -> Option<Vec<u8>> {
        let canvas = self.canvases.get(&canvas)?;
        let mut data = canvas.pixmap.data().to_vec();
        unpremultiply(&mut data);
        encode_png(&data, canvas.pixmap.width(), canvas.pixmap.height())
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_image_into(
    canvas: &mut Canvas,
    image: ImageSource<'_>,
    source_x: f32,
    source_y: f32,
    source_width: f32,
    source_height: f32,
    dest_x: f32,
    dest_y: f32,
    dest_width: f32,
    dest_height: f32,
) {
    if source_width <= 0.0 || source_height <= 0.0 || dest_width <= 0.0 || dest_height <= 0.0 {
        return;
    }

    let source_data =
        &image.data[..(image.width as usize * image.height as usize * 4).min(image.data.len())];
    let premultiplied = if image.premultiplied {
        Cow::Borrowed(source_data)
    } else {
        premultiply(source_data, image.width as usize, image.height as usize)
    };
    let Some(source) =
        tiny_skia::PixmapRef::from_bytes(premultiplied.as_ref(), image.width, image.height)
    else {
        return;
    };

    let cropped = if source_x != 0.0
        || source_y != 0.0
        || source_width != image.width as f32
        || source_height != image.height as f32
    {
        crop_pixmap(
            &source,
            source_x as i32,
            source_y as i32,
            source_width as u32,
            source_height as u32,
        )
    } else {
        None
    };

    let transform = tiny_skia::Transform::from_row(
        dest_width / source_width,
        0.0,
        0.0,
        dest_height / source_height,
        dest_x,
        dest_y,
    );
    let paint = tiny_skia::PixmapPaint {
        opacity: 1.0,
        blend_mode: tiny_skia::BlendMode::SourceOver,
        quality: tiny_skia::FilterQuality::Bilinear,
    };

    match &cropped {
        Some(pixmap) => canvas
            .pixmap
            .draw_pixmap(0, 0, pixmap.as_ref(), &paint, transform, None),
        None => canvas
            .pixmap
            .draw_pixmap(0, 0, source, &paint, transform, None),
    }
}

fn encode_png(data: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    let mut output = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut output, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(data).ok()?;
    }
    Some(output)
}

fn premultiply(data: &[u8], width: usize, height: usize) -> Cow<'_, [u8]> {
    let source = &data[..(width * height * 4).min(data.len())];
    if source
        .as_chunks::<4>()
        .0
        .iter()
        .all(|pixel| pixel[3] == 255)
    {
        return Cow::Borrowed(source);
    }
    let mut output = source.to_vec();
    for pixel in output.as_chunks_mut::<4>().0 {
        let alpha = pixel[3] as u32;
        if alpha == 255 {
            continue;
        }
        pixel[0] = ((pixel[0] as u32 * alpha + 127) / 255) as u8;
        pixel[1] = ((pixel[1] as u32 * alpha + 127) / 255) as u8;
        pixel[2] = ((pixel[2] as u32 * alpha + 127) / 255) as u8;
    }
    Cow::Owned(output)
}

fn unpremultiply(data: &mut [u8]) {
    for pixel in data.as_chunks_mut::<4>().0 {
        let alpha = pixel[3] as u32;
        if alpha == 0 || alpha == 255 {
            continue;
        }
        pixel[0] = ((pixel[0] as u32 * 255 + alpha / 2) / alpha).min(255) as u8;
        pixel[1] = ((pixel[1] as u32 * 255 + alpha / 2) / alpha).min(255) as u8;
        pixel[2] = ((pixel[2] as u32 * 255 + alpha / 2) / alpha).min(255) as u8;
    }
}

fn crop_pixmap(
    source: &tiny_skia::PixmapRef<'_>,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> Option<tiny_skia::Pixmap> {
    let mut output = tiny_skia::Pixmap::new(width.max(1), height.max(1))?;
    let source_width = source.width() as i64;
    let source_height = source.height() as i64;
    let source_stride = source.width() as usize * 4;

    for row in 0..height as i64 {
        let source_y = y as i64 + row;
        if source_y < 0 || source_y >= source_height {
            continue;
        }
        let start = (-(x as i64)).max(0);
        let end = (source_width - x as i64).min(width as i64).max(0);
        if end <= start {
            continue;
        }
        let count = (end - start) as usize;
        let dest_offset = row as usize * width as usize * 4 + start as usize * 4;
        let source_offset = source_y as usize * source_stride + (x as i64 + start) as usize * 4;
        output.data_mut()[dest_offset..dest_offset + count * 4]
            .copy_from_slice(&source.data()[source_offset..source_offset + count * 4]);
    }

    Some(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT_FONT: &[u8] = include_bytes!("../../../assets/fonts/DejaVuSansMono.ttf");

    fn setup() -> (Canvas2d, u32, u32) {
        let mut store = Canvas2d::new(DEFAULT_FONT.to_vec());
        let canvas = store.create_canvas(32, 16);
        let context = store.create_context(canvas).unwrap();
        (store, canvas, context)
    }

    #[test]
    fn clears_rects() {
        let (mut store, canvas, context) = setup();
        store.set_fill_style(context, "#ff0000");
        store.put_image_data(
            context,
            0,
            0,
            2,
            2,
            &[
                255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255,
            ],
        );
        store.clear_rect(context, 0.0, 0.0, 1.0, 1.0);
        let bytes = store.canvas_bytes(canvas).unwrap();
        assert_eq!(&bytes[0..4], &[0, 0, 0, 0]);
        assert_ne!(&bytes[4..8], &[0, 0, 0, 0]);
    }

    #[test]
    fn round_trips_image_data() {
        let (mut store, _canvas, context) = setup();
        let pixels = [10u8, 20, 30, 128, 40, 50, 60, 255];
        store.put_image_data(context, 1, 1, 2, 1, &pixels);
        let read = store.get_image_data(context, 1, 1, 2, 1);
        assert_eq!(read, pixels);
    }

    #[test]
    fn draws_into_canvas() {
        let (mut store, canvas, context) = setup();
        store.set_fill_style(context, "#ffffff");
        store.set_font(context, "16px monospace");
        store.set_text_baseline(context, "top");
        store.fill_text(context, "hi", 0.0, 0.0);
        let bytes = store.canvas_bytes(canvas).unwrap();
        let painted = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|pixel| pixel[3] > 0)
            .count();
        assert!(painted > 0, "painted pixels: {painted}");
    }

    #[test]
    fn encodes_png() {
        let (store, canvas, _context) = setup();
        let png = store.to_png(canvas).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn crops_image_regions() {
        let (mut store, _canvas, context) = setup();
        let pixels = [
            1u8, 0, 0, 255, 2, 0, 0, 255, 3, 0, 0, 255, //
            4, 0, 0, 255, 5, 0, 0, 255, 6, 0, 0, 255,
        ];
        store.put_image_data(context, 0, 0, 3, 2, &pixels);
        let source = ImageSource {
            width: 3,
            height: 2,
            data: &pixels,
            premultiplied: false,
        };
        store.draw_image(context, source, 1.0, 0.0, 2.0, 1.0, 0.0, 0.0, 2.0, 1.0);
        let read = store.get_image_data(context, 0, 0, 2, 1);
        assert_eq!(&read[..4], &[2, 0, 0, 255]);
        assert_eq!(&read[4..], &[3, 0, 0, 255]);
    }

    #[test]
    fn crops_regions_larger_than_the_source() {
        let (mut store, _canvas, context) = setup();
        let pixels = [9u8, 0, 0, 255, 8, 0, 0, 255];
        let source = ImageSource {
            width: 2,
            height: 1,
            data: &pixels,
            premultiplied: false,
        };
        store.draw_image(context, source, 1.0, 0.0, 8.0, 1.0, 0.0, 0.0, 8.0, 1.0);
        let read = store.get_image_data(context, 0, 0, 8, 1);
        assert_eq!(&read[..4], &[8, 0, 0, 255]);
        assert_eq!(&read[4..8], &[0, 0, 0, 0]);
    }

    #[test]
    fn draws_canvas_without_double_premultiplying() {
        let mut store = Canvas2d::new(DEFAULT_FONT.to_vec());
        let source = store.create_canvas(1, 1);
        let source_context = store.create_context(source).unwrap();
        store.put_image_data(source_context, 0, 0, 1, 1, &[255, 0, 0, 128]);
        let destination = store.create_canvas(1, 1);
        let destination_context = store.create_context(destination).unwrap();
        store.draw_image_from_canvas(
            destination_context,
            source,
            0.0,
            0.0,
            1.0,
            1.0,
            0.0,
            0.0,
            1.0,
            1.0,
        );
        assert_eq!(
            store.get_image_data(destination_context, 0, 0, 1, 1),
            vec![255, 0, 0, 128]
        );
    }

    #[test]
    fn crops_regions_that_start_before_the_source() {
        let (mut store, _canvas, context) = setup();
        let pixels = [7u8, 0, 0, 255, 6, 0, 0, 255];
        let source = ImageSource {
            width: 2,
            height: 1,
            data: &pixels,
            premultiplied: false,
        };
        store.draw_image(context, source, -1.0, 0.0, 2.0, 1.0, 0.0, 0.0, 2.0, 1.0);
        let read = store.get_image_data(context, 0, 0, 2, 1);
        assert_eq!(&read[..4], &[0, 0, 0, 0]);
        assert_eq!(&read[4..], &[7, 0, 0, 255]);
    }
}
