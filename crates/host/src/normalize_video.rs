//! Brings a source video inside the theme's publish envelope, or copies it
//! untouched.
//!
//! A conforming file is returned as the same bytes: re-encoding an asset that
//! already passes would degrade it a little on every resubmission. A file
//! longer than the envelope is refused rather than truncated, because a cut
//! background loop is not the one the author made. Audio and a missing
//! `+faststart` are fixed by a stream copy; everything else re-encodes with the
//! source's capped-CRF ladder.
//!
//! **The encoder is the panel sidecar**, not a bundled binary: `YUNARI_FFMPEG`,
//! then `ffmpeg.exe` beside this executable, then the one on `PATH`.

use std::path::Path;

use yunari_library::store::MAX_ASSET_BYTES;
use yunari_model::video::{self, Facts, MAX_DURATION_MS, MAX_FRAME_RATE, MAX_LONG_EDGE};

use crate::panel_video::{Encoder, Outcome, Run, Scratch, spawn};

/// One re-encoded or copied video, and which of the two it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Normalized {
    pub bytes: Vec<u8>,
    pub reencoded: bool,
}

/// A file the envelope refuses before the encoder runs.
const UNREADABLE: &str = "That video cannot be read; an H.264 MP4 is required.";
const ENCODE_FAILED: &str = "That video could not be re-encoded.";
const NOT_CONFORMING: &str = "That video could not be re-encoded into the publish envelope; the encoder's output does not conform.";

pub fn normalize(bytes: &[u8]) -> Result<Normalized, String> {
    // A file that already conforms costs no encoder, so a host without one can
    // still import it.
    if let Some(copied) = conforming(bytes)? {
        return Ok(copied);
    }
    let encoder = Encoder::find().map_err(|_| {
        "No ffmpeg encoder is available on this host, so the video cannot be re-encoded.".to_owned()
    })?;
    normalize_with(&spawn(encoder.path()), bytes)
}

/// The file copied as it is, or `None` when it has to be re-encoded. A file
/// longer than the envelope is refused rather than truncated.
fn conforming(bytes: &[u8]) -> Result<Option<Normalized>, String> {
    let facts = video::read(bytes).ok_or_else(|| UNREADABLE.to_owned())?;
    if video::refusals("video", &facts).is_empty() {
        return Ok(Some(Normalized {
            bytes: bytes.to_vec(),
            reencoded: false,
        }));
    }
    if facts.duration_ms > MAX_DURATION_MS {
        return Err(format!(
            "That video is {} ms long; the maximum is {MAX_DURATION_MS} ms. Trim it and try again.",
            facts.duration_ms
        ));
    }
    Ok(None)
}

/// The testable seam: the encoder is injected so a fixture needs no ffmpeg.
fn normalize_with(run: &Run<'_>, bytes: &[u8]) -> Result<Normalized, String> {
    if let Some(copied) = conforming(bytes)? {
        return Ok(copied);
    }
    let facts = video::read(bytes).ok_or_else(|| UNREADABLE.to_owned())?;

    // A copy is lossless, and the output is still re-probed below because the
    // flags are a promise rather than proof.
    let copy_only = copy_remux_suffices(&facts);
    let directory = Scratch::new()?;
    let input = directory.path.join("in.mp4");
    let output = directory.path.join("out.mp4");
    std::fs::write(&input, bytes).map_err(|_| ENCODE_FAILED.to_owned())?;
    let args = if copy_only {
        remux_args(&input, &output)
    } else {
        encode_args(&input, &output, &facts)
    };
    let Outcome { code, .. } = run(&args).map_err(|_| ENCODE_FAILED.to_owned())?;
    if code != 0 {
        return Err(format!(
            "That video could not be re-encoded; ffmpeg exited with code {code}."
        ));
    }
    let encoded = std::fs::read(&output).map_err(|_| ENCODE_FAILED.to_owned())?;
    let ok =
        video::read(&encoded).is_some_and(|facts| video::refusals("normalized", &facts).is_empty());
    if !ok {
        return Err(NOT_CONFORMING.to_owned());
    }
    Ok(Normalized {
        bytes: encoded,
        reencoded: true,
    })
}

/// The file's only faults are audio and/or faststart, so a copy can fix it.
fn copy_remux_suffices(facts: &Facts) -> bool {
    let fixed = Facts {
        has_audio: false,
        faststart: true,
        ..facts.clone()
    };
    video::refusals("video", &fixed).is_empty()
}

/// A copy that drops audio and moves `moov` forward, for the two faults that do
/// not touch the picture.
fn remux_args(input: &Path, output: &Path) -> Vec<String> {
    [
        "-hide_banner",
        "-nostdin",
        "-y",
        "-i",
        input.to_str().unwrap_or_default(),
        "-an",
        "-c:v",
        "copy",
        "-movflags",
        "+faststart",
        "-f",
        "mp4",
        output.to_str().unwrap_or_default(),
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

/// The capped-CRF ladder the source uses for a source it has to re-encode.
fn encode_args(input: &Path, output: &Path, facts: &Facts) -> Vec<String> {
    let (width, height) = box_for_source(facts.width, facts.height);
    let cap = bitrate_cap(width.max(height), facts.duration_ms);
    [
        "-hide_banner",
        "-nostdin",
        "-y",
        "-i",
        input.to_str().unwrap_or_default(),
        "-an",
        "-vf",
        &format!(
            "scale={width}:{height}:force_original_aspect_ratio=decrease:force_divisible_by=2,\
             pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:color=black"
        ),
        "-r",
        &format!("{}", MAX_FRAME_RATE as u32),
        "-c:v",
        "libx264",
        "-profile:v",
        "high",
        "-pix_fmt",
        "yuv420p",
        "-crf",
        "23",
        // CRF alone lets a hard 4K frame spend more than the asset cap allows.
        "-maxrate",
        &cap.to_string(),
        "-bufsize",
        &(cap * 2).to_string(),
        "-movflags",
        "+faststart",
        "-f",
        "mp4",
        output.to_str().unwrap_or_default(),
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

/// The capped, even box a re-encode lands in: the source aspect is preserved
/// and only the long edge is capped.
fn box_for_source(width: u32, height: u32) -> (u32, u32) {
    let long_edge = width.max(height);
    let factor = if long_edge > MAX_LONG_EDGE {
        MAX_LONG_EDGE as f64 / long_edge as f64
    } else {
        1.0
    };
    (
        even_at_most(width as f64 * factor),
        even_at_most(height as f64 * factor),
    )
}

fn even_at_most(value: f64) -> u32 {
    (value / 2.0).floor().max(1.0) as u32 * 2
}

/// The rate cap by output long edge, lowered until 90% of the asset bound
/// covers the whole clip.
fn bitrate_cap(long_edge: u32, duration_ms: u64) -> u64 {
    let ladder = if long_edge <= 1920 {
        6_000_000
    } else if long_edge <= 2560 {
        12_000_000
    } else {
        20_000_000
    };
    let seconds = duration_ms.max(1_000) as f64 / 1000.0;
    let bound = (MAX_ASSET_BYTES as f64 * 0.9 * 8.0 / seconds).floor() as u64;
    ladder.min(bound).max(500_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFORMING: &[u8] = include_bytes!("../../model/tests/video-fixtures/conforming.mp4");

    #[test]
    fn a_conforming_file_is_copied_without_running_the_encoder() {
        let called = std::cell::Cell::new(0);
        let run = |args: &[String]| {
            let _ = args;
            called.set(called.get() + 1);
            Ok(Outcome {
                code: 0,
                stderr: String::new(),
            })
        };
        let normalized = normalize_with(&run, CONFORMING).unwrap();
        assert_eq!(normalized.bytes, CONFORMING);
        assert!(!normalized.reencoded);
        assert_eq!(called.get(), 0);
    }

    #[test]
    fn bytes_that_are_not_an_mp4_are_refused_before_the_encoder() {
        let error = normalize_with(&|_: &[String]| unreachable!(), b"not a video").unwrap_err();
        assert_eq!(error, UNREADABLE);
    }

    #[test]
    fn the_reencode_keeps_the_source_aspect_and_caps_the_long_edge() {
        assert_eq!(box_for_source(7680, 4320), (4096, 2304));
        assert_eq!(box_for_source(3840, 2160), (3840, 2160));
        assert_eq!(box_for_source(1920, 1080), (1920, 1080));
        assert_eq!(box_for_source(1, 1), (2, 2));
    }

    #[test]
    fn the_rate_cap_never_spends_past_the_asset_bound() {
        // A 60-second 4K clip cannot spend the 20 Mb/s ladder rung.
        assert_eq!(bitrate_cap(4096, 60_000), 10_066_329);
        assert_eq!(bitrate_cap(1280, 10_000), 6_000_000);
    }
}
