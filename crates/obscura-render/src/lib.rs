use std::collections::HashMap;
use std::io::Cursor;

use base64::Engine;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::{ColorType, ImageEncoder};
use obscura_dom::{DomTree, Node, NodeData, NodeId};
use url::Url;

const DEFAULT_WIDTH: u32 = 1280;
const DEFAULT_HEIGHT: u32 = 720;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureFormat {
    Png,
    Jpeg,
}

#[derive(Clone, Copy, Debug)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Clip {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Debug)]
pub struct CaptureOptions {
    pub format: CaptureFormat,
    pub quality: u8,
    pub viewport: Viewport,
    pub clip: Option<Clip>,
    pub capture_beyond_viewport: bool,
}

impl Default for CaptureOptions {
    fn default() -> Self {
        Self {
            format: CaptureFormat::Png,
            quality: 80,
            viewport: Viewport::default(),
            clip: None,
            capture_beyond_viewport: false,
        }
    }
}

pub fn capture_base64(
    dom: &DomTree,
    css_sources: &[String],
    base_url: Option<&Url>,
    options: &CaptureOptions,
) -> Result<String, String> {
    let mut renderer = Renderer::new(dom, css_sources, base_url, options.clone());
    let mut image = renderer.render();
    if let Some(clip) = options.clip {
        image = image.crop(clip);
    }
    image.encode(options.format, options.quality)
}

#[derive(Clone, Debug)]
struct Rule {
    selector: SimpleSelector,
    declarations: HashMap<String, String>,
}

#[derive(Clone, Debug)]
enum SimpleSelector {
    Tag(String),
    Class(String),
    Id(String),
}

#[derive(Clone, Debug)]
struct Style {
    display: String,
    color: Color,
    background: Color,
    border_color: Color,
    font_size: f32,
    font_weight: u16,
    line_height: f32,
    margin: Edges,
    padding: Edges,
    border_width: f32,
    width: Option<f32>,
    height: Option<f32>,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            display: "block".to_string(),
            color: Color::rgb(20, 24, 31),
            background: Color::transparent(),
            border_color: Color::rgb(190, 196, 208),
            font_size: 16.0,
            font_weight: 400,
            line_height: 19.2,
            margin: Edges::default(),
            padding: Edges::default(),
            border_width: 0.0,
            width: None,
            height: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Edges {
    top: f32,
    right: f32,
    bottom: f32,
    left: f32,
}

#[derive(Clone, Copy, Debug)]
struct Color {
    r: u8,
    g: u8,
    b: u8,
    a: u8,
}

impl Color {
    const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    const fn transparent() -> Self {
        Self::rgba(0, 0, 0, 0)
    }
}

#[derive(Clone, Copy, Debug)]
struct Rect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

#[derive(Clone)]
struct Raster {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl Raster {
    fn new(width: u32, height: u32) -> Self {
        let mut pixels = vec![255; width.saturating_mul(height).saturating_mul(4) as usize];
        for chunk in pixels.chunks_exact_mut(4) {
            chunk[3] = 255;
        }
        Self {
            width,
            height,
            pixels,
        }
    }

    fn crop(self, clip: Clip) -> Self {
        let x = clip.x.max(0.0).floor() as u32;
        let y = clip.y.max(0.0).floor() as u32;
        let w = clip.width.max(1.0).ceil() as u32;
        let h = clip.height.max(1.0).ceil() as u32;
        let mut out = Raster::new(w, h);
        for row in 0..h {
            for col in 0..w {
                let sx = x + col;
                let sy = y + row;
                if sx < self.width && sy < self.height {
                    let si = ((sy * self.width + sx) * 4) as usize;
                    let di = ((row * w + col) * 4) as usize;
                    out.pixels[di..di + 4].copy_from_slice(&self.pixels[si..si + 4]);
                }
            }
        }
        out
    }

    fn encode(&self, format: CaptureFormat, quality: u8) -> Result<String, String> {
        let mut bytes = Vec::new();
        match format {
            CaptureFormat::Png => {
                let encoder = PngEncoder::new(Cursor::new(&mut bytes));
                encoder
                    .write_image(
                        &self.pixels,
                        self.width,
                        self.height,
                        ColorType::Rgba8.into(),
                    )
                    .map_err(|e| format!("PNG encode failed: {e}"))?;
            }
            CaptureFormat::Jpeg => {
                let mut rgb = Vec::with_capacity((self.width * self.height * 3) as usize);
                for px in self.pixels.chunks_exact(4) {
                    rgb.extend_from_slice(&px[..3]);
                }
                let mut encoder =
                    JpegEncoder::new_with_quality(Cursor::new(&mut bytes), quality.clamp(1, 100));
                encoder
                    .encode(&rgb, self.width, self.height, ColorType::Rgb8.into())
                    .map_err(|e| format!("JPEG encode failed: {e}"))?;
            }
        }
        Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
    }

    fn fill_rect(&mut self, rect: Rect, color: Color) {
        if color.a == 0 || rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        let x0 = rect.x.floor().max(0.0) as u32;
        let y0 = rect.y.floor().max(0.0) as u32;
        let x1 = (rect.x + rect.w).ceil().max(0.0).min(self.width as f32) as u32;
        let y1 = (rect.y + rect.h).ceil().max(0.0).min(self.height as f32) as u32;
        for y in y0..y1 {
            for x in x0..x1 {
                let i = ((y * self.width + x) * 4) as usize;
                blend(&mut self.pixels[i..i + 4], color);
            }
        }
    }

    fn stroke_rect(&mut self, rect: Rect, width: f32, color: Color) {
        if width <= 0.0 {
            return;
        }
        let w = width.max(1.0);
        self.fill_rect(Rect { h: w, ..rect }, color);
        self.fill_rect(
            Rect {
                y: rect.y + rect.h - w,
                h: w,
                ..rect
            },
            color,
        );
        self.fill_rect(Rect { w, ..rect }, color);
        self.fill_rect(
            Rect {
                x: rect.x + rect.w - w,
                w,
                ..rect
            },
            color,
        );
    }

    fn draw_text(&mut self, text: &str, x: f32, y: f32, style: &Style, max_width: f32) -> f32 {
        let char_w = (style.font_size * 0.62).max(5.0);
        let glyph_w = (char_w * 0.72).max(3.0);
        let glyph_h = (style.font_size * 0.78).max(6.0);
        let line_h = style.line_height.max(style.font_size * 1.2);
        let mut cx = x;
        let mut cy = y;
        for word in text.split_whitespace() {
            let word_w = word.chars().count() as f32 * char_w;
            if cx > x && cx + word_w > x + max_width {
                cx = x;
                cy += line_h;
            }
            for ch in word.chars() {
                self.draw_pseudo_glyph(
                    ch,
                    cx,
                    cy,
                    glyph_w,
                    glyph_h,
                    style.color,
                    style.font_weight,
                );
                cx += char_w;
            }
            cx += char_w;
        }
        cy + line_h
    }

    fn draw_pseudo_glyph(
        &mut self,
        ch: char,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: Color,
        weight: u16,
    ) {
        if ch.is_whitespace() {
            return;
        }
        let cols = 5;
        let rows = 7;
        let cell_w = (w / cols as f32).max(1.0);
        let cell_h = (h / rows as f32).max(1.0);
        let mut seed = ch as u32;
        for row in 0..rows {
            for col in 0..cols {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let border = row == 0 || row == rows - 1 || col == 0 || col == cols - 1;
                let on = if ch.is_ascii_alphanumeric() {
                    border || seed % 5 == 0
                } else {
                    seed % 3 != 0
                };
                if on {
                    let bold = if weight >= 600 { 1.25 } else { 1.0 };
                    self.fill_rect(
                        Rect {
                            x: x + col as f32 * cell_w,
                            y: y + row as f32 * cell_h,
                            w: (cell_w * bold).max(1.0),
                            h: cell_h.max(1.0),
                        },
                        color,
                    );
                }
            }
        }
    }
}

fn blend(dst: &mut [u8], src: Color) {
    let alpha = src.a as f32 / 255.0;
    let inv = 1.0 - alpha;
    dst[0] = (src.r as f32 * alpha + dst[0] as f32 * inv).round() as u8;
    dst[1] = (src.g as f32 * alpha + dst[1] as f32 * inv).round() as u8;
    dst[2] = (src.b as f32 * alpha + dst[2] as f32 * inv).round() as u8;
    dst[3] = 255;
}

struct Renderer<'a> {
    dom: &'a DomTree,
    rules: Vec<Rule>,
    base_url: Option<&'a Url>,
    options: CaptureOptions,
}

impl<'a> Renderer<'a> {
    fn new(
        dom: &'a DomTree,
        css_sources: &[String],
        base_url: Option<&'a Url>,
        options: CaptureOptions,
    ) -> Self {
        let mut css = String::new();
        for source in css_sources {
            css.push_str(source);
            css.push('\n');
        }
        for style_id in dom.query_selector_all("style").unwrap_or_default() {
            css.push_str(&dom.text_content(style_id));
            css.push('\n');
        }
        Self {
            dom,
            rules: parse_rules(&css),
            base_url,
            options,
        }
    }

    fn render(&mut self) -> Raster {
        let width = self.options.viewport.width.max(1);
        let viewport_height = self.options.viewport.height.max(1);
        let root = self.dom.find_body_or_root();
        let mut measure = Raster::new(width, viewport_height);
        let content_h = self.layout_node(root, 0.0, 0.0, width as f32, &mut measure);
        let height = if self.options.capture_beyond_viewport {
            content_h.ceil().max(viewport_height as f32) as u32
        } else {
            viewport_height
        };
        let mut raster = Raster::new(width, height);
        self.layout_node(root, 0.0, 0.0, width as f32, &mut raster);
        raster
    }

    fn layout_node(&self, id: NodeId, x: f32, y: f32, width: f32, raster: &mut Raster) -> f32 {
        let node = match self.dom.get_node(id) {
            Some(node) => node,
            None => return y,
        };
        match &node.data {
            NodeData::Text { contents } => {
                let style = Style::default();
                raster.draw_text(contents, x, y, &style, width)
            }
            NodeData::Element { name, .. } => {
                let tag = name.local.as_ref();
                if is_skipped_tag(tag) {
                    return y;
                }
                let mut style = self.computed_style(&node);
                apply_tag_defaults(tag, &mut style);
                if style.display == "none" {
                    return y;
                }
                if is_inline_like(tag) {
                    let text = self.dom.text_content(id);
                    return raster.draw_text(&text, x, y, &style, width);
                }

                let outer_x = x + style.margin.left;
                let outer_y = y + style.margin.top;
                let outer_w = style
                    .width
                    .unwrap_or(width - style.margin.left - style.margin.right)
                    .max(1.0);
                let content_x = outer_x + style.border_width + style.padding.left;
                let mut cursor_y = outer_y + style.border_width + style.padding.top;
                let content_w =
                    (outer_w - style.border_width * 2.0 - style.padding.left - style.padding.right)
                        .max(1.0);

                if is_form_control(tag) {
                    let h = style
                        .height
                        .unwrap_or(if tag == "textarea" { 72.0 } else { 34.0 });
                    let rect = Rect {
                        x: outer_x,
                        y: outer_y,
                        w: outer_w,
                        h,
                    };
                    raster.fill_rect(rect, Color::rgb(248, 250, 252));
                    raster.stroke_rect(rect, 1.0, style.border_color);
                    let label = node
                        .get_attribute("value")
                        .unwrap_or(node.get_attribute("placeholder").unwrap_or(""));
                    raster.draw_text(
                        label,
                        content_x + 4.0,
                        outer_y + 8.0,
                        &style,
                        content_w - 8.0,
                    );
                    return outer_y + h + style.margin.bottom;
                }

                if tag == "img" {
                    let h = style.height.unwrap_or(120.0);
                    let rect = Rect {
                        x: outer_x,
                        y: outer_y,
                        w: outer_w,
                        h,
                    };
                    if !self.draw_image_node(&node, rect, raster) {
                        raster.fill_rect(rect, Color::rgb(226, 232, 240));
                        raster.stroke_rect(rect, 1.0, Color::rgb(148, 163, 184));
                    }
                    return outer_y + h + style.margin.bottom;
                }

                let start_y = cursor_y;
                for child in self.dom.children(id) {
                    cursor_y = self.layout_node(child, content_x, cursor_y, content_w, raster);
                }
                let content_h = style
                    .height
                    .unwrap_or((cursor_y - start_y) + style.padding.bottom + style.border_width);
                let block_h =
                    content_h + style.border_width * 2.0 + style.padding.top + style.padding.bottom;
                let rect = Rect {
                    x: outer_x,
                    y: outer_y,
                    w: outer_w,
                    h: block_h.max(style.line_height),
                };
                raster.fill_rect(rect, style.background);
                raster.stroke_rect(rect, style.border_width, style.border_color);
                cursor_y.max(outer_y + rect.h) + style.margin.bottom
            }
            NodeData::Document => {
                let mut cursor_y = y;
                for child in self.dom.children(id) {
                    cursor_y = self.layout_node(child, x, cursor_y, width, raster);
                }
                cursor_y
            }
            _ => y,
        }
    }

    fn computed_style(&self, node: &Node) -> Style {
        let mut style = Style::default();
        for rule in &self.rules {
            if selector_matches(&rule.selector, node) {
                apply_declarations(&mut style, &rule.declarations);
            }
        }
        if let Some(inline) = node.get_attribute("style") {
            let declarations = parse_declarations(inline);
            apply_declarations(&mut style, &declarations);
        }
        style
    }

    fn draw_image_node(&self, node: &Node, rect: Rect, raster: &mut Raster) -> bool {
        let Some(src) = node.get_attribute("src") else {
            return false;
        };
        let data = if let Some(data) = decode_data_uri(src) {
            data
        } else if let Some(base) = self.base_url {
            let _ = base.join(src);
            return false;
        } else {
            return false;
        };
        let Ok(img) = image::load_from_memory(&data) else {
            return false;
        };
        let img = img.to_rgba8();
        let src_w = img.width().max(1);
        let src_h = img.height().max(1);
        for dy in 0..rect.h.max(1.0) as u32 {
            for dx in 0..rect.w.max(1.0) as u32 {
                let sx = ((dx as f32 / rect.w.max(1.0)) * src_w as f32).floor() as u32;
                let sy = ((dy as f32 / rect.h.max(1.0)) * src_h as f32).floor() as u32;
                let px = img.get_pixel(sx.min(src_w - 1), sy.min(src_h - 1));
                raster.fill_rect(
                    Rect {
                        x: rect.x + dx as f32,
                        y: rect.y + dy as f32,
                        w: 1.0,
                        h: 1.0,
                    },
                    Color::rgba(px[0], px[1], px[2], px[3]),
                );
            }
        }
        true
    }
}

fn is_skipped_tag(tag: &str) -> bool {
    matches!(
        tag,
        "head" | "script" | "style" | "meta" | "link" | "title" | "noscript"
    )
}

fn is_inline_like(tag: &str) -> bool {
    matches!(
        tag,
        "span" | "a" | "strong" | "em" | "b" | "i" | "small" | "label"
    )
}

fn is_form_control(tag: &str) -> bool {
    matches!(tag, "input" | "button" | "textarea" | "select")
}

fn apply_tag_defaults(tag: &str, style: &mut Style) {
    match tag {
        "body" => {
            style.margin = Edges {
                top: 8.0,
                right: 8.0,
                bottom: 8.0,
                left: 8.0,
            };
        }
        "h1" => {
            style.font_size = 32.0;
            style.font_weight = 700;
            style.line_height = 38.0;
            style.margin.top = 12.0;
            style.margin.bottom = 12.0;
        }
        "h2" => {
            style.font_size = 24.0;
            style.font_weight = 700;
            style.line_height = 30.0;
            style.margin.top = 10.0;
            style.margin.bottom = 10.0;
        }
        "p" | "ul" | "ol" => {
            style.margin.bottom = 12.0;
        }
        "button" => {
            style.background = Color::rgb(241, 245, 249);
            style.border_width = 1.0;
            style.padding = Edges {
                top: 7.0,
                right: 12.0,
                bottom: 7.0,
                left: 12.0,
            };
        }
        "input" | "textarea" | "select" => {
            style.border_width = 1.0;
            style.padding = Edges {
                top: 6.0,
                right: 8.0,
                bottom: 6.0,
                left: 8.0,
            };
        }
        _ => {}
    }
}

fn parse_rules(css: &str) -> Vec<Rule> {
    let mut rules = Vec::new();
    for block in css.split('}') {
        let Some((selector_text, declarations_text)) = block.split_once('{') else {
            continue;
        };
        let declarations = parse_declarations(declarations_text);
        if declarations.is_empty() {
            continue;
        }
        for selector in selector_text.split(',') {
            if let Some(selector) = parse_selector(selector.trim()) {
                rules.push(Rule {
                    selector,
                    declarations: declarations.clone(),
                });
            }
        }
    }
    rules
}

fn parse_selector(input: &str) -> Option<SimpleSelector> {
    if input.is_empty() || input.contains([' ', '>', '+', '~', ':', '[']) {
        return None;
    }
    if let Some(class) = input.strip_prefix('.') {
        return Some(SimpleSelector::Class(class.to_string()));
    }
    if let Some(id) = input.strip_prefix('#') {
        return Some(SimpleSelector::Id(id.to_string()));
    }
    Some(SimpleSelector::Tag(input.to_ascii_lowercase()))
}

fn selector_matches(selector: &SimpleSelector, node: &Node) -> bool {
    match selector {
        SimpleSelector::Tag(tag) => node
            .as_element()
            .map(|name| name.local.as_ref().eq_ignore_ascii_case(tag))
            .unwrap_or(false),
        SimpleSelector::Class(class) => node
            .get_attribute("class")
            .map(|classes| classes.split_whitespace().any(|c| c == class))
            .unwrap_or(false),
        SimpleSelector::Id(id) => node.get_attribute("id").map(|v| v == id).unwrap_or(false),
    }
}

fn parse_declarations(input: &str) -> HashMap<String, String> {
    let mut declarations = HashMap::new();
    for declaration in input.split(';') {
        let Some((name, value)) = declaration.split_once(':') else {
            continue;
        };
        declarations.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
    }
    declarations
}

fn apply_declarations(style: &mut Style, declarations: &HashMap<String, String>) {
    for (name, value) in declarations {
        match name.as_str() {
            "display" => style.display = value.trim().to_ascii_lowercase(),
            "color" => style.color = parse_color(value).unwrap_or(style.color),
            "background" | "background-color" => {
                style.background = parse_color(value).unwrap_or(style.background)
            }
            "border-color" => style.border_color = parse_color(value).unwrap_or(style.border_color),
            "font-size" => style.font_size = parse_px(value).unwrap_or(style.font_size),
            "font-weight" => {
                style.font_weight =
                    value
                        .parse::<u16>()
                        .unwrap_or(if value.eq_ignore_ascii_case("bold") {
                            700
                        } else {
                            style.font_weight
                        })
            }
            "line-height" => {
                style.line_height = parse_px(value).unwrap_or_else(|| {
                    value
                        .parse::<f32>()
                        .map(|n| n * style.font_size)
                        .unwrap_or(style.line_height)
                })
            }
            "width" => style.width = parse_px(value),
            "height" => style.height = parse_px(value),
            "margin" => style.margin = parse_edges(value).unwrap_or(style.margin),
            "margin-top" => style.margin.top = parse_px(value).unwrap_or(style.margin.top),
            "margin-right" => style.margin.right = parse_px(value).unwrap_or(style.margin.right),
            "margin-bottom" => style.margin.bottom = parse_px(value).unwrap_or(style.margin.bottom),
            "margin-left" => style.margin.left = parse_px(value).unwrap_or(style.margin.left),
            "padding" => style.padding = parse_edges(value).unwrap_or(style.padding),
            "padding-top" => style.padding.top = parse_px(value).unwrap_or(style.padding.top),
            "padding-right" => style.padding.right = parse_px(value).unwrap_or(style.padding.right),
            "padding-bottom" => {
                style.padding.bottom = parse_px(value).unwrap_or(style.padding.bottom)
            }
            "padding-left" => style.padding.left = parse_px(value).unwrap_or(style.padding.left),
            "border" | "border-width" => {
                style.border_width = parse_px(value).unwrap_or_else(|| {
                    value
                        .split_whitespace()
                        .find_map(parse_px)
                        .unwrap_or(style.border_width)
                });
                if let Some(color) = value.split_whitespace().find_map(parse_color) {
                    style.border_color = color;
                }
            }
            _ => {}
        }
    }
}

fn parse_edges(value: &str) -> Option<Edges> {
    let values: Vec<f32> = value.split_whitespace().filter_map(parse_px).collect();
    match values.as_slice() {
        [all] => Some(Edges {
            top: *all,
            right: *all,
            bottom: *all,
            left: *all,
        }),
        [vertical, horizontal] => Some(Edges {
            top: *vertical,
            right: *horizontal,
            bottom: *vertical,
            left: *horizontal,
        }),
        [top, horizontal, bottom] => Some(Edges {
            top: *top,
            right: *horizontal,
            bottom: *bottom,
            left: *horizontal,
        }),
        [top, right, bottom, left, ..] => Some(Edges {
            top: *top,
            right: *right,
            bottom: *bottom,
            left: *left,
        }),
        _ => None,
    }
}

fn parse_px(value: &str) -> Option<f32> {
    let trimmed = value.trim();
    if trimmed.eq_ignore_ascii_case("auto") {
        return None;
    }
    let number = trimmed
        .strip_suffix("px")
        .unwrap_or(trimmed)
        .trim()
        .parse::<f32>()
        .ok()?;
    if number.is_finite() {
        Some(number.max(0.0))
    } else {
        None
    }
}

fn parse_color(value: &str) -> Option<Color> {
    let raw = value.trim().to_ascii_lowercase();
    let value = raw
        .split_whitespace()
        .find(|part| {
            part.starts_with('#') || part.starts_with("rgb") || named_color(part).is_some()
        })
        .unwrap_or(&raw);
    if let Some(hex) = value.strip_prefix('#') {
        return match hex.len() {
            3 => Some(Color::rgb(
                u8::from_str_radix(&hex[0..1].repeat(2), 16).ok()?,
                u8::from_str_radix(&hex[1..2].repeat(2), 16).ok()?,
                u8::from_str_radix(&hex[2..3].repeat(2), 16).ok()?,
            )),
            6 => Some(Color::rgb(
                u8::from_str_radix(&hex[0..2], 16).ok()?,
                u8::from_str_radix(&hex[2..4], 16).ok()?,
                u8::from_str_radix(&hex[4..6], 16).ok()?,
            )),
            _ => None,
        };
    }
    if let Some(inner) = value.strip_prefix("rgb(").and_then(|v| v.strip_suffix(')')) {
        let parts: Vec<u8> = inner
            .split(',')
            .filter_map(|p| p.trim().parse::<u8>().ok())
            .collect();
        if parts.len() >= 3 {
            return Some(Color::rgb(parts[0], parts[1], parts[2]));
        }
    }
    named_color(value)
}

fn named_color(value: &str) -> Option<Color> {
    Some(match value {
        "transparent" => Color::transparent(),
        "black" => Color::rgb(0, 0, 0),
        "white" => Color::rgb(255, 255, 255),
        "red" => Color::rgb(220, 38, 38),
        "green" => Color::rgb(22, 163, 74),
        "blue" => Color::rgb(37, 99, 235),
        "yellow" => Color::rgb(234, 179, 8),
        "orange" => Color::rgb(249, 115, 22),
        "gray" | "grey" => Color::rgb(107, 114, 128),
        _ => return None,
    })
}

fn decode_data_uri(src: &str) -> Option<Vec<u8>> {
    let data = src.strip_prefix("data:")?;
    let (_meta, body) = data.split_once(',')?;
    base64::engine::general_purpose::STANDARD.decode(body).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use obscura_dom::parse_html;

    #[test]
    fn png_capture_has_png_header() {
        let dom = parse_html("<html><body><h1>Hello</h1><p>World</p></body></html>");
        let data = capture_base64(&dom, &[], None, &CaptureOptions::default()).unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .unwrap();
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn jpeg_capture_has_jpeg_header() {
        let dom = parse_html("<html><body><p>Hello</p></body></html>");
        let opts = CaptureOptions {
            format: CaptureFormat::Jpeg,
            ..CaptureOptions::default()
        };
        let data = capture_base64(&dom, &[], None, &opts).unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .unwrap();
        assert_eq!(&bytes[..2], b"\xff\xd8");
    }

    #[test]
    fn clip_changes_encoded_dimensions() {
        let dom = parse_html("<html><body><p>Hello</p></body></html>");
        let opts = CaptureOptions {
            clip: Some(Clip {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 100.0,
            }),
            ..CaptureOptions::default()
        };
        let data = capture_base64(&dom, &[], None, &opts).unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .unwrap();
        let image = image::load_from_memory(&bytes).unwrap();
        assert_eq!((image.width(), image.height()), (200, 100));
    }

    #[test]
    fn display_none_changes_rendered_pixels() {
        let visible = parse_html("<html><body><p style=\"color:red\">Hidden</p></body></html>");
        let hidden =
            parse_html("<html><body><p style=\"display:none;color:red\">Hidden</p></body></html>");
        let opts = CaptureOptions {
            clip: Some(Clip {
                x: 0.0,
                y: 0.0,
                width: 160.0,
                height: 80.0,
            }),
            ..CaptureOptions::default()
        };
        let a = capture_base64(&visible, &[], None, &opts).unwrap();
        let b = capture_base64(&hidden, &[], None, &opts).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn class_selector_applies_background() {
        let dom = parse_html("<html><body><div class=\"card\">Hello</div></body></html>");
        let opts = CaptureOptions {
            clip: Some(Clip {
                x: 0.0,
                y: 0.0,
                width: 120.0,
                height: 80.0,
            }),
            ..CaptureOptions::default()
        };
        let css = ".card { background-color: #0000ff; padding: 8px; }".to_string();
        let data = capture_base64(&dom, &[css], None, &opts).unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .unwrap();
        let image = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert!(image.pixels().any(|p| p[2] > 180 && p[0] < 80));
    }
}
