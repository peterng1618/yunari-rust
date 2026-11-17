# Coverage-gaps raster probe

Headless observations for the R3/R6/R8/R9/R14/R15 audit. It uses only the
port's public crates, writes every fixture into a fresh temp directory, and
prints one line per observation.

Run from the repository root so Cargo picks up `.cargo/config.toml`:

```powershell
$env:CARGO_TARGET_DIR = "target"
cargo run --manifest-path docs/parity/probes/coverage-gaps/Cargo.toml -- <mode>
```

Modes: `svg`, `avif`, `vertical`, `letter-spacing`, `overflow`, `short-box`,
`glass-grain`, `glass-sampling`, `content-fit`, `video`.

`glass-sampling` and `video` need ffmpeg. Set `YUNARI_FFMPEG` to an executable,
or put `ffmpeg.exe` on `PATH`; the probe prints `skipped: no ffmpeg` rather than
drawing a false conclusion.
