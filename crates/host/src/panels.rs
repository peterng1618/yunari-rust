//! Frames for the panels this host drives: the theme the display holds, the
//! readings it asks for, fitted into the panel's own glass and encoded the way
//! its device reads pictures.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use yunari_library::Library;
use yunari_model::document::{AssetKind, ThemeDocument};
use yunari_panels::manager::{DisplayRecord, FrameSource, OverlaySource, VideoSource};
use yunari_panels::runner::VideoClip;
use yunari_panels::{PanelShape, StillFormat, VideoForm, for_device};
use yunari_providers::registry::{Ask, Registry};
use yunari_render::Fonts;
use yunari_render_raster::{Images, Raster, fit_into};

/// A JPEG quality of 85 is the source's: a panel's glass hides the difference
/// and every step above it costs wire time on a frame per second.
const JPEG_QUALITY: u8 = 85;

/// The frames one host serves its panels, with the caches a frame needs.
///
/// The font database and the decoded assets are shared by every panel: a
/// second copy would re-read every file once per panel per tick.
pub struct PanelFrames {
    library: Library,
    registry: Arc<Registry>,
    settings_dir: PathBuf,
    fonts: Mutex<Fonts>,
    images: Mutex<Images>,
    /// One document per theme id, so a tick does not read the same file twice.
    documents: Mutex<BTreeMap<String, ThemeDocument>>,
}

impl PanelFrames {
    pub fn new(library: Library, registry: Arc<Registry>, settings_dir: PathBuf) -> Self {
        Self {
            library,
            registry,
            settings_dir,
            fonts: Mutex::new(Fonts::new()),
            images: Mutex::new(Images::empty()),
            documents: Mutex::new(BTreeMap::new()),
        }
    }

    /// The closure a manager hands each runner.
    pub fn source(self: &Arc<Self>) -> FrameSource {
        let frames = Arc::clone(self);
        Arc::new(move |record, width, height, format| frames.frame(record, width, height, format))
    }

    /// The clip resolver a manager hands each runner.
    pub fn video_source(self: &Arc<Self>) -> VideoSource {
        let frames = Arc::clone(self);
        Arc::new(move |record| frames.video_clip(record))
    }

    /// The UI-only capture that rides over a playing clip.
    pub fn overlay_source(self: &Arc<Self>) -> OverlaySource {
        let frames = Arc::clone(self);
        Arc::new(move |record, width, height| frames.overlay(record, width, height))
    }

    /// The encoded clip a display's theme plays itself, or `None` when the
    /// theme has no video background. Anything that cannot be resolved is a
    /// still panel rather than a failed one; only an encode failure is news.
    pub fn video_clip(&self, record: &DisplayRecord) -> Result<Option<VideoClip>, String> {
        let assignment = crate::assignment::Assignment::load(&self.settings_dir);
        let Some(theme) = assignment.resolve(&self.library, &record.id, None) else {
            return Ok(None);
        };
        let Some(document) = self.read_document(&theme) else {
            return Ok(None);
        };
        let Some(media) = document.background.media.as_ref() else {
            return Ok(None);
        };
        let Some(asset) = document
            .assets
            .iter()
            .find(|asset| asset.id == media.asset_id)
        else {
            return Ok(None);
        };
        if asset.kind != AssetKind::Video {
            return Ok(None);
        }
        let Some(bytes) = self
            .library
            .assets(&theme, &document)
            .get(&asset.path)
            .cloned()
        else {
            return Ok(None);
        };
        let device = record
            .device
            .as_ref()
            .ok_or_else(|| "That display names no panel.".to_owned())?;
        let family = for_device(device.vendor_id, device.product_id)
            .ok_or_else(|| "No driver on this host claims that panel.".to_owned())?;
        let Some(form) = family.video else {
            return Ok(None);
        };

        let shape = match family.shape {
            PanelShape::Circle => yunari_render::video::PanelShape::Circle,
            PanelShape::Rect => yunari_render::video::PanelShape::Rect,
        };
        let angle = record.rotation.unwrap_or(0);
        let rotation = yunari_render::video::video_rotation(angle, shape)
            .ok_or_else(|| yunari_render::video::turn_refusal(angle))?;
        let box_ = (family.geometry.width as f32, family.geometry.height as f32);
        let fill = yunari_render::plan::media_fill(&document, box_.0, box_.1);
        let placement = yunari_render::video::placement_of(&document, shape, fill);
        let encoder = crate::panel_video::Encoder::find()?;

        let clip = match form {
            VideoForm::StoredMp4 => VideoClip::Mp4(encoder.transcode_mp4(
                &bytes,
                &placement,
                rotation,
                &crate::panel_video::cache_dir(&self.settings_dir),
            )?),
            VideoForm::StreamH264 => {
                let clip = encoder.encode_h264(&bytes, box_, &placement, rotation)?;
                VideoClip::H264 {
                    bytes: clip.h264,
                    seconds: clip.seconds,
                }
            }
        };
        Ok(Some(clip))
    }

    /// The UI alone as a PNG, with the theme's background left out so the
    /// playing clip shows through its transparency.
    pub fn overlay(
        &self,
        record: &DisplayRecord,
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>, String> {
        let Some(document) = self.document(record) else {
            let plate = Raster {
                width,
                height,
                rgba: vec![0; width as usize * height as usize * 4],
            };
            return encode(&plate, StillFormat::Png);
        };
        let raster = self.raster(&document, record, width, height, false)?;
        encode(&raster, StillFormat::Png)
    }

    /// One still for a record's panel.
    pub fn frame(
        &self,
        record: &DisplayRecord,
        width: u32,
        height: u32,
        format: StillFormat,
    ) -> Result<Vec<u8>, String> {
        let Some(document) = self.document(record) else {
            // A display no theme is assigned still shows a plate: a panel that
            // went black would read as a dead panel rather than a choice.
            let plate = Raster {
                width,
                height,
                rgba: vec![0; width as usize * height as usize * 4],
            };
            return encode(&plate, format);
        };
        self.render(&document, record, width, height, format)
    }

    fn render(
        &self,
        document: &ThemeDocument,
        record: &DisplayRecord,
        width: u32,
        height: u32,
        format: StillFormat,
    ) -> Result<Vec<u8>, String> {
        let raster = self.raster(document, record, width, height, true)?;
        encode(&raster, format)
    }

    /// The document's pixels at the panel's size. `background` false leaves the
    /// backdrop and its paint out, which is the UI a playing clip is seen
    /// through.
    fn raster(
        &self,
        document: &ThemeDocument,
        record: &DisplayRecord,
        width: u32,
        height: u32,
        background: bool,
    ) -> Result<Raster, String> {
        let keys: Vec<String> = document
            .bindings
            .values()
            .flatten()
            .map(|binding| binding.semantic_key.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        let readings = if keys.is_empty() {
            yunari_render::Readings::default()
        } else {
            let now = jiff::Timestamp::now().as_millisecond();
            let cycle = self.registry.sample(
                &[Ask::new(record.id.clone(), keys.clone())],
                u64::try_from(now).unwrap_or_default(),
            );
            crate::frame::readings_for(&cycle, &keys)
        };

        let mut fonts = self
            .fonts
            .lock()
            .map_err(|_| "The font cache is poisoned.")?;
        let mut list = crate::frame::frame(document, &readings, &mut fonts);
        if !background {
            list.background = None;
            list.backdrop = None;
        }
        let round = record
            .device
            .as_ref()
            .and_then(|device| for_device(device.vendor_id, device.product_id))
            .is_some_and(|family| family.shape == PanelShape::Circle);
        let rotation = record.rotation.unwrap_or(0);
        let mut images = self
            .images
            .lock()
            .map_err(|_| "The asset cache is poisoned.")?;
        fit_into(
            &list,
            width,
            height,
            rotation,
            round,
            Some(&mut fonts),
            Some(&mut images),
        )
    }

    /// One display's still as a PNG, the settings surface's preview.
    ///
    /// The theme is the one being considered, not the assignment: a preview is
    /// what an owner is choosing between, and the panel keeps its own.
    pub fn preview_of(&self, theme: &str, record: &DisplayRecord) -> Result<Vec<u8>, String> {
        let family = record
            .device
            .as_ref()
            .and_then(|device| for_device(device.vendor_id, device.product_id))
            .ok_or_else(|| "No driver on this host claims that panel.".to_owned())?;
        let document = self
            .read_document(theme)
            .ok_or_else(|| format!("The theme {theme} could not be read."))?;
        self.render(
            &document,
            record,
            family.geometry.width,
            family.geometry.height,
            StillFormat::Png,
        )
    }

    /// The document a display holds, or `None` when it holds none.
    fn document(&self, record: &DisplayRecord) -> Option<ThemeDocument> {
        let assignment = crate::assignment::Assignment::load(&self.settings_dir);
        let theme = assignment.resolve(&self.library, &record.id, None)?;
        self.read_document(&theme)
    }

    /// One theme's document, read at most once per process.
    fn read_document(&self, theme: &str) -> Option<ThemeDocument> {
        let mut documents = self.documents.lock().ok()?;
        if let Some(document) = documents.get(theme) {
            return Some(document.clone());
        }
        let stored = self.library.read(theme).ok()?;
        // The theme's own faces are registered before its first frame, so a
        // family the document names shapes the panel.
        let base = self.library.root().join(theme);
        if let Ok(mut fonts) = self.fonts.lock() {
            for refusal in
                fonts.load_faces(&stored.document, |path| std::fs::read(base.join(path)).ok())
            {
                eprintln!("{refusal}");
            }
        }
        documents.insert(theme.to_owned(), stored.document.clone());
        Some(stored.document)
    }
}

/// Start the panel loop: one tick a second, and a retry pass every fifteen.
///
/// The retry pass is what brings a panel back after it was unplugged, held by
/// another program, or restarted by the operating system; the tick is what puts
/// frames on it. Both run off the async runtime, because a slow panel must never
/// hold a request still.
pub fn spawn(app: crate::http::Shared) {
    tokio::spawn(async move {
        let mut ticks = 0_u32;
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(1_000));
        loop {
            interval.tick().await;
            let manager = Arc::clone(&app.manager);
            let _ = tokio::task::spawn_blocking(move || manager.tick()).await;
            ticks = ticks.wrapping_add(1);
            if ticks == 1 || ticks.is_multiple_of(15) {
                let records: Vec<yunari_panels::manager::DisplayRecord> = app
                    .displays
                    .lock()
                    .map(|store| store.list().iter().map(manager_record).collect())
                    .unwrap_or_default();
                let manager = Arc::clone(&app.manager);
                let _ = tokio::task::spawn_blocking(move || manager.reconcile(&records)).await;
            }
        }
    });
}

/// The manager's view of a stored record.
pub fn manager_record(
    record: &crate::displays::DisplayRecord,
) -> yunari_panels::manager::DisplayRecord {
    yunari_panels::manager::DisplayRecord {
        id: record.id.clone(),
        label: record.label.clone(),
        panel: record.kind == crate::displays::DisplayKind::Panel,
        rotation: record.rotation,
        device: record.device.clone(),
        render_mode: record.render_mode,
    }
}

/// The bytes a device reads: JPEG or PNG of the fitted frame, or the raw RGBA
/// a panel that reorders its own pixels takes.
pub fn encode(raster: &Raster, format: StillFormat) -> Result<Vec<u8>, String> {
    if format == StillFormat::Rgba {
        return Ok(raster.rgba.clone());
    }
    let image = image::RgbaImage::from_raw(raster.width, raster.height, raster.rgba.clone())
        .ok_or_else(|| "The frame does not fill the panel.".to_owned())?;
    let mut bytes = Vec::new();
    if format == StillFormat::Png {
        image
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .map_err(|error| error.to_string())?;
        return Ok(bytes);
    }
    let rgb = image::DynamicImage::ImageRgba8(image).to_rgb8();
    let mut writer = std::io::Cursor::new(&mut bytes);
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut writer, JPEG_QUALITY)
        .encode_image(&rgb)
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::displays::DEFAULT_DISPLAY;
    use yunari_library::SaveRequest;
    use yunari_model::document::{
        Artboard, ArtboardShape, Background, ContentFit, Metadata, Node, NodeKind, StyleLiteral,
        StyleValue, Transform,
    };

    struct Fixture {
        dir: PathBuf,
    }

    fn fixture(name: &str) -> Fixture {
        let dir = std::env::temp_dir().join(format!(
            "yunari-panel-frames-{name}-{}-{}",
            std::process::id(),
            jiff::Timestamp::now().as_millisecond()
        ));
        std::fs::create_dir_all(&dir).ok();
        Fixture { dir }
    }

    impl Fixture {
        fn frames(&self) -> Arc<PanelFrames> {
            let registry = Arc::new(Registry::new());
            Arc::new(PanelFrames::new(
                Library::new(self.dir.join("themes")),
                registry,
                self.dir.clone(),
            ))
        }

        fn with_theme(&self) -> &Self {
            let library = Library::new(self.dir.join("themes"));
            let document = ThemeDocument {
                schema_version: yunari_model::document::SCHEMA_VERSION.to_owned(),
                id: "fixture".to_owned(),
                metadata: Metadata {
                    theme_language: Some("en".to_owned()),
                    ..Metadata::default()
                },
                artboard: Artboard {
                    width: 100.0,
                    height: 100.0,
                    content_fit: ContentFit::Contain,
                    shape: ArtboardShape::Rect,
                },
                background: Background::default(),
                palette: BTreeMap::new(),
                type_tokens: BTreeMap::new(),
                assets: Vec::new(),
                nodes: vec![Node {
                    id: "block".to_owned(),
                    name: Some("block".to_owned()),
                    kind: NodeKind::Rectangle { corner_radius: 0.0 },
                    transform: Transform {
                        x: 0.0,
                        y: 0.0,
                        width: 100.0,
                        height: 100.0,
                        ..Transform::default()
                    },
                    visible: true,
                    locked: false,
                    style: BTreeMap::from([(
                        "fill".to_owned(),
                        StyleValue::Literal(StyleLiteral {
                            value: serde_json::json!({"kind": "solid", "color": "#ff8800"}),
                        }),
                    )]),
                    glass: None,
                    children: Vec::new(),
                }],
                bindings: BTreeMap::new(),
                editor_metadata: BTreeMap::new(),
            };
            library
                .save(&SaveRequest {
                    id: "fixture",
                    document: &document,
                    assets: &BTreeMap::new(),
                    credits: None,
                    base: None,
                    overwrite: true,
                })
                .expect("the fixture theme saves");
            let mut assignment = crate::assignment::Assignment::load(&self.dir);
            assignment.set(DEFAULT_DISPLAY, Some("fixture"));
            self
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.dir).ok();
        }
    }

    fn record(id: &str) -> DisplayRecord {
        DisplayRecord {
            id: id.to_owned(),
            label: id.to_owned(),
            panel: true,
            rotation: None,
            device: Some(yunari_panels::manager::PanelDeviceId {
                vendor_id: 0x1e71,
                product_id: 0x3008,
                serial: String::new(),
                device_id: None,
            }),
            render_mode: None,
        }
    }

    #[test]
    fn an_assigned_theme_is_fitted_to_the_panels_own_glass() {
        let fixture = fixture("assigned");
        fixture.with_theme();
        let frames = fixture.frames();
        let png = frames
            .frame(&record(DEFAULT_DISPLAY), 320, 320, StillFormat::Png)
            .expect("a frame");
        let decoded = image::load_from_memory(&png).expect("a PNG");
        assert_eq!((decoded.width(), decoded.height()), (320, 320));
    }

    #[test]
    fn an_unassigned_display_shows_a_plate_not_an_error() {
        let fixture = fixture("empty");
        let frames = fixture.frames();
        let rgba = frames
            .frame(&record("unassigned"), 320, 320, StillFormat::Rgba)
            .expect("a plate");
        assert_eq!(rgba.len(), 320 * 320 * 4);
        assert!(rgba.iter().all(|byte| *byte == 0));
    }

    #[test]
    fn a_turzx_frame_is_the_panels_own_size_in_rgba() {
        let fixture = fixture("rgba");
        fixture.with_theme();
        let frames = fixture.frames();
        let rgba = frames
            .frame(&record(DEFAULT_DISPLAY), 800, 480, StillFormat::Rgba)
            .expect("a frame");
        assert_eq!(rgba.len(), 800 * 480 * 4);
    }

    #[test]
    fn a_turned_panel_fits_the_rotated_picture() {
        let fixture = fixture("turned");
        fixture.with_theme();
        let frames = fixture.frames();
        let mut turned = record(DEFAULT_DISPLAY);
        // A rectangle's glass takes quarter turns; the Kraken's round glass
        // takes any angle, so this pins the stricter rule.
        turned.device = Some(yunari_panels::manager::PanelDeviceId {
            vendor_id: 0x1cbe,
            product_id: 0xa092,
            serial: String::new(),
            device_id: None,
        });
        turned.rotation = Some(90);
        let png = frames
            .frame(&turned, 464, 1920, StillFormat::Png)
            .expect("a frame");
        let decoded = image::load_from_memory(&png).expect("a PNG");
        assert_eq!((decoded.width(), decoded.height()), (464, 1920));
        // A rotation the rectangle cannot take is refused rather than rounded.
        turned.rotation = Some(45);
        assert!(frames.frame(&turned, 464, 1920, StillFormat::Png).is_err());
    }

    #[test]
    fn a_poisoned_cache_is_a_failure_not_a_panic() {
        let _ = Mutex::new(());
        let fixture = fixture("poison");
        let frames = fixture.frames();
        // Nothing poisons the caches in this test; it pins that the frame path
        // returns a sentence rather than unwrapping a lock.
        assert!(
            frames
                .frame(&record("none"), 4, 4, StillFormat::Png)
                .is_ok()
        );
    }
}
