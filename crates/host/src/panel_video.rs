//! The two clips a panel plays itself: a TURZX MP4 it decodes from storage and
//! the TUR USB's raw H.264 stream.
//!
//! **The encoder is a sidecar, not a bundled binary.** This build runs an
//! ffmpeg the owner points it at: `YUNARI_FFMPEG`, then `ffmpeg.exe` beside
//! this executable, then the one on `PATH`. A host with none says so in words
//! and leaves the panel on still frames rather than showing black.
//!
//! The flags and filter graphs are the source's own. The TURZX clip is
//! 800×480 at 30 fps, High profile, CRF 23, `+faststart`; the TUR USB stream is
//! the vendor's own `libx264 -x264opts bframes=0 -threads 4 -preset ultrafast`,
//! because its decoder waits on any frame that reorders.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

use yunari_render::video::{self, EncodeFilter, PanelRotation, PanelTurn, Placement, SourceFacts};

/// How the TURZX output is keyed: moves whenever the arguments or the filter
/// change, so a clip encoded by an older transform is not served.
pub const TRANSCODE_VERSION: &str = "v5";
/// The cache's byte bound; eviction drops the least recently used clip first.
pub const CACHE_BYTES: u64 = 256 * 1024 * 1024;
/// ffmpeg's log goes to stderr; a runaway log must not fill memory.
const MAX_STDERR_BYTES: usize = 4 * 1024 * 1024;

static SCRATCH: AtomicU32 = AtomicU32::new(0);

/// What one encoder process answered.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub code: i32,
    pub stderr: String,
}

/// The testable seam: run ffmpeg with these arguments.
pub type Run<'a> = dyn Fn(&[String]) -> Result<Outcome, String> + 'a;

/// One clip ready to feed: the elementary stream and how long it plays.
#[derive(Debug, Clone, PartialEq)]
pub struct PanelClip {
    pub h264: Vec<u8>,
    /// The clip's length in seconds, or 0 when ffmpeg did not report one.
    pub seconds: f64,
}

/// Where the panel's disposable cache lives, beside the other host caches.
pub fn cache_dir(settings: &Path) -> PathBuf {
    settings
        .parent()
        .unwrap_or(settings)
        .join("cache")
        .join("panel-video")
}

/// The ffmpeg this host will run.
#[derive(Debug, Clone)]
pub struct Encoder {
    path: PathBuf,
}

impl Encoder {
    /// Finds the encoder, or says where one was looked for.
    pub fn find() -> Result<Self, String> {
        if let Some(path) = std::env::var_os("YUNARI_FFMPEG").map(PathBuf::from)
            && path.is_file()
        {
            return Ok(Self { path });
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
                return Ok(Self { path: beside });
            }
        }
        if let Some(path) = search_path() {
            return Ok(Self { path });
        }
        Err(
            "No ffmpeg is available on this host, so the panel video was not encoded. \
             Set YUNARI_FFMPEG or put ffmpeg.exe beside Yunari."
                .to_owned(),
        )
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The MP4 the TURZX decodes itself, from the cache when its key is there.
    pub fn transcode_mp4(
        &self,
        bytes: &[u8],
        placement: &Placement,
        rotation: PanelRotation,
        cache: &Path,
    ) -> Result<Vec<u8>, String> {
        let run = spawn(&self.path);
        transcode_with(&run, bytes, placement, rotation, cache)
    }

    /// The TUR USB's elementary stream and its playing length.
    pub fn encode_h264(
        &self,
        mp4: &[u8],
        box_: (f32, f32),
        placement: &Placement,
        rotation: PanelRotation,
    ) -> Result<PanelClip, String> {
        let directory = Scratch::new()?;
        let input = directory.path.join("in.mp4");
        let output = directory.path.join("out.h264");
        std::fs::write(&input, mp4)
            .map_err(|error| format!("The input could not be staged: {error}"))?;

        let facts = yunari_model::video::read(mp4).map(|facts| SourceFacts {
            width: facts.width as f32,
            height: facts.height as f32,
            duration_ms: Some(facts.duration_ms as f64),
        });
        let filter = video::panel_filter(
            video::rotation_box(box_, rotation),
            placement,
            facts,
            rotation,
        );
        let args = h264_args(&input, &output, &filter);
        let outcome = spawn(&self.path)(&args)?;
        if outcome.code != 0 {
            return Err(format!(
                "The panel video could not be encoded; ffmpeg exited with code {}.",
                outcome.code
            ));
        }
        let h264 = std::fs::read(&output)
            .map_err(|_| "ffmpeg finished but wrote no panel clip.".to_owned())?;
        Ok(PanelClip {
            h264,
            seconds: clip_seconds(&outcome.stderr),
        })
    }
}

/// The TURZX's own wire glass.
const BOX: (f32, f32) = (800.0, 480.0);

fn search_path() -> Option<PathBuf> {
    let variable = std::env::var_os("PATH")?;
    let name = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    std::env::split_paths(&variable)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// The TURZX encode, in the source's own order: compose, bound the composed
/// graph by the source's length, then the panel's MP4 profile.
pub fn encode_args(input: &Path, output: &Path, filter: &EncodeFilter) -> Vec<String> {
    let mut args = common(input, filter);
    args.extend([
        "-r".to_owned(),
        "30".to_owned(),
        "-c:v".to_owned(),
        "libx264".to_owned(),
        "-profile:v".to_owned(),
        "high".to_owned(),
        "-pix_fmt".to_owned(),
        "yuv420p".to_owned(),
        "-crf".to_owned(),
        "23".to_owned(),
        "-movflags".to_owned(),
        "+faststart".to_owned(),
        "-f".to_owned(),
        "mp4".to_owned(),
    ]);
    args.push(display(output));
    args
}

/// The TUR USB encode: the vendor's `ThemeController.TransferH264` flags.
pub fn h264_args(input: &Path, output: &Path, filter: &EncodeFilter) -> Vec<String> {
    let mut args = common(input, filter);
    args.extend([
        "-c:v".to_owned(),
        "libx264".to_owned(),
        // The panel's decoder reads 4:2:0 only, and an authored clip may not.
        "-pix_fmt".to_owned(),
        "yuv420p".to_owned(),
        "-x264opts".to_owned(),
        "bframes=0".to_owned(),
        "-threads".to_owned(),
        "4".to_owned(),
        "-preset".to_owned(),
        "ultrafast".to_owned(),
        "-f".to_owned(),
        "h264".to_owned(),
    ]);
    args.push(display(output));
    args
}

fn common(input: &Path, filter: &EncodeFilter) -> Vec<String> {
    let mut args: Vec<String> = ["-hide_banner", "-nostdin", "-y", "-i"]
        .into_iter()
        .map(ToOwned::to_owned)
        .collect();
    args.push(display(input));
    args.push("-an".to_owned());
    if filter.complex {
        args.extend([
            "-filter_complex".to_owned(),
            filter.filter.clone(),
            "-map".to_owned(),
            "[out]".to_owned(),
        ]);
    } else {
        args.extend(["-vf".to_owned(), filter.filter.clone()]);
    }
    if let Some(seconds) = filter.duration_seconds {
        args.extend(["-t".to_owned(), format!("{seconds}")]);
    }
    args
}

fn display(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// ffmpeg prints `Duration: HH:MM:SS.xx` once per input, before encoding.
pub fn clip_seconds(stderr: &str) -> f64 {
    for line in stderr.lines() {
        let Some(rest) = line.split("Duration:").nth(1) else {
            continue;
        };
        let rest = rest.trim_start();
        let Some(clock) = rest.split(',').next() else {
            continue;
        };
        let parts: Vec<&str> = clock.trim().split(':').collect();
        if parts.len() != 3 {
            continue;
        }
        let (Ok(hours), Ok(minutes), Ok(seconds)) = (
            parts[0].trim().parse::<f64>(),
            parts[1].parse::<f64>(),
            parts[2].parse::<f64>(),
        ) else {
            continue;
        };
        let total = hours * 3600.0 + minutes * 60.0 + seconds;
        // A zero or absurd duration is not a length the feed can pace against.
        return if total.is_finite() && total > 0.0 {
            total
        } else {
            0.0
        };
    }
    0.0
}

fn spawn(ffmpeg: &Path) -> impl Fn(&[String]) -> Result<Outcome, String> + '_ {
    move |args| {
        let mut child = Command::new(ffmpeg)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("The encoder could not start: {error}"))?;
        let mut stderr = Vec::new();
        if let Some(mut pipe) = child.stderr.take() {
            use std::io::Read;
            let _ = std::io::copy(
                &mut (&mut pipe).take(MAX_STDERR_BYTES as u64 + 1),
                &mut stderr,
            );
            // Keep the tail: a long log's last words are the failure.
            if stderr.len() > MAX_STDERR_BYTES {
                stderr.drain(..stderr.len() - MAX_STDERR_BYTES);
            }
        }
        let status = child
            .wait()
            .map_err(|error| format!("The encoder could not be waited on: {error}"))?;
        Ok(Outcome {
            code: status.code().unwrap_or(1),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
        })
    }
}

/// A scratch directory that removes itself, named so two encodes cannot
/// collide.
struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new() -> Result<Self, String> {
        let count = SCRATCH.fetch_add(1, Ordering::SeqCst);
        let path =
            std::env::temp_dir().join(format!("yunari-panel-video-{}-{count}", std::process::id()));
        std::fs::create_dir_all(&path)
            .map_err(|error| format!("A scratch directory could not be made: {error}"))?;
        Ok(Self { path })
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// The cache file a key names.
fn cached(directory: &Path, key: &str) -> PathBuf {
    directory.join(format!("{TRANSCODE_VERSION}-{key}.mp4"))
}

/// The pan, the composition and the turn change the pixels, so all of them
/// are in the key: the same source authored elsewhere or played on another
/// mount is a different clip.
fn cache_key(bytes: &[u8], placement: &Placement, rotation: PanelRotation) -> String {
    let turn = match rotation {
        PanelRotation::Turn(turn) => serde_json::json!({
            "kind": "turn",
            "turn": match turn {
                PanelTurn::None => "none",
                PanelTurn::Cw => "cw",
                PanelTurn::Half => "half",
                PanelTurn::Ccw => "ccw",
            },
        }),
        PanelRotation::Angle(degrees) => serde_json::json!({
            "kind": "angle",
            "degrees": degrees,
        }),
    };
    let placed = serde_json::json!({
        "offsetX": placement.offset_x,
        "offsetY": placement.offset_y,
        "scale": placement.scale,
        "flipX": placement.flip_x,
        "flipY": placement.flip_y,
        "composition": placement.composition,
        "fill": placement.fill,
        "shape": match placement.shape {
            video::PanelShape::Rect => "rect",
            video::PanelShape::Circle => "circle",
        },
    });
    let mut material = bytes.to_vec();
    material.push(0);
    material.extend(serde_json::to_vec(&turn).unwrap_or_default());
    material.push(0);
    material.extend(serde_json::to_vec(&placed).unwrap_or_default());
    yunari_library::sha256_hex(&material)
}

/// Writes through a sibling scratch file, fsynced before the rename, so an
/// interrupted write leaves a `.tmp` nobody reuses.
fn write_atomic(directory: &Path, file: &Path, data: &[u8]) -> std::io::Result<()> {
    std::fs::create_dir_all(directory)?;
    let count = SCRATCH.fetch_add(1, Ordering::SeqCst);
    let name = file
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let scratch = directory.join(format!(".{name}.{}.{count}.tmp", std::process::id()));
    {
        use std::io::Write;
        let mut handle = std::fs::File::create(&scratch)?;
        handle.write_all(data)?;
        handle.sync_all()?;
    }
    match std::fs::rename(&scratch, file) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = std::fs::remove_file(&scratch);
            Err(error)
        }
    }
}

/// Drops the least recently used cached clips until the directory fits the
/// bound. Only names this cache writes are candidates; the file just written is
/// kept even if it alone is over the bound.
fn evict(directory: &Path, keep: &Path) -> std::io::Result<()> {
    let mut entries: Vec<(PathBuf, u64, std::time::SystemTime)> = Vec::new();
    let Ok(names) = std::fs::read_dir(directory) else {
        return Ok(());
    };
    for entry in names.flatten() {
        let path = entry.path();
        let Some(name) = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
        else {
            continue;
        };
        if !name.starts_with(&format!("{TRANSCODE_VERSION}-")) || !name.ends_with(".mp4") {
            continue;
        }
        if let Ok(meta) = entry.metadata() {
            let used = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
            entries.push((path, meta.len(), used));
        }
    }

    let mut total: u64 = entries.iter().map(|(_, size, _)| *size).sum();
    entries.sort_by_key(|(_, _, used)| *used);
    for (path, size, _) in entries {
        if total <= CACHE_BYTES {
            break;
        }
        if path == keep {
            continue;
        }
        if std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(size);
        }
    }
    Ok(())
}

/// One TURZX transcode over an injected runner, so the encoder seam and its
/// cache are testable without ffmpeg.
fn transcode_with(
    run: &Run<'_>,
    bytes: &[u8],
    placement: &Placement,
    rotation: PanelRotation,
    cache: &Path,
) -> Result<Vec<u8>, String> {
    let key = cache_key(bytes, placement, rotation);
    let at = cached(cache, &key);
    if let Ok(hit) = std::fs::read(&at) {
        // A hit is a use: eviction drops the least recently used clip first.
        // `set_len` on the same length is the touch this std offers.
        if let Ok(file) = std::fs::OpenOptions::new().write(true).open(&at) {
            let _ = file.set_len(hit.len() as u64);
        }
        return Ok(hit);
    }

    let facts = yunari_model::video::read(bytes).map(|facts| SourceFacts {
        width: facts.width as f32,
        height: facts.height as f32,
        duration_ms: Some(facts.duration_ms as f64),
    });
    let filter = video::panel_filter(
        video::rotation_box((BOX.0, BOX.1), rotation),
        placement,
        facts,
        rotation,
    );

    let directory = Scratch::new()?;
    let input = directory.path.join("in.mp4");
    let output = directory.path.join("out.mp4");
    std::fs::write(&input, bytes)
        .map_err(|error| format!("The input could not be staged: {error}"))?;
    let outcome = run(&encode_args(&input, &output, &filter))?;
    if outcome.code != 0 {
        return Err(format!(
            "The panel video could not be encoded; ffmpeg exited with code {}.",
            outcome.code
        ));
    }
    let encoded = std::fs::read(&output)
        .map_err(|_| "ffmpeg finished but wrote no panel video.".to_owned())?;

    // A cache that cannot be written is not a failed transcode.
    let _ = write_atomic(cache, &at, &encoded);
    let _ = evict(cache, &at);
    Ok(encoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(code: i32, stderr: &str) -> Outcome {
        Outcome {
            code,
            stderr: stderr.to_owned(),
        }
    }

    fn placement() -> Placement {
        Placement {
            scale: 1.0,
            ..Placement::default()
        }
    }

    #[test]
    fn the_turzx_clip_is_the_panels_own_profile() {
        let filter = video::panel_filter(
            (800.0, 480.0),
            &placement(),
            None,
            PanelRotation::Turn(PanelTurn::None),
        );
        let args = encode_args(Path::new("in.mp4"), Path::new("out.mp4"), &filter);
        let joined = args.join(" ");
        assert!(joined.contains("-vf scale=800:480"), "{joined}");
        assert!(joined.contains("-r 30"), "{joined}");
        assert!(joined.contains("-c:v libx264"), "{joined}");
        assert!(joined.contains("-profile:v high"), "{joined}");
        assert!(joined.contains("-crf 23"), "{joined}");
        assert!(joined.contains("-movflags +faststart"), "{joined}");
        assert!(args.ends_with(&["-f".to_owned(), "mp4".to_owned(), "out.mp4".to_owned()]));
    }

    #[test]
    fn the_tur_usb_stream_is_the_vendors_own_encode() {
        let filter = video::panel_filter(
            (480.0, 800.0),
            &placement(),
            None,
            PanelRotation::Turn(PanelTurn::None),
        );
        let args = h264_args(Path::new("in.mp4"), Path::new("out.h264"), &filter);
        let joined = args.join(" ");
        assert!(joined.contains("-pix_fmt yuv420p"), "{joined}");
        assert!(joined.contains("-x264opts bframes=0"), "{joined}");
        assert!(joined.contains("-threads 4"), "{joined}");
        assert!(joined.contains("-preset ultrafast"), "{joined}");
        assert!(joined.contains("-f h264"), "{joined}");
        // The vendor fixes no frame rate and no bitrate.
        assert!(!joined.contains("-r "), "{joined}");
        assert!(!joined.contains("-b:v"), "{joined}");
    }

    #[test]
    fn a_composed_filter_takes_the_complex_path_and_the_sources_length() {
        let filter = video::panel_filter(
            (800.0, 480.0),
            &Placement {
                composition: Some(yunari_model::document::BackgroundComposition {
                    anchor: yunari_model::document::BackgroundAnchor::Top,
                    extent: Some(0.5),
                    fade: None,
                }),
                scale: 1.0,
                ..Placement::default()
            },
            Some(SourceFacts {
                width: 320.0,
                height: 180.0,
                duration_ms: Some(2_000.0),
            }),
            PanelRotation::Turn(PanelTurn::None),
        );
        let args = encode_args(Path::new("in.mp4"), Path::new("out.mp4"), &filter);
        assert!(args.iter().any(|arg| arg == "-filter_complex"));
        assert!(args.iter().any(|arg| arg == "-map"));
        let time = args.iter().position(|arg| arg == "-t").expect("a duration");
        assert_eq!(args[time + 1], "2");
    }

    #[test]
    fn ffmpegs_duration_line_becomes_the_clips_seconds() {
        assert_eq!(clip_seconds("  Duration: 00:00:02.13, start: 0.0"), 2.13);
        assert_eq!(clip_seconds("Duration: 01:02:03.50, bitrate:"), 3723.5);
        assert_eq!(clip_seconds("no duration here"), 0.0);
        assert_eq!(clip_seconds("Duration: N/A, start: 0.0"), 0.0);
    }

    #[test]
    fn a_failed_encoder_answers_the_sentence_the_runner_reports() {
        let directory =
            std::env::temp_dir().join(format!("yunari-panel-video-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let run = |_: &[String]| Ok(outcome(1, "boom"));
        let result = transcode_with(
            &run,
            b"bytes",
            &placement(),
            PanelRotation::Turn(PanelTurn::None),
            &directory,
        );
        assert_eq!(
            result.unwrap_err(),
            "The panel video could not be encoded; ffmpeg exited with code 1."
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_cache_hit_serves_the_clip_without_running_the_encoder() {
        let directory =
            std::env::temp_dir().join(format!("yunari-panel-video-hit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let calls = std::cell::Cell::new(0);
        let run = |args: &[String]| {
            calls.set(calls.get() + 1);
            let out = PathBuf::from(args.last().expect("an output path"));
            std::fs::write(&out, b"encoded").unwrap();
            Ok(outcome(0, "Duration: 00:00:05.00, start: 0.0"))
        };
        let first = transcode_with(
            &run,
            b"bytes",
            &placement(),
            PanelRotation::Turn(PanelTurn::None),
            &directory,
        )
        .expect("encoded");
        assert_eq!(first, b"encoded");
        assert_eq!(calls.get(), 1);

        let second = transcode_with(
            &run,
            b"bytes",
            &placement(),
            PanelRotation::Turn(PanelTurn::None),
            &directory,
        )
        .expect("cached");
        assert_eq!(second, b"encoded");
        assert_eq!(calls.get(), 1, "the cache answered");
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_turn_is_part_of_the_cache_key() {
        let none = cache_key(b"same", &placement(), PanelRotation::Turn(PanelTurn::None));
        let turned = cache_key(b"same", &placement(), PanelRotation::Turn(PanelTurn::Cw));
        assert_ne!(none, turned);
    }
}
