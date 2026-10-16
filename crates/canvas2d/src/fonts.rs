use std::collections::HashMap;

use ttf_parser::OutlineBuilder;

use crate::color::Rgba;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TextMetrics {
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Baseline {
    #[default]
    Alphabetic,
    Top,
    Middle,
    Bottom,
}

impl Baseline {
    pub fn from_css(value: &str) -> Self {
        match value {
            "top" | "hanging" => Self::Top,
            "middle" => Self::Middle,
            "bottom" | "ideographic" => Self::Bottom,
            _ => Self::Alphabetic,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

impl TextAlign {
    pub fn from_css(value: &str) -> Self {
        match value {
            "center" => Self::Center,
            "right" | "end" => Self::Right,
            _ => Self::Left,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct StrokeSpec {
    pub width: f32,
    pub line_join: tiny_skia::LineJoin,
}

pub struct FontStore {
    system: fontdb::Database,
    custom: HashMap<String, Vec<u8>>,
    default: Vec<u8>,
}

impl FontStore {
    pub fn new(default: Vec<u8>) -> Self {
        let mut system = fontdb::Database::new();
        system.load_system_fonts();
        Self {
            system,
            custom: HashMap::new(),
            default,
        }
    }

    pub fn register(&mut self, family: &str, data: Vec<u8>) {
        self.custom.insert(family.to_ascii_lowercase(), data);
    }

    pub fn has_family(&self, family: &str) -> bool {
        let family = family.to_ascii_lowercase();
        if self.custom.contains_key(&family) {
            return true;
        }
        let generic = generic_family(family.as_str());
        let is_generic = matches!(
            generic,
            fontdb::Family::Monospace
                | fontdb::Family::SansSerif
                | fontdb::Family::Serif
                | fontdb::Family::Cursive
                | fontdb::Family::Fantasy
        );
        let families = [generic];
        if self
            .system
            .query(&fontdb::Query {
                families: &families,
                weight: fontdb::Weight::NORMAL,
                stretch: fontdb::Stretch::Normal,
                style: fontdb::Style::Normal,
            })
            .is_some()
        {
            return true;
        }
        is_generic && ttf_parser::Face::parse(&self.default, 0).is_ok()
    }

    pub fn measure(&self, font: &str, text: &str) -> TextMetrics {
        let (size, family, weight, style) = parse_font(font);
        let fallback = ttf_parser::Face::parse(&self.default, 0).ok();
        let result = self.with_face(&family, weight, style, &mut |face| {
            let scale = size / face.units_per_em() as f32;
            let ascent = face.ascender() as f32 * scale;
            let descent = -(face.descender() as f32) * scale;
            let mut width = 0.0f32;
            for character in text.chars() {
                if let Some((glyph_face, glyph)) =
                    resolve_glyph(&face, fallback.as_ref(), character)
                {
                    let glyph_scale = size / glyph_face.units_per_em() as f32;
                    width += glyph_face.glyph_hor_advance(glyph).unwrap_or(0) as f32 * glyph_scale;
                }
            }
            TextMetrics {
                width,
                ascent,
                descent,
            }
        });
        result.unwrap_or(TextMetrics {
            width: 0.0,
            ascent: size * 0.8,
            descent: size * 0.2,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        pixmap: &mut tiny_skia::Pixmap,
        font: &str,
        text: &str,
        x: f32,
        y: f32,
        baseline: Baseline,
        align: TextAlign,
        color: Rgba,
        stroke: Option<StrokeSpec>,
    ) {
        let (size, family, weight, style) = parse_font(font);
        let fallback = ttf_parser::Face::parse(&self.default, 0).ok();
        let metrics = self.measure(font, text);
        let start_x = match align {
            TextAlign::Left => x,
            TextAlign::Center => x - metrics.width / 2.0,
            TextAlign::Right => x - metrics.width,
        };

        let _ = self.with_face(&family, weight, style, &mut |face| {
            let units = face.units_per_em() as f32;
            let scale = size / units;
            let baseline_y = match baseline {
                Baseline::Alphabetic => y,
                Baseline::Top => y + face.ascender() as f32 * scale,
                Baseline::Middle => {
                    y + (face.ascender() as f32 + face.descender() as f32) * scale / 2.0
                }
                Baseline::Bottom => y + face.descender() as f32 * scale,
            };

            let mut paint = tiny_skia::Paint {
                anti_alias: true,
                ..Default::default()
            };
            paint.set_color(color.to_color());

            let mut pen_x = start_x;
            for character in text.chars() {
                let Some((glyph_face, glyph)) = resolve_glyph(&face, fallback.as_ref(), character)
                else {
                    continue;
                };
                let glyph_scale = size / glyph_face.units_per_em() as f32;
                let advance = glyph_face.glyph_hor_advance(glyph).unwrap_or(0) as f32 * glyph_scale;
                if let Some(path) = glyph_path(glyph_face, glyph) {
                    let transform = tiny_skia::Transform::from_row(
                        glyph_scale,
                        0.0,
                        0.0,
                        -glyph_scale,
                        pen_x,
                        baseline_y,
                    );
                    if let Some(stroke) = stroke {
                        let stroke = tiny_skia::Stroke {
                            width: stroke.width,
                            line_join: stroke.line_join,
                            ..Default::default()
                        };
                        pixmap.stroke_path(&path, &paint, &stroke, transform, None);
                    }
                    pixmap.fill_path(&path, &paint, tiny_skia::FillRule::Winding, transform, None);
                }
                pen_x += advance;
            }
        });
    }

    fn with_face<R>(
        &self,
        family: &str,
        weight: u16,
        style: u8,
        callback: &mut dyn FnMut(ttf_parser::Face<'_>) -> R,
    ) -> Option<R> {
        let family = family.to_ascii_lowercase();
        if let Some(data) = self.custom.get(&family) {
            if let Ok(face) = ttf_parser::Face::parse(data, 0) {
                return Some(callback(face));
            }
        }

        let generic = generic_family(family.as_str());
        let weight = if weight >= 600 {
            fontdb::Weight::BOLD
        } else {
            fontdb::Weight::NORMAL
        };
        let style = if style == 1 || style == 2 {
            fontdb::Style::Italic
        } else {
            fontdb::Style::Normal
        };
        let query = fontdb::Query {
            families: &[generic],
            weight,
            stretch: fontdb::Stretch::Normal,
            style,
        };
        if let Some(id) = self.system.query(&query) {
            let found = self.system.with_face_data(id, |data, index| {
                ttf_parser::Face::parse(data, index)
                    .ok()
                    .map(&mut *callback)
            });
            if let Some(result) = found.flatten() {
                return Some(result);
            }
        }

        ttf_parser::Face::parse(&self.default, 0).ok().map(callback)
    }
}

fn resolve_glyph<'f, 'd>(
    face: &'f ttf_parser::Face<'d>,
    fallback: Option<&'f ttf_parser::Face<'d>>,
    character: char,
) -> Option<(&'f ttf_parser::Face<'d>, ttf_parser::GlyphId)> {
    if let Some(glyph) = face.glyph_index(character) {
        return Some((face, glyph));
    }
    let fallback = fallback?;
    fallback
        .glyph_index(character)
        .map(|glyph| (fallback, glyph))
}

fn generic_family(family: &str) -> fontdb::Family<'_> {
    match family {
        "monospace" => fontdb::Family::Monospace,
        "sans-serif" => fontdb::Family::SansSerif,
        "serif" => fontdb::Family::Serif,
        "cursive" => fontdb::Family::Cursive,
        "fantasy" => fontdb::Family::Fantasy,
        name => fontdb::Family::Name(name),
    }
}

fn glyph_path(face: &ttf_parser::Face<'_>, glyph: ttf_parser::GlyphId) -> Option<tiny_skia::Path> {
    struct Builder {
        inner: tiny_skia::PathBuilder,
    }

    impl OutlineBuilder for Builder {
        fn move_to(&mut self, x: f32, y: f32) {
            self.inner.move_to(x, y);
        }
        fn line_to(&mut self, x: f32, y: f32) {
            self.inner.line_to(x, y);
        }
        fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
            self.inner.quad_to(x1, y1, x, y);
        }
        fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
            self.inner.cubic_to(x1, y1, x2, y2, x, y);
        }
        fn close(&mut self) {
            self.inner.close();
        }
    }

    let mut builder = Builder {
        inner: tiny_skia::PathBuilder::new(),
    };
    face.outline_glyph(glyph, &mut builder)?;
    builder.inner.finish()
}

pub fn parse_font(font: &str) -> (f32, String, u16, u8) {
    let mut size = 10.0f32;
    let mut weight = 400u16;
    let mut style = 0u8;
    let mut family_parts: Vec<String> = Vec::new();

    for token in font.split_whitespace() {
        if let Some(value) = token.strip_suffix("px") {
            if let Ok(parsed) = value.parse::<f32>() {
                size = parsed;
                continue;
            }
        }
        if let Some(value) = token.strip_suffix('%') {
            if let Ok(parsed) = value.parse::<f32>() {
                size = parsed / 100.0 * 16.0;
                continue;
            }
        }
        match token.to_ascii_lowercase().as_str() {
            "bold" => weight = 700,
            "normal" | "400" => weight = 400,
            "italic" | "oblique" => style = 1,
            _ => family_parts.push(token.to_string()),
        }
    }

    let family = family_parts
        .join(" ")
        .trim_matches(|character| character == '"' || character == '\'')
        .split(',')
        .next()
        .unwrap_or("monospace")
        .trim()
        .to_string();

    let family = if family.is_empty() {
        "monospace".to_string()
    } else {
        family
    };

    (size, family, weight, style)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT_FONT: &[u8] = include_bytes!("../../../assets/fonts/DejaVuSansMono.ttf");
    const ASCII_ONLY: &[u8] = include_bytes!("../tests/fixtures/ascii-only.ttf");

    #[test]
    fn parses_font_shorthand() {
        assert_eq!(parse_font("16px monospace").0, 16.0);
        assert_eq!(parse_font("16px monospace").1, "monospace");
        assert_eq!(parse_font("bold 24px \"Deja Vu\"").1, "Deja Vu");
        assert_eq!(parse_font("bold 24px \"Deja Vu\"").2, 700);
        assert_eq!(parse_font("italic 12px serif").3, 1);
    }

    #[test]
    fn measures_and_draws() {
        let store = FontStore::new(DEFAULT_FONT.to_vec());
        let metrics = store.measure("16px monospace", "hello");
        assert!(metrics.width > 0.0);
        assert!(metrics.ascent > 0.0);

        let mut pixmap = tiny_skia::Pixmap::new(64, 32).unwrap();
        store.draw(
            &mut pixmap,
            "16px monospace",
            "hi",
            2.0,
            2.0,
            Baseline::Top,
            TextAlign::Left,
            Rgba::WHITE,
            None,
        );
        let painted = pixmap
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[3] > 0);
        assert!(painted);
    }

    #[test]
    fn draws_missing_glyphs_with_the_default_font() {
        let narrow = ttf_parser::Face::parse(ASCII_ONLY, 0).unwrap();
        assert!(narrow.glyph_index('a').is_some());
        assert!(narrow.glyph_index('•').is_none());

        let mut store = FontStore::new(DEFAULT_FONT.to_vec());
        store.register("narrow", ASCII_ONLY.to_vec());

        assert!(store.measure("16px narrow", "•").width > 0.0);

        let mut pixmap = tiny_skia::Pixmap::new(32, 32).unwrap();
        store.draw(
            &mut pixmap,
            "16px narrow",
            "•",
            2.0,
            2.0,
            Baseline::Top,
            TextAlign::Left,
            Rgba::WHITE,
            None,
        );
        let painted = pixmap
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[3] > 0);
        assert!(painted);
    }

    #[test]
    fn strokes_text() {
        let store = FontStore::new(DEFAULT_FONT.to_vec());
        let mut pixmap = tiny_skia::Pixmap::new(64, 32).unwrap();
        store.draw(
            &mut pixmap,
            "16px monospace",
            "hi",
            2.0,
            2.0,
            Baseline::Top,
            TextAlign::Left,
            Rgba::WHITE,
            Some(StrokeSpec {
                width: 2.0,
                line_join: tiny_skia::LineJoin::Round,
            }),
        );
        let painted = pixmap
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[3] > 0);
        assert!(painted);
    }
}
