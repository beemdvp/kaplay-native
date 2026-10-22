use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use kaplay_native_canvas2d::{Canvas2d, ImageSource};

const FONT: &[u8] = include_bytes!("../../../assets/fonts/DejaVuSansMono.ttf");
const WIDTH: u32 = 960;
const HEIGHT: u32 = 540;
const SHORT_TEXT: &str = "Player 1  score: 12345";
const LONG_TEXT: &str = "The quick brown fox jumps over the lazy dog while the score climbs to 1234567 and the wave counter rolls over, testing text layout and glyph rasterisation across a longer run of characters.";

fn image(width: u32, height: u32, alpha: bool) -> Vec<u8> {
    let mut data = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let a = if alpha { ((x + y) % 256) as u8 } else { 255 };
            data.extend_from_slice(&[(x % 256) as u8, (y % 256) as u8, 0x88, a]);
        }
    }
    data
}

fn setup() -> (Canvas2d, u32, u32) {
    let mut canvas = Canvas2d::new(FONT.to_vec());
    let surface = canvas.create_canvas(WIDTH, HEIGHT);
    let context = canvas.create_context(surface).expect("context");
    canvas.set_fill_style(context, "#3366cc");
    canvas.set_font(context, "16px monospace");
    (canvas, surface, context)
}

fn draw_images(c: &mut Criterion) {
    let (mut canvas, _surface, context) = setup();
    let mut group = c.benchmark_group("canvas2d/draw_image");
    for size in [64u32, 256, 512] {
        let opaque = image(size, size, false);
        let alpha = image(size, size, true);
        group.throughput(Throughput::Bytes((size * size * 4) as u64));
        group.bench_with_input(BenchmarkId::new("opaque", size), &size, |b, &size| {
            b.iter(|| {
                canvas.draw_image(
                    context,
                    ImageSource {
                        width: size,
                        height: size,
                        data: &opaque,
                        premultiplied: false,
                    },
                    0.0,
                    0.0,
                    size as f32,
                    size as f32,
                    0.0,
                    0.0,
                    size as f32,
                    size as f32,
                );
            });
        });
        group.bench_with_input(BenchmarkId::new("alpha", size), &size, |b, &size| {
            b.iter(|| {
                canvas.draw_image(
                    context,
                    ImageSource {
                        width: size,
                        height: size,
                        data: &alpha,
                        premultiplied: false,
                    },
                    0.0,
                    0.0,
                    size as f32,
                    size as f32,
                    0.0,
                    0.0,
                    size as f32,
                    size as f32,
                );
            });
        });
    }
    group.finish();

    let source = canvas.create_canvas(256, 256);
    let source_context = canvas.create_context(source).expect("context");
    canvas.clear_rect(source_context, 0.0, 0.0, 256.0, 256.0);
    c.bench_function("canvas2d/draw_image_from_canvas", |b| {
        b.iter(|| {
            canvas.draw_image_from_canvas(
                context, source, 0.0, 0.0, 256.0, 256.0, 0.0, 0.0, 256.0, 256.0,
            );
        });
    });
}

fn primitives(c: &mut Criterion) {
    let (mut canvas, _surface, context) = setup();
    let mut group = c.benchmark_group("canvas2d/clear_rect");
    group.bench_function("16x16", |b| {
        b.iter(|| canvas.clear_rect(context, 10.0, 10.0, 16.0, 16.0));
    });
    group.bench_function("960x540", |b| {
        b.iter(|| canvas.clear_rect(context, 0.0, 0.0, WIDTH as f32, HEIGHT as f32));
    });
    group.finish();

    let mut group = c.benchmark_group("canvas2d/text");
    group.bench_function("fill_text/short", |b| {
        b.iter(|| canvas.fill_text(context, SHORT_TEXT, 10.0, 30.0));
    });
    group.bench_function("fill_text/long", |b| {
        b.iter(|| canvas.fill_text(context, LONG_TEXT, 10.0, 30.0));
    });
    group.bench_function("stroke_text/long", |b| {
        b.iter(|| canvas.stroke_text(context, LONG_TEXT, 10.0, 30.0));
    });
    group.bench_function("measure_text/short", |b| {
        b.iter(|| canvas.measure_text(context, SHORT_TEXT));
    });
    group.bench_function("measure_text/long", |b| {
        b.iter(|| canvas.measure_text(context, LONG_TEXT));
    });
    group.finish();

    c.bench_function("canvas2d/create-canvas", |b| {
        b.iter(|| {
            let canvas_id = canvas.create_canvas(256, 256);
            canvas.destroy_canvas(canvas_id);
        });
    });
}

fn pixels(c: &mut Criterion) {
    let (mut canvas, surface, context) = setup();
    let full = canvas.get_image_data(context, 0, 0, WIDTH, HEIGHT);
    let mut group = c.benchmark_group("canvas2d/pixels");
    group.throughput(Throughput::Bytes((WIDTH * HEIGHT * 4) as u64));
    group.bench_function("get_image_data/full", |b| {
        b.iter(|| canvas.get_image_data(context, 0, 0, WIDTH, HEIGHT));
    });
    group.throughput(Throughput::Bytes((256 * 256 * 4) as u64));
    group.bench_function("get_image_data/256", |b| {
        b.iter(|| canvas.get_image_data(context, 0, 0, 256, 256));
    });
    group.throughput(Throughput::Bytes((WIDTH * HEIGHT * 4) as u64));
    group.bench_function("put_image_data/full", |b| {
        b.iter(|| canvas.put_image_data(context, 0, 0, WIDTH, HEIGHT, &full));
    });
    group.throughput(Throughput::Bytes((WIDTH * HEIGHT * 4) as u64));
    group.bench_function("to_png/full", |b| b.iter(|| canvas.to_png(surface)));
    group.finish();
}

criterion_group!(benches, draw_images, primitives, pixels);
criterion_main!(benches);
