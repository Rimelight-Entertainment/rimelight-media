//! Pictures: open (PNG, JPEG, WebP, GIF, BMP, ICO with the image crate; SVG with resvg), edit (rotate, flip, crop,
//! resize, burn markup in) and save. Pixels are straight (not premultiplied) RGBA.

use std::path::Path;
use std::sync::{Arc, OnceLock};

use resvg::tiny_skia;
use resvg::usvg;

/// An RGBA picture, rows packed.
#[derive(Clone, Debug)]
pub struct Picture {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

/// SVGs are drawn so their longer side is at least this many pixels (sharp when zoomed a little).
const SVG_MIN_SIDE: f32 = 2048.0;

fn fonts() -> Arc<usvg::fontdb::Database> {
    static DB: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        db.set_sans_serif_family("Segoe UI");
        Arc::new(db)
    })
    .clone()
}

fn svg_options() -> usvg::Options<'static> {
    let mut opt = usvg::Options::default();
    opt.fontdb = fonts();
    opt
}

/// HEIC, HEIF, HIF and AVIF (through FFmpeg).
fn is_heif(ext: &str) -> bool {
    matches!(ext, "heic" | "heif" | "hif" | "avif")
}

/// Camera RAW formats (through rawler).
fn is_raw(ext: &str) -> bool {
    matches!(
        ext,
        "dng" | "cr2" | "cr3" | "nef" | "nrw" | "arw" | "srf" | "sr2" | "raf" | "orf" | "rw2" | "pef" | "srw" | "3fr"
            | "erf" | "kdc" | "mrw" | "x3f" | "iiq" | "rwl"
    )
}

pub fn load(path: &Path) -> Result<Picture, String> {
    let ext = crate::files::ext_of(path);
    if ext == "svg" {
        return load_svg(path);
    }
    if is_heif(&ext) {
        return crate::picture_heif::load(path);
    }
    if is_raw(&ext) {
        return crate::picture_raw::load(path);
    }
    let reader = image::ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    use image::ImageDecoder as _;
    let orientation = decoder.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = image::DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    img.apply_orientation(orientation);
    let rgba = img.into_rgba8();
    Ok(Picture { w: rgba.width(), h: rgba.height(), rgba: rgba.into_raw() })
}

/// A small version of a picture file (filmstrip), at most w x h, upright.
pub fn thumbnail(path: &Path, w: u32, h: u32) -> Option<Picture> {
    if crate::files::ext_of(path) == "svg" {
        let pic = load_svg(path).ok()?;
        let s = (w as f32 / pic.w as f32).min(h as f32 / pic.h as f32).min(1.0);
        return Some(resize(&pic, ((pic.w as f32 * s) as u32).max(1), ((pic.h as f32 * s) as u32).max(1)));
    }
    let ext = crate::files::ext_of(path);
    if is_heif(&ext) || is_raw(&ext) {
        let pic = load(path).ok()?;
        let s = (w as f32 / pic.w as f32).min(h as f32 / pic.h as f32).min(1.0);
        return Some(resize(&pic, ((pic.w as f32 * s) as u32).max(1), ((pic.h as f32 * s) as u32).max(1)));
    }
    use image::ImageDecoder as _;
    let mut decoder = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.into_decoder().ok()?;
    let orientation = decoder.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = image::DynamicImage::from_decoder(decoder).ok()?.thumbnail(w, h);
    img.apply_orientation(orientation);
    let rgba = img.into_rgba8();
    Some(Picture { w: rgba.width(), h: rgba.height(), rgba: rgba.into_raw() })
}

/// An animated picture (GIF or WebP): every frame, full canvas, with how long it shows.
pub struct Animation {
    pub frames: Vec<(Picture, u32)>,
    /// True when frames were left out to stay under the memory limit.
    pub cut: bool,
}

/// Most memory an animation's frames may take (decoded RGBA).
const MAX_ANIMATION_BYTES: usize = 1_200_000_000;

/// The frames of an animated GIF or WebP; None for a still picture (or any other format).
pub fn load_animation(path: &Path) -> Option<Animation> {
    use image::AnimationDecoder as _;
    let file = std::io::BufReader::new(std::fs::File::open(path).ok()?);
    let frames: image::Frames<'_> = match crate::files::ext_of(path).as_str() {
        "gif" => image::codecs::gif::GifDecoder::new(file).ok()?.into_frames(),
        "webp" => {
            let decoder = image::codecs::webp::WebPDecoder::new(file).ok()?;
            if !decoder.has_animation() {
                return None;
            }
            decoder.into_frames()
        }
        _ => return None,
    };
    let mut out = Vec::new();
    let mut bytes = 0usize;
    let mut cut = false;
    for frame in frames {
        let Ok(frame) = frame else { break };
        let (num, den) = frame.delay().numer_denom_ms();
        let ms = if den == 0 { 0 } else { num / den };
        // Browsers show 0 and 10 ms delays as 100 ms; so does this.
        let ms = if ms <= 10 { 100 } else { ms };
        let buf = frame.into_buffer();
        bytes += buf.as_raw().len();
        if bytes > MAX_ANIMATION_BYTES {
            cut = true;
            break;
        }
        out.push((Picture { w: buf.width(), h: buf.height(), rgba: buf.into_raw() }, ms));
    }
    (out.len() > 1).then_some(Animation { frames: out, cut })
}

fn load_svg(path: &Path) -> Result<Picture, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let tree = usvg::Tree::from_data(&data, &svg_options()).map_err(|e| e.to_string())?;
    let size = tree.size();
    let scale = (SVG_MIN_SIDE / size.width().max(size.height())).max(1.0);
    let w = (size.width() * scale).ceil().max(1.0) as u32;
    let h = (size.height() * scale).ceil().max(1.0) as u32;
    let mut pixmap = tiny_skia::Pixmap::new(w, h).ok_or("the drawing is too large")?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    Ok(from_pixmap(&pixmap))
}

fn from_pixmap(p: &tiny_skia::Pixmap) -> Picture {
    let mut rgba = Vec::with_capacity(p.data().len());
    for px in p.pixels() {
        let c = px.demultiply();
        rgba.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    Picture { w: p.width(), h: p.height(), rgba }
}

fn to_pixmap(pic: &Picture) -> Option<tiny_skia::Pixmap> {
    let mut data = Vec::with_capacity(pic.rgba.len());
    for c in pic.rgba.chunks_exact(4) {
        let a = u16::from(c[3]);
        let m = |v: u8| ((u16::from(v) * a + 127) / 255) as u8;
        data.extend_from_slice(&[m(c[0]), m(c[1]), m(c[2]), c[3]]);
    }
    tiny_skia::Pixmap::from_vec(data, tiny_skia::IntSize::from_wh(pic.w, pic.h)?)
}

pub fn to_slint(pic: &Picture) -> slint::Image {
    let buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(&pic.rgba, pic.w, pic.h);
    slint::Image::from_rgba8(buf)
}

fn as_image(pic: &Picture) -> image::RgbaImage {
    image::RgbaImage::from_raw(pic.w, pic.h, pic.rgba.clone()).unwrap_or_else(|| image::RgbaImage::new(1, 1))
}

fn from_image(img: image::RgbaImage) -> Picture {
    Picture { w: img.width(), h: img.height(), rgba: img.into_raw() }
}

pub fn rotate_cw(pic: &Picture) -> Picture {
    from_image(image::imageops::rotate90(&as_image(pic)))
}

pub fn flip(pic: &Picture, left_right: bool) -> Picture {
    let img = as_image(pic);
    from_image(if left_right { image::imageops::flip_horizontal(&img) } else { image::imageops::flip_vertical(&img) })
}

/// Crops to a box given as fractions of the picture.
pub fn crop(pic: &Picture, fx: f32, fy: f32, fw: f32, fh: f32) -> Picture {
    let x = ((fx.clamp(0.0, 1.0) * pic.w as f32).round() as u32).min(pic.w.saturating_sub(1));
    let y = ((fy.clamp(0.0, 1.0) * pic.h as f32).round() as u32).min(pic.h.saturating_sub(1));
    let w = ((fw * pic.w as f32).round() as u32).clamp(1, pic.w - x);
    let h = ((fh * pic.h as f32).round() as u32).clamp(1, pic.h - y);
    crop_px(pic, x, y, w, h)
}

pub fn crop_px(pic: &Picture, x: u32, y: u32, w: u32, h: u32) -> Picture {
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for row in y..y + h {
        let start = ((row * pic.w + x) * 4) as usize;
        rgba.extend_from_slice(&pic.rgba[start..start + (w * 4) as usize]);
    }
    Picture { w, h, rgba }
}

pub fn resize(pic: &Picture, w: u32, h: u32) -> Picture {
    let filter = if w < pic.w { image::imageops::FilterType::Lanczos3 } else { image::imageops::FilterType::CatmullRom };
    from_image(image::imageops::resize(&as_image(pic), w.max(1), h.max(1), filter))
}

/// One markup mark in picture pixels.
#[derive(Clone, Debug)]
pub struct Mark {
    /// 0 pen, 1 highlighter, 2 arrow, 3 box, 4 text, 5 picture, 6 ellipse, 7 line.
    pub kind: i32,
    pub rgba: [u8; 4],
    pub width: f32,
    pub points: Vec<(f32, f32)>,
    pub text: String,
    /// A box or an ellipse filled with its colour.
    pub filled: bool,
    /// A picture mark: its pixels, drawn into the rectangle from points[0] to points[1].
    pub image: Option<Arc<Picture>>,
}

impl Mark {
    /// The box round the mark: x, y, width, height (picture pixels).
    pub fn bounds(&self) -> (f32, f32, f32, f32) {
        let Some(&(x0, y0)) = self.points.first() else { return (0.0, 0.0, 0.0, 0.0) };
        if self.kind == 4 {
            return (x0, y0, self.font_size() * 0.6 * self.text.chars().count().max(1) as f32, self.font_size() * 1.3);
        }
        let (mut x1, mut y1, mut x2, mut y2) = (x0, y0, x0, y0);
        for &(x, y) in &self.points {
            x1 = x1.min(x);
            y1 = y1.min(y);
            x2 = x2.max(x);
            y2 = y2.max(y);
        }
        (x1, y1, x2 - x1, y2 - y1)
    }

    /// The path the mark draws, in the SVG path syntax Slint's Path also reads.
    pub fn commands(&self) -> String {
        let p = &self.points;
        if p.is_empty() {
            return String::new();
        }
        let (x0, y0) = p[0];
        let (x1, y1) = *p.last().unwrap_or(&p[0]);
        match self.kind {
            5 => String::new(),
            7 => format!("M {x0:.1} {y0:.1} L {x1:.1} {y1:.1}"),
            6 => {
                // An ellipse in four cubic curves inside the box from (x0, y0) to (x1, y1).
                const K: f32 = 0.5523;
                let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
                let (rx, ry) = ((x1 - x0).abs() / 2.0, (y1 - y0).abs() / 2.0);
                let (kx, ky) = (rx * K, ry * K);
                format!(
                    "M {cx:.1} {:.1} C {:.1} {:.1} {:.1} {:.1} {:.1} {cy:.1} C {:.1} {:.1} {:.1} {:.1} {cx:.1} {:.1} C {:.1} {:.1} {:.1} {:.1} {:.1} {cy:.1} C {:.1} {:.1} {:.1} {:.1} {cx:.1} {:.1} Z",
                    cy - ry,
                    cx + kx,
                    cy - ry,
                    cx + rx,
                    cy - ky,
                    cx + rx,
                    cx + rx,
                    cy + ky,
                    cx + kx,
                    cy + ry,
                    cy + ry,
                    cx - kx,
                    cy + ry,
                    cx - rx,
                    cy + ky,
                    cx - rx,
                    cx - rx,
                    cy - ky,
                    cx - kx,
                    cy - ry,
                    cy - ry,
                )
            }
            2 => {
                let ang = (y1 - y0).atan2(x1 - x0);
                let head = (self.width * 4.0).max(14.0);
                let a1 = ang + 2.6;
                let a2 = ang - 2.6;
                format!(
                    "M {x0:.1} {y0:.1} L {x1:.1} {y1:.1} M {:.1} {:.1} L {x1:.1} {y1:.1} L {:.1} {:.1}",
                    x1 + head * a1.cos(),
                    y1 + head * a1.sin(),
                    x1 + head * a2.cos(),
                    y1 + head * a2.sin()
                )
            }
            3 => format!("M {x0:.1} {y0:.1} L {x1:.1} {y0:.1} L {x1:.1} {y1:.1} L {x0:.1} {y1:.1} Z"),
            _ => {
                let mut s = format!("M {x0:.1} {y0:.1}");
                if p.len() == 1 {
                    s.push_str(&format!(" L {:.1} {y0:.1}", x0 + 0.1));
                }
                for (x, y) in &p[1..] {
                    s.push_str(&format!(" L {x:.1} {y:.1}"));
                }
                s
            }
        }
    }

    /// Stroke width as drawn (the highlighter is wide).
    pub fn stroke_width(&self) -> f32 {
        if self.kind == 1 { self.width * 4.0 } else { self.width }
    }

    /// Text size in picture pixels.
    pub fn font_size(&self) -> f32 {
        (self.width * 4.0).max(12.0)
    }

    /// Rough distance from (x, y) to the mark, for the eraser, the move tool and the picture marks (0 inside those).
    pub fn distance(&self, x: f32, y: f32) -> f32 {
        if self.points.is_empty() {
            return f32::MAX;
        }
        if self.kind == 4 || self.kind == 5 {
            let (tx, ty, w, h) = self.bounds();
            let dx = (tx - x).max(x - (tx + w)).max(0.0);
            let dy = (ty - y).max(y - (ty + h)).max(0.0);
            return dx.hypot(dy);
        }
        let pts: Vec<(f32, f32)> = if (self.kind == 3 || self.kind == 6) && self.points.len() >= 2 {
            let (x0, y0) = self.points[0];
            let (x1, y1) = *self.points.last().unwrap_or(&self.points[0]);
            vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)]
        } else if self.kind == 2 && self.points.len() >= 2 {
            vec![self.points[0], *self.points.last().unwrap_or(&self.points[0])]
        } else {
            self.points.clone()
        };
        if pts.len() == 1 {
            return (pts[0].0 - x).hypot(pts[0].1 - y);
        }
        pts.windows(2)
            .map(|s| seg_dist((x, y), s[0], s[1]))
            .fold(f32::MAX, f32::min)
    }
}

fn seg_dist(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    (a.0 + t * dx - p.0).hypot(a.1 + t * dy - p.1)
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Draws the marks into the picture, in order: vector marks through SVG, picture marks composited in place.
pub fn burn_marks(pic: &Picture, marks: &[Mark]) -> Result<Picture, String> {
    if marks.is_empty() {
        return Ok(pic.clone());
    }
    let mut out = pic.clone();
    let mut run: Vec<&Mark> = Vec::new();
    for m in marks {
        if m.kind == 5 {
            out = burn_run(&out, &run)?;
            run.clear();
            composite(&mut out, m);
        } else {
            run.push(m);
        }
    }
    burn_run(&out, &run)
}

/// The vector marks (no picture marks) drawn into the picture.
fn burn_run(pic: &Picture, marks: &[&Mark]) -> Result<Picture, String> {
    if marks.is_empty() {
        return Ok(pic.clone());
    }
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">",
        w = pic.w,
        h = pic.h
    );
    for m in marks {
        let [r, g, b, a] = m.rgba;
        let opacity = f32::from(a) / 255.0;
        if m.kind == 4 {
            let (x, y) = m.points.first().copied().unwrap_or_default();
            let size = m.font_size();
            svg.push_str(&format!(
                "<text x=\"{x}\" y=\"{}\" font-family=\"Segoe UI\" font-weight=\"600\" font-size=\"{size}\" fill=\"#{r:02x}{g:02x}{b:02x}\" fill-opacity=\"{opacity}\">{}</text>",
                y + size * 0.92,
                xml_escape(&m.text)
            ));
        } else {
            let fill = if m.filled { format!("#{r:02x}{g:02x}{b:02x}\" fill-opacity=\"{opacity}") } else { "none".to_string() };
            svg.push_str(&format!(
                "<path d=\"{}\" fill=\"{fill}\" stroke=\"#{r:02x}{g:02x}{b:02x}\" stroke-opacity=\"{opacity}\" stroke-width=\"{}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>",
                m.commands(),
                m.stroke_width()
            ));
        }
    }
    svg.push_str("</svg>");
    let tree = usvg::Tree::from_str(&svg, &svg_options()).map_err(|e| e.to_string())?;
    let mut pixmap = to_pixmap(pic).ok_or("the picture is empty")?;
    resvg::render(&tree, tiny_skia::Transform::identity(), &mut pixmap.as_mut());
    Ok(from_pixmap(&pixmap))
}

/// Puts a picture mark's pixels over the picture, scaled to its rectangle (alpha blended, straight alpha).
fn composite(out: &mut Picture, m: &Mark) {
    let Some(src) = m.image.as_deref() else { return };
    if m.points.len() < 2 {
        return;
    }
    let (x0, y0) = m.points[0];
    let (x1, y1) = m.points[m.points.len() - 1];
    let (left, top) = (x0.min(x1).round() as i64, y0.min(y1).round() as i64);
    let w = ((x1 - x0).abs().round() as u32).max(1);
    let h = ((y1 - y0).abs().round() as u32).max(1);
    let scaled = if w == src.w && h == src.h { src.clone() } else { resize(src, w, h) };
    for sy in 0..h {
        let dy = top + i64::from(sy);
        if dy < 0 || dy >= i64::from(out.h) {
            continue;
        }
        for sx in 0..w {
            let dx = left + i64::from(sx);
            if dx < 0 || dx >= i64::from(out.w) {
                continue;
            }
            let si = ((sy * w + sx) * 4) as usize;
            let di = ((dy as u32 * out.w + dx as u32) * 4) as usize;
            let a = u32::from(scaled.rgba[si + 3]);
            if a == 0 {
                continue;
            }
            let inv = 255 - a;
            for c in 0..3 {
                out.rgba[di + c] = ((u32::from(scaled.rgba[si + c]) * a + u32::from(out.rgba[di + c]) * inv + 127) / 255) as u8;
            }
            out.rgba[di + 3] = (a + u32::from(out.rgba[di + 3]) * inv / 255).min(255) as u8;
        }
    }
}

/// The picture with a rectangle (picture pixels) filled with one colour.
pub fn fill_rect(pic: &Picture, x: u32, y: u32, w: u32, h: u32, rgba: [u8; 4]) -> Picture {
    let mut out = pic.clone();
    for row in y..(y + h).min(pic.h) {
        for col in x..(x + w).min(pic.w) {
            let i = ((row * pic.w + col) * 4) as usize;
            out.rgba[i..i + 4].copy_from_slice(&rgba);
        }
    }
    out
}

/// Paint bucket: fills the area around (x, y) whose colours are within `tolerance` of the seed pixel (scanline fill).
/// A translucent fill colour is blended over the pixels.
pub fn flood_fill(pic: &Picture, x: u32, y: u32, fill: [u8; 4], tolerance: u8) -> Picture {
    let mut out = pic.clone();
    if x >= pic.w || y >= pic.h {
        return out;
    }
    let (w, h) = (pic.w as usize, pic.h as usize);
    let at = |px: usize, py: usize| (py * w + px) * 4;
    let seed = at(x as usize, y as usize);
    let target = [pic.rgba[seed], pic.rgba[seed + 1], pic.rgba[seed + 2], pic.rgba[seed + 3]];
    let tol = i32::from(tolerance);
    let similar = |i: usize| (0..4).all(|c| (i32::from(pic.rgba[i + c]) - i32::from(target[c])).abs() <= tol);
    let mut done = vec![false; w * h];
    let mut stack = vec![(x as usize, y as usize)];
    let a = u32::from(fill[3]);
    while let Some((sx, sy)) = stack.pop() {
        if done[sy * w + sx] || !similar(at(sx, sy)) {
            continue;
        }
        // Widen the run to the left and right of the seed.
        let mut l = sx;
        while l > 0 && !done[sy * w + l - 1] && similar(at(l - 1, sy)) {
            l -= 1;
        }
        let mut r = sx;
        while r + 1 < w && !done[sy * w + r + 1] && similar(at(r + 1, sy)) {
            r += 1;
        }
        for cx in l..=r {
            done[sy * w + cx] = true;
            let i = at(cx, sy);
            for c in 0..3 {
                out.rgba[i + c] = ((u32::from(fill[c]) * a + u32::from(pic.rgba[i + c]) * (255 - a) + 127) / 255) as u8;
            }
            out.rgba[i + 3] = pic.rgba[i + 3].max(fill[3]);
        }
        // Seeds on the rows above and below: one per run of matching pixels.
        for ny in [sy.wrapping_sub(1), sy + 1] {
            if ny >= h {
                continue;
            }
            let mut cx = l;
            while cx <= r {
                if !done[ny * w + cx] && similar(at(cx, ny)) {
                    stack.push((cx, ny));
                    while cx <= r && !done[ny * w + cx] && similar(at(cx, ny)) {
                        cx += 1;
                    }
                } else {
                    cx += 1;
                }
            }
        }
    }
    out
}

/// Can this extension be written back?
pub fn can_save(path: &Path) -> bool {
    matches!(crate::files::ext_of(path).as_str(), "png" | "jpg" | "jpeg" | "jfif" | "webp" | "bmp")
}

/// Saves by extension: PNG, JPEG (quality 92), WebP (lossless) or BMP. Anything else is saved as PNG.
pub fn save(pic: &Picture, path: &Path) -> Result<(), String> {
    let img = as_image(pic);
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut out = std::io::BufWriter::new(file);
    let result = match crate::files::ext_of(path).as_str() {
        "jpg" | "jpeg" | "jfif" => {
            let rgb = image::DynamicImage::ImageRgba8(img).into_rgb8();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 92)
                .encode_image(&rgb)
        }
        "webp" => image::codecs::webp::WebPEncoder::new_lossless(&mut out).encode(
            img.as_raw(),
            img.width(),
            img.height(),
            image::ExtendedColorType::Rgba8,
        ),
        "bmp" => image::DynamicImage::ImageRgba8(img).write_to(&mut out, image::ImageFormat::Bmp),
        _ => image::DynamicImage::ImageRgba8(img).write_to(&mut out, image::ImageFormat::Png),
    };
    result.map_err(|e| e.to_string())?;
    use std::io::Write as _;
    out.flush().map_err(|e| e.to_string())
}

/// Puts the picture on the clipboard.
pub fn copy_to_clipboard(pic: &Picture) -> Result<(), String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    cb.set_image(arboard::ImageData {
        width: pic.w as usize,
        height: pic.h as usize,
        bytes: std::borrow::Cow::Borrowed(&pic.rgba),
    })
    .map_err(|e| e.to_string())
}

/// The app icon (the logo drawn at `size`), for the tray-less places that need pixels: the .ico file at install.
pub fn logo(size: u32) -> Option<Picture> {
    render_logo(include_str!("../assets/icons/logo-color.svg"), size)
}

/// The logo with its gradient in other colours (one per kind of file in Explorer).
pub fn logo_tinted(size: u32, top: &str, bottom: &str) -> Option<Picture> {
    let svg = include_str!("../assets/icons/logo-color.svg").replace("#4DA0FF", top).replace("#1659C4", bottom);
    render_logo(&svg, size)
}

fn render_logo(svg: &str, size: u32) -> Option<Picture> {
    let data = svg.as_bytes();
    let tree = usvg::Tree::from_data(data, &usvg::Options::default()).ok()?;
    let scale = size as f32 / tree.size().width();
    let mut pixmap = tiny_skia::Pixmap::new(size, size)?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    Some(from_pixmap(&pixmap))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checker(w: u32, h: u32) -> Picture {
        let mut rgba = Vec::new();
        for y in 0..h {
            for x in 0..w {
                rgba.extend_from_slice(&[(x * 10) as u8, (y * 10) as u8, 0, 255]);
            }
        }
        Picture { w, h, rgba }
    }

    #[test]
    fn rotate_crop_resize() {
        let p = checker(4, 2);
        let r = rotate_cw(&p);
        assert_eq!((r.w, r.h), (2, 4));
        let c = crop(&p, 0.5, 0.0, 0.5, 1.0);
        assert_eq!((c.w, c.h), (2, 2));
        assert_eq!(c.rgba[0], 20);
        let s = resize(&p, 8, 4);
        assert_eq!((s.w, s.h), (8, 4));
    }

    #[test]
    fn marks_burn_in() {
        let p = Picture { w: 40, h: 40, rgba: vec![255; 40 * 40 * 4] };
        let m = Mark { kind: 3, rgba: [255, 0, 0, 255], width: 4.0, points: vec![(5.0, 5.0), (35.0, 35.0)], text: String::new(), filled: false, image: None };
        let out = burn_marks(&p, &[m]).unwrap();
        let px = &out.rgba[((5 * 40 + 20) * 4) as usize..][..4];
        assert!(px[0] > 200 && px[1] < 80, "the box edge is red: {px:?}");
    }
}
