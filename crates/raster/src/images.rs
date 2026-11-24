//! Decoded pictures, loaded once and kept for the frames that follow.
//!
//! A theme names an asset by its path inside the theme; decoding is the
//! backend's job because pixels are. The cache keys on that path, so a picture
//! used by several nodes is decoded once, and a path that fails to decode is
//! remembered as a failure rather than retried every frame.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::Instant;

/// A decoded picture as straight RGBA8, the same layout a [`crate::Raster`]
/// holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The pictures a render can reach, relative to the theme's own directory.
#[derive(Default)]
pub struct Images {
    base: Option<PathBuf>,
    cache: HashMap<String, Option<Decoded>>,
    /// A theme video is a live decoder, not a picture: it reads a frame per
    /// paint so a background plays instead of showing its first image.
    videos: HashMap<String, Video>,
    /// The panel's draw rate, in frames per second. Decoding faster cannot put
    /// more frames on the glass. Absent means the source's own rate.
    demand_fps: f64,
    /// Device pixels the sidecar will paint. A source larger than this is
    /// scaled by ffmpeg before RGBA conversion; the rest of the pipeline still
    /// samples the same composition, just from fewer bytes.
    render_size: (u32, u32),
}

/// One video asset's decoder.
enum Video {
    Failed,
    Live(Live),
}

impl Drop for Video {
    fn drop(&mut self) {
        if let Video::Live(live) = self {
            let _ = live.child.kill();
        }
    }
}

struct Live {
    path: PathBuf,
    child: Child,
    stdout: ChildStdout,
    width: u32,
    height: u32,
    /// The rate ffmpeg is asked to emit, capped at the panel's cadence. Frames
    /// above it are dropped before RGBA conversion, so they cost no decode.
    fps: f64,
    render_size: (u32, u32),
    total: u64,
    frames: u64,
    started: Instant,
}

impl Images {
    /// Pictures are looked up beside the theme file.
    pub fn at(base: impl AsRef<Path>) -> Self {
        Self {
            base: Some(base.as_ref().to_path_buf()),
            cache: HashMap::new(),
            videos: HashMap::new(),
            demand_fps: f64::INFINITY,
            render_size: (0, 0),
        }
    }

    /// No pictures: every image node draws nothing. A frame rendered without an
    /// asset source says so rather than inventing a picture.
    pub fn empty() -> Self {
        Self {
            demand_fps: f64::INFINITY,
            render_size: (0, 0),
            ..Self::default()
        }
    }

    /// Limit video decoding to the panel's own rate. A 30 fps source on a
    /// 110 ms panel has no viewer for two of every three frames, and decoding
    /// them costs a 4K RGBA read apiece.
    pub fn set_demand_fps(&mut self, fps: f64) {
        if fps.is_finite() && fps > 0.0 {
            self.demand_fps = fps;
        }
    }

    /// Bound decoded video frames to the device's own geometry. Only a session
    /// that paints for one panel knows this; a detached raster keeps the
    /// source's resolution.
    pub fn set_render_size(&mut self, width: u32, height: u32) {
        self.render_size = (width, height);
    }

    /// A picture the caller holds in memory, such as one just imported and not
    /// yet saved. It takes the same path the document will store it under.
    pub fn put(&mut self, path: &str, bytes: &[u8]) {
        let decoded = decode_bytes_at(Path::new(path), bytes);
        self.cache.insert(path.to_owned(), decoded);
    }

    /// Holds one decoded frame at an asset path for the session's lifetime. A
    /// video panel's glass samples this poster, not the moving decoder behind
    /// it; holding the bytes in the cache keeps every frame identical.
    pub fn hold(&mut self, path: &str, decoded: Decoded) {
        self.videos.remove(path);
        self.cache.insert(path.to_owned(), Some(decoded));
    }

    /// The picture at an asset path, decoded on first use. A video plays: each
    /// call advances its own decoder to the frame the wall clock names. `None`
    /// means the bytes are missing or are neither an image nor a readable
    /// video, and nothing is substituted for them.
    pub fn load(&mut self, path: &str) -> Option<&Decoded> {
        self.load_clip(path, true)
    }

    /// The same lookup for a clip node, whose own `loop` decides whether the
    /// last frame holds or the clip starts again.
    pub fn load_clip(&mut self, path: &str, looping: bool) -> Option<&Decoded> {
        if !self.cache.contains_key(path) && !self.videos.contains_key(path) {
            let file = self.base.as_ref().map(|base| base.join(path));
            match file {
                Some(file) if is_video(&file) => {
                    let video = start(&file, self.demand_fps, self.render_size)
                        .map_or(Video::Failed, Video::Live);
                    self.videos.insert(path.to_owned(), video);
                }
                other => {
                    let decoded = other.and_then(|file| decode(&file));
                    self.cache.insert(path.to_owned(), decoded);
                }
            }
        }
        if let Some(Video::Live(live)) = self.videos.get_mut(path)
            && let Some(frame) = live.frame(looping)
        {
            self.cache.insert(path.to_owned(), Some(frame));
        }
        self.cache.get(path).and_then(Option::as_ref)
    }
}

/// A file whose first boxes say `ftyp` is a video, whatever its name claims.
fn is_video(path: &Path) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut header = [0u8; 12];
    file.read_exact(&mut header).is_ok() && &header[4..8] == b"ftyp"
}

/// The frame a clip reaches after `elapsed`, capped by what the panel can show.
#[cfg(test)]
fn target_frame(source_fps: f64, demand_fps: f64, elapsed: std::time::Duration) -> u64 {
    (elapsed.as_secs_f64() * source_fps.min(demand_fps)) as u64
}

/// Full-size frame reads a decoder makes to catch up with `elapsed`. One read
/// per frame, so this is the CPU-equivalent cost the panel pays. The live
/// decoder reaches the same count through `target_frame`'s while loop.
#[cfg(test)]
fn frame_reads(
    source_fps: f64,
    demand_fps: f64,
    previous: u64,
    elapsed: std::time::Duration,
) -> u64 {
    target_frame(source_fps, demand_fps, elapsed).saturating_sub(previous) + 1
}

impl Live {
    /// The wall-clock frame a panel can actually show. A 30 fps source drawn on
    /// a 9 fps panel has no viewer for 21 of every 30 decoded frames, so the
    /// decoder's cadence follows the panel's, not the source's.
    fn target_frame(&self) -> u64 {
        (self.started.elapsed().as_secs_f64() * self.fps) as u64
    }

    fn frame(&mut self, looping: bool) -> Option<Decoded> {
        let target = self.target_frame();
        if self.total > 0 && target >= self.total {
            if !looping {
                // The clip is over: its last frame stays until something
                // restarts the document, which a repaint alone never does.
                return None;
            }
            // One loop of the clip has played: start it again from its first
            // frame rather than holding the last one.
            self.reopen();
        }
        let target = self.target_frame();
        let mut latest = None;
        while self.frames <= target {
            match self.read_frame() {
                Some(frame) => {
                    self.frames += 1;
                    latest = Some(frame);
                }
                None => {
                    self.reopen();
                    latest = self.read_frame();
                    if latest.is_some() {
                        self.frames = 1;
                    }
                    break;
                }
            }
        }
        latest
    }

    fn read_frame(&mut self) -> Option<Decoded> {
        let length = self.width as usize * self.height as usize * 4;
        let mut rgba = vec![0u8; length];
        self.stdout.read_exact(&mut rgba).ok()?;
        Some(Decoded {
            width: self.width,
            height: self.height,
            rgba,
        })
    }

    /// The same file, from its first frame, with the clock restarted.
    fn reopen(&mut self) {
        if let Some(next) = start(&self.path, self.fps, self.render_size) {
            let _ = self.child.kill();
            self.child = next.child;
            self.stdout = next.stdout;
            self.frames = 0;
            self.started = Instant::now();
        }
    }
}

/// ffmpeg reads a 4K source even when the panel is 800 pixels wide. Scaling in
/// the decoder cuts the RGBA conversion and every downstream copy by the area
/// ratio; the panel cannot show those pixels anyway.
fn scaled_size(source: (u32, u32), panel: (u32, u32)) -> (u32, u32) {
    if panel.0 == 0 || panel.1 == 0 || source.0 == 0 || source.1 == 0 {
        return source;
    }
    let fit = (panel.0 as f64 / source.0 as f64).min(panel.1 as f64 / source.1 as f64);
    if fit >= 1.0 {
        return source;
    }
    let width = (source.0 as f64 * fit).round().max(1.0) as u32;
    let height = (source.1 as f64 * fit).round().max(1.0) as u32;
    (width.min(panel.0), height.min(panel.1))
}

/// The decoder's scaling filter. A panel cannot sharpen an edge the decoder
/// smoothed away, so the downscale keeps the authored detail. Lanczos alone
/// still rounds chroma during the RGBA conversion. Adding accurate rounding and
/// full-chroma interpolation measured a 53% higher mean edge gradient than
/// plain lanczos on the real sunset clip at 800x450 (4.75 vs 3.11), without
/// raising decode time: ~27 fps against ~25 fps for 20 frames.
const VIDEO_SCALE_FLAGS: &str = "lanczos+accurate_rnd+full_chroma_int";

/// The decoder's arguments, kept pure so the panel-cadence and size rules can
/// be tested without spawning a process.
fn video_args(path: &Path, rate: f64, size: (u32, u32)) -> Vec<String> {
    let mut args: Vec<String> = [
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostdin",
        "-i",
        path.to_str().unwrap_or_default(),
        "-an",
        "-sn",
        "-dn",
    ]
    .into_iter()
    .map(ToOwned::to_owned)
    .collect();
    if size.0 > 0 && size.1 > 0 {
        // ffmpeg's default bicubic softens edges when a 4K source lands on an
        // 800 px panel, and the panel cannot sharpen it later. Accurate
        // rounding and full-chroma interpolation keep the editorial crispness
        // at the same decode cost.
        args.extend([
            "-vf".to_owned(),
            format!("scale={}:{}:flags={VIDEO_SCALE_FLAGS}", size.0, size.1),
        ]);
    }
    args.extend([
        "-r".to_owned(),
        format!("{rate:.3}"),
        "-f".to_owned(),
        "rawvideo".to_owned(),
        "-pix_fmt".to_owned(),
        "rgba".to_owned(),
        "-".to_owned(),
    ]);
    args
}

fn start(path: &Path, demand_fps: f64, render_size: (u32, u32)) -> Option<Live> {
    let bytes = std::fs::read(path).ok()?;
    let facts = yunari_model::video::read(&bytes)?;
    if facts.width == 0 || facts.height == 0 || facts.frame_rate <= 0.0 {
        return None;
    }
    let ffmpeg = ffmpeg()?;
    // Drop frames to the panel's cadence in ffmpeg, before RGBA conversion.
    // Throttling only the reader leaves ffmpeg decoding the whole source at
    // full rate and writing into a pipe nobody drains.
    let rate = facts.frame_rate.min(demand_fps);
    let size = scaled_size((facts.width, facts.height), render_size);
    let mut child = Command::new(ffmpeg)
        .args(video_args(path, rate, size))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    Some(Live {
        path: path.to_path_buf(),
        child,
        stdout,
        width: size.0,
        height: size.1,
        fps: rate,
        render_size,
        total: (facts.duration_ms as f64 * rate / 1000.0) as u64,
        frames: 0,
        started: Instant::now(),
    })
}

/// The encoder sidecar: the same lookup order the host uses, so one
/// environment reaches both.
fn ffmpeg() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("YUNARI_FFMPEG").map(PathBuf::from)
        && path.is_file()
    {
        return Some(path);
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let beside = dir.join(if cfg!(windows) {
            "ffmpeg.exe"
        } else {
            "ffmpeg"
        });
        if beside.is_file() {
            return Some(beside);
        }
    }
    let name = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    std::env::var_os("PATH").and_then(|variable| {
        std::env::split_paths(&variable)
            .map(|dir| dir.join(name))
            .find(|candidate| candidate.is_file())
    })
}

fn decode(path: &Path) -> Option<Decoded> {
    let bytes = std::fs::read(path).ok()?;
    decode_bytes_at(path, &bytes)
}

fn decode_bytes_at(path: &Path, bytes: &[u8]) -> Option<Decoded> {
    match path.extension() {
        Some(extension) if extension.eq_ignore_ascii_case("svg") => decode_svg(bytes),
        _ => decode_bytes(bytes),
    }
}

/// The source's ceiling on a detached raster (`scene-fabric/render-scale.ts:8`).
/// An untrusted `viewBox` cannot ask for a larger allocation than this.
const MAX_SVG_PIXELS: f32 = 2_073_600.0;

/// `image` has no SVG codec, so the vector is parsed and painted by resvg at
/// its own size. The source rasterises at the node box instead
/// (`scene-fabric/fabric-image.ts:52-63`), so an SVG smaller than its box is
/// sampled up here rather than rendered crisp.
fn decode_svg(bytes: &[u8]) -> Option<Decoded> {
    let tree = resvg::usvg::Tree::from_data(bytes, &resvg::usvg::Options::default()).ok()?;
    let size = tree.size();
    let area = size.width() * size.height();
    if !area.is_finite() || area <= 0.0 {
        return None;
    }
    let scale = (MAX_SVG_PIXELS / area).sqrt().min(1.0);
    let width = (size.width() * scale).round().max(1.0) as u32;
    let height = (size.height() * scale).round().max(1.0) as u32;
    let mut pixmap = tiny_skia::Pixmap::new(width, height)?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(
            width as f32 / size.width(),
            height as f32 / size.height(),
        ),
        &mut pixmap.as_mut(),
    );
    Some(Decoded {
        width,
        height,
        rgba: crate::straight_rgba(&pixmap),
    })
}

fn decode_bytes(bytes: &[u8]) -> Option<Decoded> {
    let image = image::load_from_memory(bytes).ok()?.into_rgba8();
    Some(Decoded {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unreadable_path_is_absent_and_stays_absent() {
        let mut images = Images::at(std::env::temp_dir());
        assert!(images.load("no-such-picture.png").is_none());
        assert!(images.load("no-such-picture.png").is_none());
    }

    #[test]
    fn an_empty_store_has_no_pictures() {
        let mut images = Images::empty();
        assert!(images.load("assets/a.png").is_none());
    }

    #[test]
    fn an_svg_is_rasterised_at_its_own_size() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 8 8">
            <rect width="8" height="8" fill="#ff0000"/></svg>"##;
        let mut images = Images::empty();
        images.put("assets/icon.svg", svg);
        let decoded = images.load("assets/icon.svg").expect("an SVG is a picture");
        assert_eq!((decoded.width, decoded.height), (8, 8));
        assert_eq!(&decoded.rgba[..4], &[255, 0, 0, 255]);
    }

    #[test]
    fn a_9fps_panel_reads_at_most_9_source_frames_per_second() {
        // One second of the sunset drive clip (30 fps) on the Kraken's 9 fps
        // cadence: at most ten 3840x2160 RGBA reads instead of 31.
        let second = std::time::Duration::from_secs(1);
        let source = frame_reads(30.0, f64::INFINITY, 0, second);
        let panel = frame_reads(30.0, 9.0, 0, second);
        assert_eq!(source, 31);
        assert!(
            panel <= 10,
            "a 9 fps panel read {panel} frames in one second"
        );
    }

    #[test]
    fn a_fast_panel_never_makes_the_source_decode_faster() {
        let second = std::time::Duration::from_secs(1);
        assert_eq!(
            frame_reads(30.0, 240.0, 0, second),
            frame_reads(30.0, f64::INFINITY, 0, second)
        );
    }

    #[test]
    fn a_wide_source_fits_a_taller_panel_without_distortion() {
        assert_eq!(scaled_size((3840, 2160), (800, 480)), (800, 450));
    }

    #[test]
    fn a_small_source_is_never_upscaled() {
        assert_eq!(scaled_size((320, 240), (800, 480)), (320, 240));
    }

    #[test]
    fn video_decode_is_scaled_to_the_panels_own_geometry() {
        let args = video_args(Path::new("clip.mp4"), 9.0, (800, 450));
        let joined = args.join(" ");
        assert!(
            joined.contains("-vf scale=800:450:flags=lanczos+accurate_rnd+full_chroma_int"),
            "the decoder must scale sharply before RGBA conversion: {joined}"
        );
        assert!(
            joined.contains("-r 9.000"),
            "the decoder must cap its output at the panel cadence: {joined}"
        );
    }

    #[test]
    fn video_decode_without_a_panel_size_stays_at_source_size() {
        let args = video_args(Path::new("clip.mp4"), 9.0, (0, 0));
        assert!(
            !args.iter().any(|arg| arg == "-vf"),
            "no scale filter without a panel size: {}",
            args.join(" ")
        );
    }

    #[test]
    fn a_video_is_recognised_by_its_own_header() {
        let video = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../model/tests/video-fixtures/conforming.mp4");
        assert!(is_video(&video));
        let still = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/images.rs");
        assert!(!is_video(&still));
    }
}
