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
    fps: f64,
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
        }
    }

    /// No pictures: every image node draws nothing. A frame rendered without an
    /// asset source says so rather than inventing a picture.
    pub fn empty() -> Self {
        Self::default()
    }

    /// A picture the caller holds in memory, such as one just imported and not
    /// yet saved. It takes the same path the document will store it under.
    pub fn put(&mut self, path: &str, bytes: &[u8]) {
        let decoded = decode_bytes_at(Path::new(path), bytes);
        self.cache.insert(path.to_owned(), decoded);
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
                    let video = start(&file).map_or(Video::Failed, Video::Live);
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

impl Live {
    fn frame(&mut self, looping: bool) -> Option<Decoded> {
        let target = (self.started.elapsed().as_secs_f64() * self.fps) as u64;
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
        let target = (self.started.elapsed().as_secs_f64() * self.fps) as u64;
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
        if let Some(next) = start(&self.path) {
            let _ = self.child.kill();
            self.child = next.child;
            self.stdout = next.stdout;
            self.frames = 0;
            self.started = Instant::now();
        }
    }
}

fn start(path: &Path) -> Option<Live> {
    let bytes = std::fs::read(path).ok()?;
    let facts = yunari_model::video::read(&bytes)?;
    if facts.width == 0 || facts.height == 0 || facts.frame_rate <= 0.0 {
        return None;
    }
    let ffmpeg = ffmpeg()?;
    let mut child = Command::new(ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-i",
            path.to_str()?,
            "-an",
            "-sn",
            "-dn",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-",
        ])
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
        width: facts.width,
        height: facts.height,
        fps: facts.frame_rate,
        total: (facts.duration_ms as f64 * facts.frame_rate / 1000.0) as u64,
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
    fn a_video_is_recognised_by_its_own_header() {
        let video = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../model/tests/video-fixtures/conforming.mp4");
        assert!(is_video(&video));
        let still = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/images.rs");
        assert!(!is_video(&still));
    }
}
