use std::path::Path;

use yunari_golden::{GoldenConfig, assert_golden, compare};

#[test]
fn identical_pixels_pass() {
    assert!(compare(&[0, 0, 0, 255], &[0, 0, 0, 255], GoldenConfig::default()).is_ok());
}

#[test]
fn one_channel_within_tolerance_passes() {
    let config = GoldenConfig {
        channel_tolerance: 2,
        max_diff_ratio: 0.0,
    };
    assert!(compare(&[10, 10, 10, 255], &[12, 10, 10, 255], config).is_ok());
}

#[test]
fn channel_beyond_tolerance_fails_with_counts() {
    let config = GoldenConfig {
        channel_tolerance: 2,
        max_diff_ratio: 0.0,
    };
    let mismatch = compare(
        &[10, 10, 10, 255, 10, 10, 10, 255],
        &[10, 10, 10, 255, 30, 10, 10, 255],
        config,
    )
    .expect_err("differs");
    assert_eq!(mismatch.differing_pixels, 1);
    assert_eq!(mismatch.total_pixels, 2);
    assert_eq!(mismatch.max_channel_delta, 20);
}

#[test]
fn differing_ratio_beyond_limit_fails() {
    let config = GoldenConfig {
        channel_tolerance: 0,
        max_diff_ratio: 0.0,
    };
    assert!(compare(&[0; 16], &[1; 16], config).is_err());
}

#[test]
fn complains_about_length_mismatch() {
    let mismatch = compare(&[0; 4], &[0; 8], GoldenConfig::default()).expect_err("lengths");
    assert_eq!(
        mismatch.message(),
        "expected and actual buffers differ in length"
    );
}

#[test]
fn writes_and_reads_a_golden() {
    let dir = std::env::temp_dir().join("yunari-golden-test");
    let _ = std::fs::remove_dir_all(&dir);
    let rgba = [255u8, 0, 0, 255].repeat(16);
    yunari_golden::write_golden(&dir, "red-4x4", 4, 4, &rgba).expect("write");
    assert_golden(&dir, "red-4x4", 4, 4, &rgba, GoldenConfig::default());
    assert!(yunari_golden::golden_path(&dir, "red-4x4").is_file());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn committed_red_golden_matches() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/goldens");
    let rgba = [255u8, 0, 0, 255].repeat(16);
    assert_golden(&dir, "red-4x4", 4, 4, &rgba, GoldenConfig::default());
}
