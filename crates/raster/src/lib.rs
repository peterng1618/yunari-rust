#![forbid(unsafe_code)]
//! The raster backend: a display list becomes pixels.
//!
//! Panels receive frames from here, and so does every golden test, so the
//! geometry a theme paints is checked on the path a device actually draws.

use tiny_skia::{
    Color as SkiaColor, FillRule, LineCap, LineJoin, LinearGradient, Paint as SkiaPaint, Path,
    PathBuilder, Pixmap, PixmapPaint, Point, Rect, Shader, SpreadMode, Stroke as SkiaStroke,
    Transform,
};

mod chart;
mod images;
pub mod sidecar;
pub use chart::ChartSamples;
pub use images::{Decoded, Images};

use yunari_render::text::{
    ShapeBuffer, SwashCache, TextColor, TextLayout, TextMetrics, TextSegment, TextShaping, attrs,
};
use yunari_render::{Affine, Color, DisplayList, Fonts, Op, Paint, PathCommand, PlanIssue, Shape};

/// A rendered frame in straight (not premultiplied) RGBA8.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Draw the list at `scale` device pixels per artboard unit.
///
/// Text needs a font database and pictures need decoded bytes; passing neither
/// draws everything else and leaves those ops absent, which is what a frame
/// with no assets to read should show.
pub fn render(
    list: &DisplayList,
    scale: f32,
    fonts: Option<&mut Fonts>,
    images: Option<&mut Images>,
) -> Raster {
    render_with_charts(list, scale, fonts, images, None, &ChartSamples::empty()).0
}

/// Render with the readings and window samples charts need, returning the
/// reasons a mark or a paint could not be drawn. A chart op with no samples
/// still draws the structure its settings describe, never a fabricated zero.
///
/// The document supplies paint references and binding names; a backend with no
/// document can only draw settings that carry their own colours.
pub fn render_with_charts(
    list: &DisplayList,
    scale: f32,
    mut fonts: Option<&mut Fonts>,
    mut images: Option<&mut Images>,
    document: Option<&yunari_model::document::ThemeDocument>,
    samples: &ChartSamples<'_>,
) -> (Raster, Vec<PlanIssue>) {
    let mut issues = Vec::new();
    // Expanded chart marks are painted through the same paths as any other
    // op, so they sit in z-order exactly where the chart op sat.
    let mut ops: Vec<Op> = Vec::with_capacity(list.ops.len());
    for op in &list.ops {
        if let Shape::Chart {
            id,
            family,
            settings,
            bindings,
            width,
            height,
            opacity,
        } = &op.shape
        {
            ops.extend(chart::expand(
                id,
                *family,
                settings,
                bindings,
                *width,
                *height,
                *opacity,
                op.transform,
                document,
                samples,
                fonts.as_deref_mut(),
                &mut issues,
            ));
        } else {
            ops.push(op.clone());
        }
    }
    let width = (list.width * scale).round().max(0.0) as u32;
    let height = (list.height * scale).round().max(0.0) as u32;
    let mut pixmap = match Pixmap::new(width, height) {
        Some(pixmap) => pixmap,
        None => {
            return (
                Raster {
                    width: 0,
                    height: 0,
                    rgba: Vec::new(),
                },
                issues,
            );
        }
    };

    if let Some(paint) = &list.background {
        pixmap.fill(SkiaColor::TRANSPARENT);
        let board = PathBuilder::from_rect(
            Rect::from_xywh(0.0, 0.0, width as f32, height as f32).expect("a sized board"),
        );
        fill(
            &mut pixmap,
            &board,
            paint,
            Transform::from_scale(scale, scale),
        );
    } else {
        pixmap.fill(SkiaColor::TRANSPARENT);
    }

    if let (Some(backdrop), Some(store)) = (&list.backdrop, images.as_deref_mut())
        && let Some(asset) = store.load(&backdrop.path)
    {
        draw_backdrop(&mut pixmap, asset, list, backdrop, scale);
    }

    for op in &ops {
        if let Shape::Text(layout) = &op.shape {
            if let Some(fonts) = fonts.as_deref_mut() {
                draw_text(&mut pixmap, layout, op.transform, scale, fonts);
            }
            continue;
        }
        if let Shape::Glass { shape, blur_radius } = &op.shape {
            draw_glass(&mut pixmap, shape, *blur_radius, op.transform, scale);
            continue;
        }
        if let Shape::Image {
            path,
            width,
            height,
            fit,
            monochrome,
            video,
            video_loop,
            ..
        } = &op.shape
        {
            let Some(asset) = images
                .as_deref_mut()
                .and_then(|store| store.load_clip(path, !*video || *video_loop))
            else {
                continue;
            };
            draw_image(
                &mut pixmap,
                asset,
                (*width, *height),
                *fit,
                *monochrome,
                op.transform,
                scale,
            );
            continue;
        }
        let Some(path) = shape_path(&op.shape) else {
            continue;
        };
        let transform = transform_of(op.transform).pre_scale(scale, scale);
        if let Some(paint) = &op.fill {
            fill(&mut pixmap, &path, paint, transform);
        }
        if let Some(stroke) = &op.stroke
            && stroke.width > 0.0
            && let Some(shader) = shader(&stroke.paint)
        {
            let skia_paint = SkiaPaint {
                shader,
                anti_alias: true,
                ..SkiaPaint::default()
            };
            let skia_stroke = SkiaStroke {
                width: stroke.width * scale,
                line_cap: LineCap::Round,
                line_join: LineJoin::Round,
                ..SkiaStroke::default()
            };
            pixmap.stroke_path(&path, &skia_paint, &skia_stroke, transform, None);
        }
    }

    (
        Raster {
            width,
            height,
            rgba: straight_rgba(&pixmap),
        },
        issues,
    )
}

/// Paint a laid-out paragraph. Each segment is shaped at device scale into its
/// own pixmap and composited through the op's transform, so a rotated or
/// scaled text node carries its glyphs with it.
fn draw_text(
    pixmap: &mut Pixmap,
    layout: &TextLayout,
    transform: Affine,
    scale: f32,
    fonts: &mut Fonts,
) {
    let mut cache = SwashCache::new();
    for segment in &layout.segments {
        let size = (segment.style.size * scale).max(1.0);
        let mut buffer = ShapeBuffer::new(fonts.system_mut(), TextMetrics::new(size, size * 1.2));
        let mut extent = (0.0_f32, 0.0_f32);
        {
            let mut buffer = buffer.borrow_with(fonts.system_mut());
            buffer.set_size(None, None);
            buffer.set_text(
                &segment.text,
                &attrs(&segment.style, layout.line_height),
                TextShaping::Advanced,
                None,
            );
            for run in buffer.layout_runs() {
                extent.0 = extent.0.max(run.line_w);
                extent.1 = extent.1.max(run.line_top + run.line_height);
            }
        }
        // The layout shaped the whole paragraph at the largest run's size, so
        // every segment shares its line's baseline. Reshaping one segment at
        // its own size gives a different ascent: a 15px `%` drawn from the
        // same line top lands far above a 70px value. Measure the single
        // segment's own baseline and move its plane down by the difference.
        let own_baseline = {
            let mut buffer = buffer.borrow_with(fonts.system_mut());
            buffer.layout_runs().next().map_or(0.0, |run| run.line_y)
        };
        let baseline_shift = ((segment.line_y - own_baseline) * scale).round() as i32;
        let width = extent.0.ceil().max(1.0) as u32 + 2;
        let height = (extent.1.ceil().max(1.0) as i32 + 2 + baseline_shift.max(0)).max(1) as u32;
        let Some(mut glyphs) = Pixmap::new(width, height) else {
            continue;
        };
        {
            let mut buffer = buffer.borrow_with(fonts.system_mut());
            buffer.draw(&mut cache, ink(segment), |x, y, _, _, pixel| {
                blend(&mut glyphs, x, y, pixel);
            });
        }
        let place = text_transform(transform, segment, scale);
        let place = place.post_translate(0.0, baseline_shift as f32);
        pixmap.draw_pixmap(0, 0, glyphs.as_ref(), &PixmapPaint::default(), place, None);
    }
}

/// The tint a segment's pixels multiply against. Alpha lives in the style's
/// colour, so the shaper keeps it.
fn ink(segment: &TextSegment) -> TextColor {
    TextColor::rgba(
        segment.style.color.r,
        segment.style.color.g,
        segment.style.color.b,
        segment.style.color.a,
    )
}

/// Where a segment's device-scale pixmap lands: the pixel grid it was shaped on
/// is device pixels already, so only the box-to-device transform applies.
fn text_transform(transform: Affine, segment: &TextSegment, scale: f32) -> Transform {
    let (x, y) = segment.origin;
    let x = x * scale;
    let y = y * scale;
    Transform::from_row(
        transform.a,
        transform.b,
        transform.c,
        transform.d,
        transform.a * x + transform.c * y + transform.e * scale,
        transform.b * x + transform.d * y + transform.f * scale,
    )
}

/// Source-over in the premultiplied space tiny-skia stores.
fn blend(pixmap: &mut Pixmap, x: i32, y: i32, color: TextColor) {
    if x < 0 || y < 0 {
        return;
    }
    let (x, y) = (x as u32, y as u32);
    if x >= pixmap.width() || y >= pixmap.height() {
        return;
    }
    let alpha = f32::from(color.a()) / 255.0;
    if alpha <= 0.0 {
        return;
    }
    let index = (y * pixmap.width() + x) as usize;
    let pixels = pixmap.pixels_mut();
    let Some(destination) = pixels.get_mut(index) else {
        return;
    };
    let under = destination.demultiply();
    let (under_r, under_g, under_b) = (
        f32::from(under.red()) / 255.0,
        f32::from(under.green()) / 255.0,
        f32::from(under.blue()) / 255.0,
    );
    let keep = f32::from(under.alpha()) / 255.0 * (1.0 - alpha);
    let alpha_out = alpha + keep;
    let channel = |source: u8, under: f32| -> u8 {
        // Premultiplied: the source's colour scales by its own alpha, and the
        // destination is already premultiplied.
        let value = f32::from(source) / 255.0 * alpha + under * keep;
        (value.clamp(0.0, 1.0) * 255.0).round() as u8
    };
    let alpha_out_byte = (alpha_out.clamp(0.0, 1.0) * 255.0).round() as u8;
    if let Some(pixel) = tiny_skia::PremultipliedColorU8::from_rgba(
        channel(color.r(), under_r),
        channel(color.g(), under_g),
        channel(color.b(), under_b),
        alpha_out_byte,
    ) {
        *destination = pixel;
    }
}

/// Frost the shape's interior: whatever is painted beneath it, blurred, clipped
/// to the same shape.
///
/// The blur is three box passes over the premultiplied region, which is a
/// Gaussian to the eye at a fraction of a real one's cost. The region is read
/// back out of the frame that is already painted, so a panel sees exactly its
/// own backdrop — not a canvas-wide copy — and costs one pass per changed
/// frame.
fn draw_glass(pixmap: &mut Pixmap, shape: &Shape, blur_radius: f32, transform: Affine, scale: f32) {
    let Some(path) = shape_path(shape) else {
        return;
    };
    let to_device = transform.then(Affine::scale(scale, scale));
    let Some((min, max)) = shape_bounds(shape) else {
        return;
    };
    let corner_a = to_device.apply(min.0, min.1);
    let corner_b = to_device.apply(max.0, max.1);
    let left = corner_a.0.min(corner_b.0).floor() as i32;
    let top = corner_a.1.min(corner_b.1).floor() as i32;
    let right = (corner_a.0.max(corner_b.0).ceil() as i32).min(pixmap.width() as i32);
    let bottom = (corner_a.1.max(corner_b.1).ceil() as i32).min(pixmap.height() as i32);
    if right <= left || bottom <= top {
        return;
    }
    // The source renderer samples a surround (`ceil(2r) + 2`), not the panel
    // box. Clamping the sample at the box gives the blur nothing outside the
    // panel to diffuse, so the edges read flat. Pad before extraction and put
    // the pattern back at the padded corner.
    let device_radius = device_blur_radius(blur_radius, transform, scale);
    let pad = (2.0 * device_radius).ceil() as i32 + 2;
    let sample_left = (left - pad).max(0);
    let sample_top = (top - pad).max(0);
    let sample_right = (right + pad).min(pixmap.width() as i32);
    let sample_bottom = (bottom + pad).min(pixmap.height() as i32);
    let Some(mut region) = extract(
        pixmap,
        sample_left,
        sample_top,
        (sample_right - sample_left) as usize,
        (sample_bottom - sample_top) as usize,
    ) else {
        return;
    };
    // CSS reads a blur radius as twice the deviation, and three box passes of
    // radius `r` reach about that deviation.
    blur(&mut region, device_radius / 2.0);
    // The pattern places the extracted region in device space. tiny-skia's
    // `fill_path` post-concats the fill transform onto a shader, so handing it
    // the device-space corner would apply the panel's translate and scale twice
    // and sample the wrong picture: a card at (-20, 198) would read from
    // (-40, 396). Pre-divide that transform out. Affine inverses are exact here
    // (the panel is non-singular or `fill_path` would paint nothing), so the
    // composed transform is the identity and the region lands where it was cut.
    let place = Transform::from_translate(sample_left as f32, sample_top as f32);
    let pattern = tiny_skia::Pattern::new(
        region.as_ref(),
        SpreadMode::Pad,
        tiny_skia::FilterQuality::Bilinear,
        1.0,
        to_device.inverse().map_or(place, |inverse| {
            Transform::from_row(
                inverse.a, inverse.b, inverse.c, inverse.d, inverse.e, inverse.f,
            )
            .post_concat(place)
        }),
    );
    let mut paint = SkiaPaint {
        shader: pattern,
        anti_alias: true,
        ..SkiaPaint::default()
    };
    paint.blend_mode = tiny_skia::BlendMode::SourceOver;
    pixmap.fill_path(
        &path,
        &paint,
        FillRule::Winding,
        transform_of(to_device),
        None,
    );
}

/// The blur radius in device pixels. The source uses the equal-area root of the
/// object's own matrix, so an anisotropic card blurs at one radius, not two.
fn device_blur_radius(blur_radius: f32, transform: Affine, scale: f32) -> f32 {
    blur_radius
        * (transform.a * transform.d - transform.b * transform.c)
            .abs()
            .sqrt()
        * scale
}

/// The box a shape's own geometry occupies, in its node's coordinates.
fn shape_bounds(shape: &Shape) -> Option<((f32, f32), (f32, f32))> {
    match shape {
        Shape::Rect { width, height, .. } | Shape::Ellipse { width, height } => {
            Some(((0.0, 0.0), (*width, *height)))
        }
        Shape::Path { commands, .. } => {
            let mut min = (f32::INFINITY, f32::INFINITY);
            let mut max = (f32::NEG_INFINITY, f32::NEG_INFINITY);
            let mut note = |x: f32, y: f32| {
                min = (min.0.min(x), min.1.min(y));
                max = (max.0.max(x), max.1.max(y));
            };
            for command in commands {
                match *command {
                    PathCommand::MoveTo(x, y) | PathCommand::LineTo(x, y) => note(x, y),
                    PathCommand::QuadTo(cx, cy, x, y) => {
                        note(cx, cy);
                        note(x, y);
                    }
                    PathCommand::CubicTo(cx1, cy1, cx2, cy2, x, y) => {
                        note(cx1, cy1);
                        note(cx2, cy2);
                        note(x, y);
                    }
                    PathCommand::Close => {}
                }
            }
            (min.0.is_finite() && max.0.is_finite()).then_some((min, max))
        }
        Shape::Line { length } => Some(((0.0, 0.0), (*length, 0.0))),
        Shape::Text(_) | Shape::Glass { .. } | Shape::Image { .. } | Shape::Chart { .. } => None,
    }
}

/// The frame's pixels under a rectangle, as their own pixmap. Missing pixels
/// stay transparent, which is what a panel hanging off the artboard samples.
fn extract(pixmap: &Pixmap, left: i32, top: i32, width: usize, height: usize) -> Option<Pixmap> {
    let mut region = Pixmap::new(width as u32, height as u32)?;
    let frame_width = pixmap.width() as i32;
    let frame_height = pixmap.height() as i32;
    let stride = frame_width as usize * 4;
    for y in 0..height as i32 {
        let row = top + y;
        if row < 0 || row >= frame_height {
            continue;
        }
        for x in 0..width as i32 {
            let column = left + x;
            if column < 0 || column >= frame_width {
                continue;
            }
            let source = row as usize * stride + column as usize * 4;
            let target = y as usize * width * 4 + x as usize * 4;
            region.data_mut()[target..target + 4]
                .copy_from_slice(&pixmap.data()[source..source + 4]);
        }
    }
    Some(region)
}

/// Three box passes over the premultiplied pixels: a Gaussian to the eye, and
/// the radius a CSS blur of this size reads as.
fn blur(pixmap: &mut Pixmap, deviation: f32) {
    let radius = deviation.round().max(0.0) as usize;
    if radius == 0 {
        return;
    }
    let width = pixmap.width() as usize;
    let height = pixmap.height() as usize;
    if width == 0 || height == 0 {
        return;
    }
    let mut buffer: Vec<u8> = pixmap.data().to_vec();
    let mut scratch = buffer.clone();
    for _ in 0..3 {
        box_blur(&buffer, &mut scratch, width, height, radius, true);
        box_blur(&scratch, &mut buffer, width, height, radius, false);
    }
    let copy = buffer.len().min(pixmap.data().len());
    pixmap.data_mut()[..copy].copy_from_slice(&buffer[..copy]);
}

/// One box pass along rows (`horizontal`) or columns, on premultiplied bytes.
fn box_blur(
    source: &[u8],
    target: &mut [u8],
    width: usize,
    height: usize,
    radius: usize,
    horizontal: bool,
) {
    let span = radius * 2 + 1;
    let (outer, inner) = if horizontal {
        (height, width)
    } else {
        (width, height)
    };
    for line in 0..outer {
        for channel in 0..4 {
            let at = |step: usize| -> usize {
                let (x, y) = if horizontal {
                    (step, line)
                } else {
                    (line, step)
                };
                (y * width + x) * 4 + channel
            };
            let mut total: u32 = 0;
            for step in -(radius as isize)..=(radius as isize) {
                total += u32::from(source[at(step.clamp(0, inner as isize - 1) as usize)]);
            }
            for step in 0..inner {
                target[at(step)] = (total / span as u32) as u8;
                let leaving = (step as isize - radius as isize).clamp(0, inner as isize - 1);
                let entering = (step as isize + radius as isize + 1).clamp(0, inner as isize - 1);
                total = total
                    .saturating_sub(u32::from(source[at(leaving as usize)]))
                    .saturating_add(u32::from(source[at(entering as usize)]));
            }
        }
    }
}

/// Paint the page's picture where the composition puts it, faded into the fill
/// by its ramps. A planner that was handed decoded source sizes resolved the
/// media already; otherwise the backend resolves it from the bytes it just
/// decoded, so a readable picture never disappears from a frame.
fn draw_backdrop(
    pixmap: &mut Pixmap,
    asset: &Decoded,
    list: &DisplayList,
    backdrop: &yunari_render::Backdrop,
    scale: f32,
) {
    if asset.width == 0 || asset.height == 0 {
        return;
    }
    let resolved;
    let placed = if backdrop.resolved.media.is_some() {
        &backdrop.resolved
    } else {
        resolved = yunari_render::background::resolve(
            (list.width, list.height),
            backdrop.composition.as_ref(),
            Some((asset.width as f32, asset.height as f32)),
            backdrop.geometry,
        );
        &resolved
    };
    let Some(media) = placed.media else {
        return;
    };
    let (cover, dest) = (media.cover, media.dest);
    if cover.width <= 0.0 || cover.height <= 0.0 {
        return;
    }
    let left = (dest.left * scale).floor().max(0.0) as i64;
    let top = (dest.top * scale).floor().max(0.0) as i64;
    let right = ((dest.left + dest.width) * scale)
        .ceil()
        .min(pixmap.width() as f32) as i64;
    let bottom = ((dest.top + dest.height) * scale)
        .ceil()
        .min(pixmap.height() as f32) as i64;
    for y in top..bottom {
        for x in left..right {
            let (box_x, box_y) = ((x as f32 + 0.5) / scale, (y as f32 + 0.5) / scale);
            let u = (box_x - cover.left) / cover.width;
            let v = (box_y - cover.top) / cover.height;
            let alpha = fade_alpha(&placed.masks, box_x, box_y);
            if alpha <= 0.0 {
                continue;
            }
            let u = if backdrop.flip_x { 1.0 - u } else { u };
            let v = if backdrop.flip_y { 1.0 - v } else { v };
            let texel = sample(asset, u.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
            let scaled_alpha = (f32::from(texel[3]) * alpha).round() as u8;
            blend(
                pixmap,
                x as i32,
                y as i32,
                TextColor::rgba(texel[0], texel[1], texel[2], scaled_alpha),
            );
        }
    }
}

/// The ramps' combined alpha at a point: each mask is transparent on one side
/// of its axis and opaque on the other, and the layers add.
fn fade_alpha(masks: &[yunari_render::background::Mask], x: f32, y: f32) -> f32 {
    if masks.is_empty() {
        return 1.0;
    }
    let mut alpha = 0.0;
    for mask in masks {
        let run_x = mask.x2 - mask.x1;
        let run_y = mask.y2 - mask.y1;
        let span = run_x * run_x + run_y * run_y;
        let t = if span <= f32::EPSILON {
            1.0
        } else {
            ((x - mask.x1) * run_x + (y - mask.y1) * run_y) / span
        };
        alpha += yunari_render::background::blend_ramp(t);
    }
    alpha.min(1.0)
}

/// Paint a picture into its box, fitted and clipped, sampled per device pixel.
///
/// The picture is resampled here rather than scaled through tiny-skia's pixmap
/// transform, so `contain`'s empty margin and `cover`'s overflow are the same
/// rectangle the display list computed, whatever the node's turn or scale.
fn draw_image(
    pixmap: &mut Pixmap,
    asset: &Decoded,
    box_: (f32, f32),
    fit: yunari_model::document::ImageFit,
    monochrome: Option<Color>,
    transform: Affine,
    scale: f32,
) {
    if asset.width == 0 || asset.height == 0 {
        return;
    }
    let place = yunari_render::placement((asset.width as f32, asset.height as f32), box_, fit);
    if place.width <= 0.0 || place.height <= 0.0 {
        return;
    }
    let to_device = transform.then(Affine::scale(scale, scale));
    let Some(to_box) = to_device.inverse() else {
        return;
    };
    let corners = [(0.0, 0.0), (box_.0, 0.0), (0.0, box_.1), (box_.0, box_.1)];
    let mut low = (f32::INFINITY, f32::INFINITY);
    let mut high = (f32::NEG_INFINITY, f32::NEG_INFINITY);
    for (x, y) in corners {
        let device = to_device.apply(x, y);
        low = (low.0.min(device.0), low.1.min(device.1));
        high = (high.0.max(device.0), high.1.max(device.1));
    }
    let left = low.0.floor().max(0.0) as i64;
    let top = low.1.floor().max(0.0) as i64;
    let right = high.0.ceil().min(pixmap.width() as f32) as i64;
    let bottom = high.1.ceil().min(pixmap.height() as f32) as i64;
    for y in top..bottom {
        for x in left..right {
            let (box_x, box_y) = to_box.apply(x as f32 + 0.5, y as f32 + 0.5);
            if box_x < 0.0 || box_y < 0.0 || box_x > box_.0 || box_y > box_.1 {
                continue;
            }
            let u = (box_x - place.x) / place.width;
            let v = (box_y - place.y) / place.height;
            if !(-0.002..=1.002).contains(&u) || !(-0.002..=1.002).contains(&v) {
                continue;
            }
            let texel = sample(asset, u.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
            let colour = match monochrome {
                // A mask: the picture's shape is its alpha, its colour is the
                // paint, and the paint's own alpha still applies.
                Some(tint) => TextColor::rgba(
                    tint.r,
                    tint.g,
                    tint.b,
                    (u16::from(texel[3]) * u16::from(tint.a) / 255) as u8,
                ),
                None => TextColor::rgba(texel[0], texel[1], texel[2], texel[3]),
            };
            blend(pixmap, x as i32, y as i32, colour);
        }
    }
}

/// One texel at a point in the picture, bilinearly interpolated so a scaled
/// picture does not shimmer.
fn sample(asset: &Decoded, u: f32, v: f32) -> [u8; 4] {
    let x = (u * asset.width as f32 - 0.5).clamp(0.0, (asset.width - 1) as f32);
    let y = (v * asset.height as f32 - 0.5).clamp(0.0, (asset.height - 1) as f32);
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(asset.width - 1);
    let y1 = (y0 + 1).min(asset.height - 1);
    let fx = x - x0 as f32;
    let fy = y - y0 as f32;
    let mut texel = [0_u8; 4];
    for (channel, byte) in texel.iter_mut().enumerate() {
        let at = |px: u32, py: u32| {
            f32::from(asset.rgba[((py * asset.width + px) * 4) as usize + channel])
        };
        let top = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * fx;
        let bottom = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * fx;
        *byte = (top + (bottom - top) * fy).round().clamp(0.0, 255.0) as u8;
    }
    texel
}

fn fill(pixmap: &mut Pixmap, path: &Path, paint: &Paint, transform: Transform) {
    let Some(shader) = shader(paint) else {
        return;
    };
    let skia_paint = SkiaPaint {
        shader,
        anti_alias: true,
        ..SkiaPaint::default()
    };
    pixmap.fill_path(path, &skia_paint, FillRule::Winding, transform, None);
}

fn transform_of(affine: Affine) -> Transform {
    Transform::from_row(affine.a, affine.b, affine.c, affine.d, affine.e, affine.f)
}

fn shader(paint: &Paint) -> Option<Shader<'_>> {
    match paint {
        Paint::Solid(color) => Some(Shader::SolidColor(SkiaColor::from_rgba8(
            color.r, color.g, color.b, color.a,
        ))),
        Paint::Linear { from, to, stops } => {
            let stops: Vec<tiny_skia::GradientStop> = stops
                .iter()
                .map(|stop| {
                    tiny_skia::GradientStop::new(
                        stop.offset,
                        SkiaColor::from_rgba8(
                            stop.color.r,
                            stop.color.g,
                            stop.color.b,
                            stop.color.a,
                        ),
                    )
                })
                .collect();
            LinearGradient::new(
                Point::from_xy(from.0, from.1),
                Point::from_xy(to.0, to.1),
                stops,
                SpreadMode::Pad,
                Transform::identity(),
            )
        }
    }
}

fn shape_path(shape: &Shape) -> Option<Path> {
    match shape {
        // Text is glyph pixels, not an outline: draw_text paints it.
        Shape::Text(_) => None,
        // A picture is its texels: draw_image paints it.
        Shape::Image { .. } => None,
        // Glass samples the frame: draw_glass paints it.
        Shape::Glass { .. } => None,
        // A chart is a family's own marks, drawn by the chart backend.
        Shape::Chart { .. } => None,
        Shape::Rect {
            width,
            height,
            corner_radius,
        } => Some(rounded_rect(*width, *height, *corner_radius)),
        Shape::Ellipse { width, height } => {
            PathBuilder::from_oval(Rect::from_xywh(0.0, 0.0, *width, *height)?)
        }
        Shape::Line { length } => {
            let mut builder = PathBuilder::new();
            builder.move_to(0.0, 0.0);
            builder.line_to(*length, 0.0);
            builder.finish()
        }
        Shape::Path { commands, closed } => {
            let mut builder = PathBuilder::new();
            for command in commands {
                match *command {
                    PathCommand::MoveTo(x, y) => builder.move_to(x, y),
                    PathCommand::LineTo(x, y) => builder.line_to(x, y),
                    PathCommand::QuadTo(cx, cy, x, y) => builder.quad_to(cx, cy, x, y),
                    PathCommand::CubicTo(c1x, c1y, c2x, c2y, x, y) => {
                        builder.cubic_to(c1x, c1y, c2x, c2y, x, y)
                    }
                    PathCommand::Close => builder.close(),
                };
            }
            if *closed {
                builder.close();
            }
            builder.finish()
        }
    }
}

/// Corners are quarter circles joined by lines, with the usual cubic
/// approximation of a quarter arc. A radius wider than half the box is the
/// widest corner the box can carry, so it is what gets drawn.
fn rounded_rect(width: f32, height: f32, radius: f32) -> Path {
    let radius = radius.clamp(0.0, width.min(height) / 2.0);
    if radius <= 0.0 {
        return PathBuilder::from_rect(
            Rect::from_xywh(0.0, 0.0, width, height).expect("a sized box"),
        );
    }
    let kappa = 0.552_284_8 * radius;
    let mut builder = PathBuilder::new();
    builder.move_to(radius, 0.0);
    builder.line_to(width - radius, 0.0);
    builder.cubic_to(
        width - radius + kappa,
        0.0,
        width,
        radius - kappa,
        width,
        radius,
    );
    builder.line_to(width, height - radius);
    builder.cubic_to(
        width,
        height - radius + kappa,
        width - radius + kappa,
        height,
        width - radius,
        height,
    );
    builder.line_to(radius, height);
    builder.cubic_to(
        radius - kappa,
        height,
        0.0,
        height - radius + kappa,
        0.0,
        height - radius,
    );
    builder.line_to(0.0, radius);
    builder.cubic_to(0.0, radius - kappa, radius - kappa, 0.0, radius, 0.0);
    builder.close();
    builder.finish().unwrap_or_else(|| {
        PathBuilder::from_rect(Rect::from_xywh(0.0, 0.0, width, height).expect("a sized box"))
    })
}

/// A display list fitted into a device's own pixel box, then turned.
///
/// Panels are not artboards: a 480×120 theme on a 464×1920 strip is contained
/// and centred, never cropped, and the strip's own quarter turn is applied
/// last. A round panel takes any integer angle and clips to its disc; a
/// rectangular one takes quarter turns only, because that is what its glass
/// shows without resampling the edges.
pub fn fit_into(
    list: &DisplayList,
    width: u32,
    height: u32,
    rotation: u16,
    round: bool,
    fonts: Option<&mut Fonts>,
    images: Option<&mut Images>,
) -> Result<Raster, String> {
    let source = render(list, 1.0, fonts, images);
    fit_raster_into(&source, width, height, rotation, round)
}

/// Fit an already-rendered frame into a device's own pixel box.
///
/// The sidecar paints charts through `render_with_charts`, which `render` here
/// cannot do: extracting the fit keeps that frame intact. The picture is
/// contained and centred, not cropped, with the device's turn applied last.
pub fn fit_raster_into(
    source: &Raster,
    width: u32,
    height: u32,
    rotation: u16,
    round: bool,
) -> Result<Raster, String> {
    if round {
        if rotation > 359 {
            return Err(format!(
                "A panel turn is 0 to 359 degrees; {rotation} is not one."
            ));
        }
    } else if !rotation.is_multiple_of(90) || rotation > 270 {
        return Err(format!(
            "The panel turns its picture by quarter turns only; {rotation}° is not one."
        ));
    }
    if source.width == 0 || source.height == 0 || width == 0 || height == 0 {
        return Ok(Raster {
            width,
            height,
            rgba: vec![0; (width as usize) * (height as usize) * 4],
        });
    }
    let pixels = Decoded {
        width: source.width,
        height: source.height,
        rgba: source.rgba.clone(),
    };
    let mut pixmap = match Pixmap::new(width, height) {
        Some(pixmap) => pixmap,
        None => {
            return Err(format!("A {width}×{height} frame could not be allocated."));
        }
    };
    pixmap.fill(SkiaColor::TRANSPARENT);

    let angle = f32::from(rotation) * std::f32::consts::PI / 180.0;
    let (sin, cos) = angle.sin_cos();
    let (source_width, source_height) = (pixels.width as f32, pixels.height as f32);
    // The box the turned picture needs, so the fit contains the turn rather
    // than cropping whatever the corners sweep into.
    let turned_width = source_width * cos.abs() + source_height * sin.abs();
    let turned_height = source_width * sin.abs() + source_height * cos.abs();
    let scale = (width as f32 / turned_width).min(height as f32 / turned_height);
    let (centre_x, centre_y) = (width as f32 / 2.0, height as f32 / 2.0);
    let (source_x, source_y) = (source_width / 2.0, source_height / 2.0);
    let radius = width.min(height) as f32 / 2.0;

    for y in 0..height {
        for x in 0..width {
            let (target_x, target_y) = (x as f32 + 0.5, y as f32 + 0.5);
            if round {
                let dx = target_x - centre_x;
                let dy = target_y - centre_y;
                if dx * dx + dy * dy > radius * radius {
                    continue;
                }
            }
            let dx = (target_x - centre_x) / scale;
            let dy = (target_y - centre_y) / scale;
            // Undo the turn, then the scale: the source point that lands on
            // this pixel, measured from the picture's own centre.
            let u = (dx * cos + dy * sin + source_x) / source_width;
            let v = (-dx * sin + dy * cos + source_y) / source_height;
            if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                continue;
            }
            let texel = sample(&pixels, u, v);
            blend(
                &mut pixmap,
                x as i32,
                y as i32,
                TextColor::rgba(texel[0], texel[1], texel[2], texel[3]),
            );
        }
    }
    Ok(Raster {
        width,
        height,
        rgba: straight_rgba(&pixmap),
    })
}

/// tiny-skia paints in premultiplied alpha; a golden file and a device both
/// want the colour the author wrote.
fn straight_rgba(pixmap: &Pixmap) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(pixmap.data().len());
    for pixel in pixmap.pixels() {
        let color = pixel.demultiply();
        rgba.extend_from_slice(&[color.red(), color.green(), color.blue(), color.alpha()]);
    }
    rgba
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use yunari_render::Color;

    fn list(json: &str) -> DisplayList {
        let document = yunari_model::document::parse_document(json).expect("fixture parses");
        yunari_render::build(&document, &mut Fonts::new()).0
    }

    /// The Fabric scene a real v3 theme carries, planned through the same path
    /// the sidecar uses, so anchors, scale and glass ops are the authored ones.
    fn fabric_list(json: &str) -> DisplayList {
        let document = yunari_model::document::parse_document(json).expect("fixture parses");
        yunari_render::fabric::build(
            &document,
            &mut Fonts::new(),
            &yunari_render::Readings::default(),
            &yunari_providers::units::UnitPreferences::default(),
        )
        .0
    }

    /// A Fabric list planned with the decoded size of its background picture.
    /// The area and ramps can be planned without bytes; placing the picture
    /// needs its own dimensions.
    fn backdrop_list(json: &str) -> DisplayList {
        let document = yunari_model::document::parse_document(json).expect("fixture parses");
        let sources = HashMap::from([("assets/pic.png".to_owned(), (2.0, 2.0))]);
        yunari_render::fabric::build_with_sources(
            &document,
            &mut Fonts::new(),
            &yunari_render::Readings::default(),
            &yunari_providers::units::UnitPreferences::default(),
            &sources,
        )
        .0
    }

    /// A list built without measured sources — the path every non-sidecar
    /// backend uses — still has to paint a picture the raster can decode.
    #[test]
    fn a_backdrop_without_a_planned_source_still_composites_the_decoded_picture() {
        let dir = image_fixture("unplanned-backdrop", [0, 0, 255]);
        let document = yunari_model::document::parse_document(
            r##"{"schemaVersion":"3.0.0","id":"t","artboard":{"width":4,"height":4},
                "background":{"mode":"advanced",
                "media":{"assetId":"pic","bounds":"theme",
                "color":{"value":"#ff0000"},
                "composition":{"anchor":"top","extent":0.5}}},
                "assets":[{"id":"pic","kind":"image","path":"assets/pic.png"}],
                "scene":{"version":"7.4.0","objects":[]}}"##,
        )
        .expect("fixture parses");
        let list = yunari_render::fabric::build(
            &document,
            &mut Fonts::new(),
            &yunari_render::Readings::default(),
            &yunari_providers::units::UnitPreferences::default(),
        )
        .0;
        let raster = render(&list, 1.0, None, Some(&mut Images::at(&dir)));
        assert_eq!(pixel(&raster, 2, 1), [0, 0, 255, 255]);
        assert_eq!(pixel(&raster, 2, 3), [255, 0, 0, 255]);
        let _ = std::fs::remove_dir_all(dir);
    }

    fn pixel(raster: &Raster, x: u32, y: u32) -> [u8; 4] {
        let index = ((y * raster.width + x) * 4) as usize;
        [
            raster.rgba[index],
            raster.rgba[index + 1],
            raster.rgba[index + 2],
            raster.rgba[index + 3],
        ]
    }

    #[test]
    fn a_landscape_theme_is_contained_in_a_portrait_panel() {
        let raster = fit_into(
            &list(
                r##"{"schemaVersion":"3.0.0","id":"t","artboard":{"width":480,"height":120},
                    "nodes":[{"id":"n","kind":{"type":"rectangle"},
                        "transform":{"x":0,"y":0,"width":240,"height":120},
                        "style":{"fill":{"value":"#ff0000"}}}]}"##,
            ),
            120,
            320,
            0,
            false,
            None,
            None,
        )
        .expect("a contained frame");
        assert_eq!((raster.width, raster.height), (120, 320));
        // The strip is 120×30 inside the tall box: margins above and below, no
        // cropping of the picture's own edges.
        assert_eq!(pixel(&raster, 10, 10)[3], 0);
        assert_eq!(pixel(&raster, 10, 160)[3], 255);
        assert_eq!(pixel(&raster, 115, 310)[3], 0);
        // The source's left half is the red one, so the left of the panel is.
        assert_eq!(pixel(&raster, 10, 160), [255, 0, 0, 255]);
        assert_eq!(pixel(&raster, 110, 160)[3], 0);
    }

    #[test]
    fn a_quarter_turn_moves_the_picture_clockwise() {
        let drawn = list(
            r##"{"schemaVersion":"3.0.0","id":"t","artboard":{"width":10,"height":10},
                "nodes":[{"id":"n","kind":{"type":"rectangle"},
                    "transform":{"x":0,"y":0,"width":5,"height":10},
                    "style":{"fill":{"value":"#00ff00"}}}]}"##,
        );
        let plain = fit_into(&drawn, 100, 100, 0, false, None, None).unwrap();
        // Left half painted before the turn.
        assert_eq!(pixel(&plain, 10, 50), [0, 255, 0, 255]);
        assert_eq!(pixel(&plain, 90, 50)[3], 0);
        let turned = fit_into(&drawn, 100, 100, 90, false, None, None).unwrap();
        // Clockwise: what was on the left is now on the top.
        assert_eq!(pixel(&turned, 50, 10), [0, 255, 0, 255]);
        assert_eq!(pixel(&turned, 50, 90)[3], 0);
    }

    #[test]
    fn a_square_panel_takes_any_angle_and_clips_its_disc() {
        let drawn = list(
            r##"{"schemaVersion":"3.0.0","id":"t","artboard":{"width":10,"height":10},
                "nodes":[{"id":"n","kind":{"type":"rectangle"},
                    "transform":{"x":0,"y":0,"width":10,"height":10},
                    "style":{"fill":{"value":"#00ff00"}}}]}"##,
        );
        let refused = fit_into(&drawn, 100, 100, 45, false, None, None).unwrap_err();
        assert_eq!(
            refused,
            "The panel turns its picture by quarter turns only; 45° is not one."
        );
        let disc = fit_into(&drawn, 100, 100, 45, true, None, None).expect("a disc");
        // The corner of the box is outside the round glass.
        assert_eq!(pixel(&disc, 1, 1)[3], 0);
        assert_eq!(pixel(&disc, 50, 50)[3], 255);
    }

    #[test]
    fn a_filled_rectangle_covers_its_box_only() {
        let raster = render(
            &list(
                r##"{"schemaVersion":"3.0.0","id":"t","artboard":{"width":10,"height":10},
                    "nodes":[{"id":"n","kind":{"type":"rectangle"},
                    "transform":{"x":2,"y":2,"width":6,"height":6},
                    "style":{"fill":{"value":"#ff0000"}}}]}"##,
            ),
            1.0,
            Some(&mut Fonts::new()),
            None,
        );
        assert_eq!(raster.width, 10);
        assert_eq!(pixel(&raster, 5, 5), [255, 0, 0, 255]);
        assert_eq!(pixel(&raster, 0, 0), [0, 0, 0, 0]);
    }

    #[test]
    fn an_ellipse_leaves_its_corner_unpainted() {
        let raster = render(
            &list(
                r##"{"schemaVersion":"3.0.0","id":"t","artboard":{"width":10,"height":10},
                    "nodes":[{"id":"n","kind":{"type":"ellipse"},
                    "transform":{"x":0,"y":0,"width":10,"height":10},
                    "style":{"fill":{"value":"#00ff00"}}}]}"##,
            ),
            1.0,
            Some(&mut Fonts::new()),
            None,
        );
        assert_eq!(pixel(&raster, 5, 5), [0, 255, 0, 255]);
        assert_eq!(pixel(&raster, 0, 0), [0, 0, 0, 0]);
    }

    #[test]
    fn a_background_fills_the_board_behind_the_nodes() {
        let raster = render(
            &list(
                r##"{"schemaVersion":"3.0.0","id":"t","artboard":{"width":4,"height":4},
                    "background":{"color":{"value":"#0000ff"}},"nodes":[]}"##,
            ),
            1.0,
            Some(&mut Fonts::new()),
            None,
        );
        assert_eq!(pixel(&raster, 0, 0), [0, 0, 255, 255]);
    }

    #[test]
    fn a_gradient_runs_along_its_axis() {
        let raster = render(
            &list(
                r##"{"schemaVersion":"3.0.0","id":"t","artboard":{"width":8,"height":8},
                    "palette":{"p":{"name":"P","value":{"kind":"gradient","angle":0,
                    "stops":[{"offset":0,"color":"#000000"},{"offset":1,"color":"#ffffff"}]}}},
                    "nodes":[{"id":"n","kind":{"type":"rectangle"},
                    "transform":{"width":8,"height":8},"style":{"fill":{"ref":"palette.p"}}}]}"##,
            ),
            1.0,
            Some(&mut Fonts::new()),
            None,
        );
        let left = pixel(&raster, 1, 4);
        let right = pixel(&raster, 6, 4);
        assert!(
            left[0] < right[0],
            "the gradient runs left to right: {left:?} then {right:?}"
        );
    }

    #[test]
    fn scale_doubles_the_frame() {
        let raster = render(
            &list(
                r##"{"schemaVersion":"3.0.0","id":"t","artboard":{"width":5,"height":5},
                    "nodes":[{"id":"n","kind":{"type":"rectangle"},
                    "transform":{"x":0,"y":0,"width":2,"height":2},
                    "style":{"fill":{"value":"#ffffff"}}}]}"##,
            ),
            2.0,
            Some(&mut Fonts::new()),
            None,
        );
        assert_eq!((raster.width, raster.height), (10, 10));
        assert_eq!(pixel(&raster, 3, 3), [255, 255, 255, 255]);
        assert_eq!(pixel(&raster, 5, 5), [0, 0, 0, 0]);
    }

    #[test]
    fn a_transparent_paint_leaves_nothing() {
        let raster = render(
            &list(
                r##"{"schemaVersion":"3.0.0","id":"t","artboard":{"width":4,"height":4},
                    "nodes":[{"id":"n","kind":{"type":"rectangle"},
                    "transform":{"width":4,"height":4},
                    "style":{"fill":{"value":"rgba(255, 0, 0, 0)"}}}]}"##,
            ),
            1.0,
            Some(&mut Fonts::new()),
            None,
        );
        assert_eq!(pixel(&raster, 2, 2), [0, 0, 0, 0]);
    }

    #[test]
    fn straight_alpha_is_the_authored_colour() {
        let raster = render(
            &list(
                r##"{"schemaVersion":"3.0.0","id":"t","artboard":{"width":2,"height":2},
                    "nodes":[{"id":"n","kind":{"type":"rectangle"},
                    "transform":{"width":2,"height":2},
                    "style":{"fill":{"value":"#ffffff"},"opacity":{"value":0.5}}}]}"##,
            ),
            1.0,
            Some(&mut Fonts::new()),
            None,
        );
        assert_eq!(pixel(&raster, 1, 1), [255, 255, 255, 128]);
    }

    #[test]
    fn a_text_node_paints_glyph_pixels_inside_its_box() {
        let raster = render(
            &list(
                r##"{"schemaVersion":"3.0.0","id":"t","artboard":{"width":120,"height":60},
                    "typeTokens":{"body":{"family":"Inter"}},
                    "nodes":[{"id":"n","transform":{"x":0,"y":0,"width":120,"height":60},
                    "kind":{"type":"text","content":{"runs":[
                        {"kind":"literal","text":"HELLO","typeToken":"typeTokens.body",
                         "style":{"fontSize":{"value":40}}}]}}}]}"##,
            ),
            1.0,
            Some(&mut Fonts::new()),
            None,
        );
        let painted = (0..raster.height)
            .flat_map(|y| (0..raster.width).map(move |x| (x, y)))
            .filter(|(x, y)| pixel(&raster, *x, *y)[3] > 0)
            .count();
        assert!(painted > 100, "glyphs painted {painted} pixels");
    }

    #[test]
    fn color_type_is_the_render_crates() {
        assert_eq!(Color::rgb(1, 2, 3).channels(), [1, 2, 3, 255]);
    }

    /// A directory that owns `assets/pic.png`, a solid `color` square.
    fn image_fixture(tag: &str, color: [u8; 3]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("yunari-image-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("assets")).expect("a place for the picture");
        let mut picture = image::RgbaImage::new(2, 2);
        for pixel in picture.pixels_mut() {
            *pixel = image::Rgba([color[0], color[1], color[2], 255]);
        }
        picture
            .save(dir.join("assets/pic.png"))
            .expect("the picture is written");
        dir
    }

    #[test]
    fn an_image_node_paints_its_picture_inside_its_box() {
        let dir = image_fixture("stretch", [255, 0, 0]);
        let raster = render(
            &list(
                r##"{"schemaVersion":"3.0.0","id":"t","artboard":{"width":8,"height":8},
                    "assets":[{"id":"pic","kind":"image","path":"assets/pic.png"}],
                    "nodes":[{"id":"n","kind":{"type":"image",
                    "content":{"assetId":"pic","fit":"stretch"}},
                    "transform":{"x":2,"y":2,"width":4,"height":4}}]}"##,
            ),
            1.0,
            None,
            Some(&mut Images::at(&dir)),
        );
        assert_eq!(pixel(&raster, 3, 3), [255, 0, 0, 255]);
        assert_eq!(pixel(&raster, 0, 0), [0, 0, 0, 0]);
        assert_eq!(pixel(&raster, 6, 6), [0, 0, 0, 0]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn contain_keeps_the_pictures_shape_and_leaves_the_margin_empty() {
        let dir = image_fixture("contain", [0, 0, 255]);
        let raster = render(
            &list(
                r##"{"schemaVersion":"3.0.0","id":"t","artboard":{"width":8,"height":4},
                    "assets":[{"id":"pic","kind":"image","path":"assets/pic.png"}],
                    "nodes":[{"id":"n","kind":{"type":"image",
                    "content":{"assetId":"pic","fit":"contain"}},
                    "transform":{"x":0,"y":0,"width":8,"height":4}}]}"##,
            ),
            1.0,
            None,
            Some(&mut Images::at(&dir)),
        );
        // A square picture in a wide box: centred, four units wide.
        assert_eq!(pixel(&raster, 1, 1), [0, 0, 0, 0]);
        assert_eq!(pixel(&raster, 4, 2), [0, 0, 255, 255]);
        assert_eq!(pixel(&raster, 6, 2), [0, 0, 0, 0]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_monochrome_picture_draws_its_alpha_in_the_paint_colour() {
        let dir = image_fixture("mask", [0, 0, 0]);
        let raster = render(
            &list(
                r##"{"schemaVersion":"3.0.0","id":"t","artboard":{"width":4,"height":4},
                    "palette":{"ink":{"name":"Ink","value":{"kind":"solid","color":"#ff0000"}}},
                    "assets":[{"id":"pic","kind":"image","path":"assets/pic.png"}],
                    "nodes":[{"id":"n","kind":{"type":"image",
                    "content":{"assetId":"pic","fit":"stretch","monochrome":{"ref":"palette.ink"}}},
                    "transform":{"x":0,"y":0,"width":4,"height":4}}]}"##,
            ),
            1.0,
            None,
            Some(&mut Images::at(&dir)),
        );
        assert_eq!(pixel(&raster, 2, 2), [255, 0, 0, 255]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_backdrop_covers_the_page_behind_its_nodes() {
        let dir = image_fixture("backdrop", [0, 0, 255]);
        let raster = render(
            &backdrop_list(
                r##"{"schemaVersion":"3.0.0","id":"t","artboard":{"width":4,"height":4},
                    "background":{"media":{"assetId":"pic","bounds":"theme",
                    "color":{"value":"#000000"}}},
                    "assets":[{"id":"pic","kind":"image","path":"assets/pic.png"}],
                    "nodes":[]}"##,
            ),
            1.0,
            None,
            Some(&mut Images::at(&dir)),
        );
        assert_eq!(pixel(&raster, 2, 2), [0, 0, 255, 255]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_advanced_top_band_leaves_the_fill_below_it() {
        let dir = image_fixture("top-band", [0, 0, 255]);
        let raster = render(
            &backdrop_list(
                r##"{"schemaVersion":"3.0.0","id":"t","artboard":{"width":4,"height":4},
                    "background":{"mode":"advanced",
                    "media":{"assetId":"pic","bounds":"theme",
                    "color":{"value":"#ff0000"},
                    "composition":{"anchor":"top","extent":0.5}}},
                    "assets":[{"id":"pic","kind":"image","path":"assets/pic.png"}],
                    "nodes":[]}"##,
            ),
            1.0,
            None,
            Some(&mut Images::at(&dir)),
        );
        assert_eq!(pixel(&raster, 2, 1), [0, 0, 255, 255]);
        assert_eq!(pixel(&raster, 2, 3), [255, 0, 0, 255]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_glass_panel_frosts_what_is_painted_beneath_it() {
        let raster = render(
            &list(
                r##"{"schemaVersion":"3.0.0","id":"t","artboard":{"width":40,"height":40},
                    "background":{"color":{"value":"#000000"}},
                    "nodes":[
                    {"id":"behind","kind":{"type":"rectangle"},
                    "transform":{"x":0,"y":0,"width":20,"height":40},
                    "style":{"fill":{"value":"#ff0000"}}},
                    {"id":"panel","kind":{"type":"rectangle"},
                    "transform":{"x":10,"y":10,"width":20,"height":20},
                    "glass":{"blurRadius":12}}]}"##,
            ),
            1.0,
            None,
            None,
        );
        // The red edge is softened into the panel, so a pixel past it is no
        // longer either the red or the background green-free black.
        let beyond = pixel(&raster, 26, 20);
        assert!(beyond[0] > 0 && beyond[0] < 255, "{beyond:?}");
        // Outside the panel nothing moved.
        assert_eq!(pixel(&raster, 35, 20), [0, 0, 0, 255]);
    }

    /// Glass samples a surround, not just the panel's own box. With a sharp
    /// black/white edge just outside a panel, the pixel nearest that edge must
    /// take on the neighbour's colour. Sampling only the panel box clamps the
    /// blur at its boundary and leaves the edge unsoftened.
    #[test]
    fn glass_diffuses_colour_from_outside_its_own_box() {
        let raster = render(
            &list(
                r##"{"schemaVersion":"3.0.0","id":"t","artboard":{"width":40,"height":20},
                    "background":{"color":{"value":"#000000"}},
                    "nodes":[
                    {"id":"source","kind":{"type":"rectangle"},
                    "transform":{"x":20,"y":0,"width":20,"height":20},
                    "style":{"fill":{"value":"#ffffff"}}},
                    {"id":"panel","kind":{"type":"rectangle"},
                    "transform":{"x":8,"y":0,"width":12,"height":20},
                    "glass":{"blurRadius":8}}]}"##,
            ),
            1.0,
            None,
            None,
        );
        // White touches the panel's right edge. A blur must carry it inward;
        // clamping the sample at the panel box leaves the edge black.
        let inside = pixel(&raster, 19, 10);
        let centre = pixel(&raster, 10, 10);
        assert!(
            inside[0] >= centre[0] + 3,
            "glass must diffuse the neighbouring white inward: {inside:?} vs {centre:?}"
        );
    }

    /// The source scales a glass blur by the equal-area root of the object's
    /// matrix (`glass.ts:474-476`). The Lian Li demo's cards are scaled 1.1421
    /// by 0.6969, so reading one axis alone makes them 64% too blurry.
    #[test]
    fn glass_blur_follows_the_equal_area_scale_not_one_axis() {
        let wide = device_blur_radius(8.0, Affine::scale(2.0, 0.5), 1.0);
        let tall = device_blur_radius(8.0, Affine::scale(0.5, 2.0), 1.0);
        assert!(
            (wide - tall).abs() < 0.01,
            "equal-area transforms must blur alike: wide {wide}, tall {tall}"
        );
        assert!((wide - 8.0).abs() < 0.01, "area scale 1 keeps the radius");
    }

    #[test]
    fn glass_over_a_backdrop_samples_the_painted_picture() {
        let dir = image_fixture("glass-backdrop", [0, 0, 255]);
        let raster = render(
            &backdrop_list(
                r##"{"schemaVersion":"3.0.0","id":"t","artboard":{"width":20,"height":20},
                    "background":{"mode":"advanced",
                    "media":{"assetId":"pic","bounds":"theme",
                    "color":{"value":"#ff0000"},
                    "composition":{"anchor":"top","extent":0.75,"fade":0.1}}},
                    "assets":[{"id":"pic","kind":"image","path":"assets/pic.png"}],
                    "nodes":[
                    {"id":"panel","kind":{"type":"rectangle"},
                     "transform":{"x":0,"y":0,"width":20,"height":6},
                     "style":{"fill":{"value":"rgba(0, 0, 0, 0.2)"}},
                     "glass":{"blurRadius":4}}]}"##,
            ),
            1.0,
            None,
            Some(&mut Images::at(&dir)),
        );
        let over = pixel(&raster, 10, 4);
        let bare = pixel(&raster, 10, 8);
        assert_eq!(bare, [0, 0, 255, 255], "backdrop paints the band");
        assert!(
            over[2] > 180,
            "glass still shows the picture beneath it: {over:?}"
        );
        assert!(
            over[0] < 80,
            "glass does not replace the picture with the fill: {over:?}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A scaled glass rect must sample the picture at each point under it, not
    /// one row stretched across its box. The image's left half is red and its
    /// right half blue, so the two ends of the glass must differ.
    #[test]
    fn a_scaled_glass_rect_samples_the_picture_beneath_every_point() {
        let dir = std::env::temp_dir().join(format!("yunari-scaled-glass-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("assets")).expect("a theme dir with assets");
        let mut picture = image::RgbaImage::new(4, 2);
        for y in 0..2 {
            for x in 0..4 {
                let color = if x < 2 {
                    [255, 0, 0, 255]
                } else {
                    [0, 0, 255, 255]
                };
                picture.put_pixel(x, y, image::Rgba(color));
            }
        }
        picture
            .save(dir.join("assets/pic.png"))
            .expect("the picture is written");

        let list = backdrop_list(
            r##"{"schemaVersion":"3.0.0","id":"t","artboard":{"width":40,"height":40},
                "background":{"mode":"advanced",
                "media":{"assetId":"pic","bounds":"theme",
                "color":{"value":"#111111"},
                "composition":{"anchor":"top","extent":1.0,"fade":0.01}}},
                "assets":[{"id":"pic","kind":"image","path":"assets/pic.png"}],
                "scene":{"version":"7.4.0","objects":[
                  {"id":"panel","type":"Rect","left":0,"top":0,"width":40,"height":40,
                   "originX":"left","originY":"top","scaleY":0.5,
                   "fill":"#00000033","yunariGlass":{"blurRadius":2}}]}}"##,
        );
        let raster = render(&list, 1.0, None, Some(&mut Images::at(&dir)));
        let left = pixel(&raster, 10, 4);
        let right = pixel(&raster, 30, 4);
        assert!(
            left[0] > left[2] + 40,
            "the glass left half shows red: {left:?}"
        );
        assert!(
            right[2] > right[0] + 40,
            "the glass right half shows blue: {right:?}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The pattern places the extracted region in device space, but `fill_path`
    /// post-concats the fill transform onto the shader. A panel that is itself
    /// translated or scaled must pre-divide that transform out, or the region
    /// lands at `op_transform ∘ sample_corner` instead of `sample_corner`.
    /// R10-A's glass sits at (-20, 198) with `scaleY: 0.6097`; Aurora's cards
    /// sit at (24, 196) and (248, 196). Both read a flat area with the bug.
    #[test]
    fn glass_samples_the_picture_directly_beneath_a_translated_panel() {
        let raster = render(
            &list(
                r##"{"schemaVersion":"3.0.0","id":"t","artboard":{"width":60,"height":40},
                    "background":{"color":{"value":"#000000"}},
                    "nodes":[
                    {"id":"red","kind":{"type":"rectangle"},
                     "transform":{"x":0,"y":0,"width":30,"height":40},
                     "style":{"fill":{"value":"#ff0000"}}},
                    {"id":"blue","kind":{"type":"rectangle"},
                     "transform":{"x":30,"y":0,"width":30,"height":40},
                     "style":{"fill":{"value":"#0000ff"}}},
                    {"id":"panel","kind":{"type":"rectangle"},
                     "transform":{"x":20,"y":10,"width":20,"height":20},
                     "style":{"fill":{"value":"rgba(0,0,0,0)"}},
                     "glass":{"blurRadius":1}}]}"##,
            ),
            1.0,
            None,
            None,
        );
        // The panel straddles the red/blue edge. Its right half must still show
        // blue: a transform applied twice samples far to the left of the panel.
        let right = pixel(&raster, 35, 20);
        let left = pixel(&raster, 25, 20);
        assert!(
            i16::from(right[2]) > i16::from(right[0]) + 60,
            "glass right half must show the blue beneath it: {right:?}"
        );
        assert!(
            i16::from(left[0]) > i16::from(left[2]) + 60,
            "glass left half must show the red beneath it: {left:?}"
        );
    }

    /// The R10-A Clean Clock card: a wide, short glass rect with a translated
    /// leading corner and a non-unit `scaleY`. The blur must land on the card,
    /// not on a shifted, rescaled copy of the sampled region.
    #[test]
    fn glass_on_the_r10_clock_card_keeps_its_sampled_region_in_place() {
        let raster = render(
            &fabric_list(
                r##"{"schemaVersion":"3.0.0","id":"t","artboard":{"width":320,"height":320},
                    "background":{"color":{"value":"#000000"}},
                    "scene":{"version":"7.4.0","objects":[
                    {"id":"red","type":"Rect","left":0,"top":0,"width":160,"height":320,
                     "originX":"left","originY":"top","fill":"#ff0000"},
                    {"id":"blue","type":"Rect","left":160,"top":0,"width":160,"height":320,
                     "originX":"left","originY":"top","fill":"#0000ff"},
                    {"id":"card","type":"Rect","left":-20,"top":198.056,"width":360,"height":200,
                     "scaleY":0.6097,"originX":"left","originY":"top",
                     "fill":"rgba(0,0,0,0)","yunariGlass":{"blurRadius":2}}]}}"##,
            ),
            1.0,
            None,
            None,
        );
        // The card spans x -20..340, straddling the source edge at 160. Both
        // halves must show their own colour through the glass.
        let left = pixel(&raster, 80, 250);
        let right = pixel(&raster, 240, 250);
        assert!(
            i16::from(left[0]) > i16::from(left[2]) + 60,
            "R10 card's left half must show red: {left:?}"
        );
        assert!(
            i16::from(right[2]) > i16::from(right[0]) + 60,
            "R10 card's right half must show blue: {right:?}"
        );
    }

    /// A big value and a small `%` unit share one line. The shaper lays both
    /// out in one buffer, so both inherit the paragraph's line height. The
    /// raster backend reshapes each segment on its own; with a hard-coded 1.2
    /// instead of the authored 1.18 the two glyph planes land at different
    /// baselines and the unit reads as sitting above the value.
    #[test]
    fn text_runs_of_different_sizes_share_a_baseline() {
        let document = yunari_model::document::parse_document(
            r##"{"schemaVersion":"3.0.0","id":"t","artboard":{"width":200,"height":100},
                "background":{"color":{"value":"#000000"}},
                "typeTokens":{"heading":{"family":"Inter","weight":700}},
                "palette":{"ink":{"name":"Ink","value":{"kind":"solid","color":"#ffffff"}},
                           "dim":{"name":"Dim","value":{"kind":"solid","color":"#ff0000"}}},
                "scene":{"version":"7.4.0","objects":[
                  {"id":"label","type":"Textbox","left":10,"top":10,"width":160,"height":70,
                   "originX":"left","originY":"top","fill":"#ffffff",
                   "yunariText":{"box":{"width":160,"height":70},
                     "align":"left","verticalAlign":"middle","overflow":"clip",
                     "runs":[
                       {"kind":"literal","text":"42","typeToken":"typeTokens.heading",
                        "style":{"color":{"ref":"palette.ink"},"fontSize":{"value":60},
                                 "lineHeight":{"value":1.18}}},
                       {"kind":"literal","text":"%","typeToken":"typeTokens.heading",
                        "style":{"color":{"ref":"palette.dim"},"fontSize":{"value":15},
                                 "lineHeight":{"value":1.18}}}]}}]}}"##,
        )
        .expect("fixture parses");
        let (planned, issues) = yunari_render::fabric::build(
            &document,
            &mut Fonts::new(),
            &yunari_render::Readings::default(),
            &yunari_providers::units::UnitPreferences::default(),
        );
        assert!(issues.is_empty(), "{issues:?}");
        let mut fonts = Fonts::new();
        let raster = render(&planned, 1.0, Some(&mut fonts), None);
        // Find the bottom of each run's ink by colour. Both runs share the
        // same shaped line, so their ink must end on the same baseline row
        // (allowing for descenders, which neither glyph here has).
        let mut value_bottom = None;
        let mut unit_bottom = None;
        for y in 0..raster.height {
            for x in 0..raster.width {
                let p = pixel(&raster, x, y);
                if p[0] > 200 && p[1] > 200 && p[2] > 200 && p[3] > 100 {
                    value_bottom = Some(y);
                }
                if p[0] > 200 && p[1] < 100 && p[2] < 100 && p[3] > 100 {
                    unit_bottom = Some(y);
                }
            }
        }
        let value = value_bottom.expect("value glyphs painted");
        let unit = unit_bottom.expect("unit glyphs painted");
        assert!(
            value.abs_diff(unit) <= 4,
            "unit and value must share a baseline: value bottom {value}, unit bottom {unit}"
        );
    }

    /// The Lian Li and Turzx cards were reported as "tint only": a flat
    /// colour over the card with no visible diffusion. Reading two frames of
    /// the same scene, one with a blur radius and one without, must differ in
    /// the pixels immediately inside the card — not merely in its fill.
    #[test]
    fn glass_blur_changes_pixels_and_not_only_the_tint() {
        let scene = |radius: u32| {
            let json = format!(
                r##"{{"schemaVersion":"3.0.0","id":"t","artboard":{{"width":80,"height":40}},
                    "background":{{"color":{{"value":"#000000"}}}},
                    "scene":{{"version":"7.4.0","objects":[
                    {{"id":"stripes","type":"Rect","left":0,"top":0,"width":40,"height":40,
                      "originX":"left","originY":"top","fill":"#ffffff"}},
                    {{"id":"stripes2","type":"Rect","left":40,"top":0,"width":40,"height":40,
                      "originX":"left","originY":"top","fill":"#000000"}},
                    {{"id":"card","type":"Rect","left":24,"top":8,"width":32,"height":24,
                      "originX":"left","originY":"top","fill":"rgba(0,0,0,0)",
                      "yunariGlass":{{"blurRadius":{radius}}}}}]}}}}"##
            );
            render(&fabric_list(&json), 1.0, None, None)
        };
        let sharp = scene(0);
        let blurred = scene(8);
        let mut differing = 0;
        for y in 9..32 {
            for x in 25..56 {
                let a = pixel(&sharp, x, y);
                let b = pixel(&blurred, x, y);
                if a != b {
                    differing += 1;
                }
            }
        }
        // The card straddles the white/black edge at x=40. Without a blur the
        // edge inside the card stays sharp; with one it diffuses. A tint-only
        // implementation would paint the same pixels in both frames.
        assert!(
            differing > 50,
            "glass blur must diffuse the edge; only {differing} pixels changed"
        );
        let sharp_edge = i16::from(pixel(&sharp, 39, 20)[0]);
        let soft_edge = i16::from(pixel(&blurred, 39, 20)[0]);
        assert!(
            soft_edge < sharp_edge,
            "the sharp side of the edge must be softened: {sharp_edge} -> {soft_edge}"
        );
    }

    #[test]
    fn a_picture_with_no_source_draws_nothing() {
        let raster = render(
            &list(
                r##"{"schemaVersion":"3.0.0","id":"t","artboard":{"width":4,"height":4},
                    "assets":[{"id":"pic","kind":"image","path":"assets/pic.png"}],
                    "nodes":[{"id":"n","kind":{"type":"image",
                    "content":{"assetId":"pic","fit":"stretch"}},
                    "transform":{"x":0,"y":0,"width":4,"height":4}}]}"##,
            ),
            1.0,
            None,
            Some(&mut Images::empty()),
        );
        assert_eq!(pixel(&raster, 2, 2), [0, 0, 0, 0]);
    }
}
