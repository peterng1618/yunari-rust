//! This PC's settings: what the host does, not what the document holds.

use std::path::{Path, PathBuf};

use gpui_kit::*;
use yunari_host::credentials::Credentials;
use yunari_host::displays::DisplayKind;
use yunari_host::hosting::Hosting;
use yunari_host::http::Shared;
use yunari_providers::weather::WeatherSettings;

use crate::ui::EditorWindow;

/// One monitor this PC has, and whether a display record follows it.
pub struct MonitorRow {
    pub uuid: String,
    pub label: String,
    pub shown: bool,
}

/// The switches, read once when the dialog opens.
pub struct SettingsDialog {
    pub settings: PathBuf,
    /// Whether phones on this network may reach the host.
    pub lan: bool,
    /// Whether the app starts with Windows.
    pub autostart: bool,
    pub monitors: Vec<MonitorRow>,
    /// Where this PC's weather comes from and the place it is for.
    pub weather: WeatherSettings,
    /// Whether the selected backend has a stored key; the key itself is never
    /// carried by the dialog.
    pub weather_key_set: bool,
    /// The credential ids the configured custom sources name, and whether each
    /// one is stored. Never the secrets.
    pub source_credentials: Vec<(String, bool)>,
    /// Which drive answers a theme's abstract disk slots, and the names beside
    /// that choice.
    pub devices: yunari_providers::devices::Assignment,
    /// The physical devices any provider could name, for the slot rows.
    pub options: Vec<yunari_providers::provider::Device>,
    /// The last result of flipping a switch, for the row under it.
    pub message: Option<String>,
}

impl SettingsDialog {
    pub fn new(
        settings: PathBuf,
        monitors: Vec<MonitorRow>,
        options: Vec<yunari_providers::provider::Device>,
    ) -> Self {
        let weather = read_weather(&settings);
        let weather_key_set = read_weather_key_set(&settings, &weather);
        let source_credentials = read_source_credentials(&settings);
        Self {
            lan: Hosting::load(&settings).lan,
            autostart: yunari_host::autostart::enabled(),
            monitors,
            weather,
            weather_key_set,
            source_credentials,
            devices: yunari_providers::devices::Assignment::load(&settings),
            options,
            settings,
            message: None,
        }
    }
}

/// Writes which drive answers a slot (`system-disk` or `data-disk`) and reports
/// the assignment as stored. `None` clears the slot.
pub fn set_device_slot(
    settings: &Path,
    slot: &str,
    device_id: Option<&str>,
) -> Result<yunari_providers::devices::Assignment, String> {
    let mut devices = yunari_providers::devices::Assignment::load(settings);
    let target = match slot {
        "system-disk" => &mut devices.system_disk,
        "data-disk" => &mut devices.data_disk,
        "gpu" => &mut devices.gpu,
        _ => return Err(format!("{slot} is not a device slot.")),
    };
    *target = device_id.map(str::to_owned);
    devices.save(settings)?;
    Ok(yunari_providers::devices::Assignment::load(settings))
}

/// The three backends this host reads, in the order the row cycles them.
pub const WEATHER_BACKENDS: [&str; 3] = ["open-meteo", "weatherapi", "openweathermap"];

/// The credential id a backend's key is stored under, `None` for the keyless
/// one.
pub fn weather_key_id(provider: &str) -> Option<&'static str> {
    match provider {
        "weatherapi" => Some("weatherapi"),
        "openweathermap" => Some("openweathermap"),
        _ => None,
    }
}

/// Whether the selected backend's key is stored on this account.
pub fn read_weather_key_set(settings: &Path, weather: &WeatherSettings) -> bool {
    weather_key_id(&weather.provider).is_some_and(|id| Credentials::at(settings).get(id).is_some())
}

pub fn weather_path(settings: &Path) -> PathBuf {
    settings.join("weather.json")
}

/// The stored settings, or the defaults when the file is absent or unreadable:
/// a wrong setting may not stop the host.
pub fn read_weather(settings: &Path) -> WeatherSettings {
    let stored: serde_json::Value = std::fs::read_to_string(weather_path(settings))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(serde_json::Value::Null);
    WeatherSettings::from_json(&stored)
}

/// Writes the settings where the host reads them on its next poll.
pub fn write_weather(settings: &Path, weather: &WeatherSettings) -> Result<(), String> {
    let text =
        serde_json::to_string_pretty(&weather.to_json()).map_err(|error| error.to_string())?;
    std::fs::create_dir_all(settings).map_err(|error| error.to_string())?;
    let temp = settings.join("weather.json.new");
    std::fs::write(
        &temp,
        format!(
            "{text}
"
        ),
    )
    .map_err(|error| error.to_string())?;
    std::fs::rename(&temp, weather_path(settings)).map_err(|error| {
        let _ = std::fs::remove_file(&temp);
        error.to_string()
    })
}

/// Stores a backend's key under the OS protector. An empty value forgets it.
pub fn set_weather_key(settings: &Path, provider: &str, secret: &str) -> Result<(), String> {
    let Some(id) = weather_key_id(provider) else {
        return Err("Open-Meteo needs no key.".to_owned());
    };
    Credentials::at(settings).set(id, secret)
}

/// The credential ids the configured custom sources name, and whether each is
/// stored. Sorted and deduplicated, so two sources may share one secret.
pub fn read_source_credentials(settings: &Path) -> Vec<(String, bool)> {
    let stored: serde_json::Value =
        std::fs::read_to_string(settings.join(yunari_providers::custom::sources::FILE))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or(serde_json::Value::Null);
    let (sources, _) = yunari_providers::custom::sources::parse(&stored);
    let mut ids: Vec<String> = sources
        .iter()
        .filter_map(|source| source.auth.credential_id.clone())
        .collect();
    ids.sort();
    ids.dedup();
    let store = Credentials::at(settings);
    ids.into_iter()
        .map(|id| {
            let set = store.get(&id).is_some();
            (id, set)
        })
        .collect()
}

/// Stores one credential under the OS protector. An empty value forgets it.
pub fn set_credential(settings: &Path, id: &str, secret: &str) -> Result<(), String> {
    Credentials::at(settings).set(id, secret)
}

/// The monitors GPUI sees, each marked when a local display record holds it.
pub fn monitor_rows(host: Option<&Shared>, cx: &App) -> Vec<MonitorRow> {
    let records = host
        .and_then(|host| host.displays.lock().ok().map(|store| store.list().to_vec()))
        .unwrap_or_default();
    cx.displays()
        .iter()
        .map(|display| {
            let uuid = display
                .uuid()
                .map(|uuid| uuid.to_string())
                .unwrap_or_default();
            let bounds = display.bounds();
            let label = format!(
                "Monitor {}×{}",
                f32::from(bounds.size.width).round() as i64,
                f32::from(bounds.size.height).round() as i64
            );
            let shown = !uuid.is_empty()
                && records.iter().any(|record| {
                    record.kind == DisplayKind::Local
                        && record.monitor.as_deref() == Some(uuid.as_str())
                });
            MonitorRow { uuid, label, shown }
        })
        .collect()
}

/// Writes the LAN choice where the host reads it on its next bind.
pub fn set_lan(settings: &Path, lan: bool) -> bool {
    let mut hosting = Hosting::load(settings);
    hosting.lan = lan;
    hosting.save(settings);
    Hosting::load(settings).lan
}

/// Writes the autostart entry; returns the state Windows now reports.
pub fn set_autostart(on: bool) -> Result<bool, String> {
    yunari_host::autostart::set(on)?;
    Ok(yunari_host::autostart::enabled())
}

/// One clickable settings row.
fn row(
    id: &'static str,
    label: String,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .w_full()
        .px_3()
        .py_2()
        .rounded_sm()
        .cursor_pointer()
        .text_color(rgb(0xe8e8ea))
        .bg(rgb(0x22222a))
        .child(label)
        .on_click(on_click)
        .into_any_element()
}

pub fn view(dialog: &SettingsDialog, cx: &mut Context<EditorWindow>) -> impl IntoElement {
    let lan = if dialog.lan {
        "On  ·  phones on this network can pair and play"
    } else {
        "Off  ·  this PC only"
    };
    let autostart = if dialog.autostart { "On" } else { "Off" };
    let rows = [
        ("settings-lan", format!("Phones on this network: {lan}")),
        (
            "settings-autostart",
            format!("Start with Windows: {autostart}"),
        ),
    ];
    let mut column = div().flex().flex_col().gap_1();
    for (id, label) in rows {
        column = column.child(
            div()
                .id(id)
                .w_full()
                .px_3()
                .py_2()
                .rounded_sm()
                .cursor_pointer()
                .text_color(rgb(0xe8e8ea))
                .bg(rgb(0x22222a))
                .child(label)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.flip_setting(id, cx);
                })),
        );
    }
    let provider = dialog.weather.provider.clone();
    let key_state = match weather_key_id(&provider) {
        None => "no key needed".to_owned(),
        Some(_) if dialog.weather_key_set => "key stored".to_owned(),
        Some(_) => "no key yet".to_owned(),
    };
    let place = if dialog.weather.label.is_empty() {
        "Windows location".to_owned()
    } else {
        format!(
            "{} ({}, {})",
            dialog.weather.label, dialog.weather.latitude, dialog.weather.longitude
        )
    };
    let windows = if dialog.weather.use_windows_location {
        "On"
    } else {
        "Off"
    };
    column = column
        .child(row(
            "settings-weather-backend",
            format!("Weather backend: {provider}  ·  {key_state}"),
            cx.listener(|this, _: &ClickEvent, _, cx| this.cycle_weather_backend(cx)),
        ))
        .child(row(
            "settings-weather-place",
            format!("Weather place: {place}"),
            cx.listener(|this, _: &ClickEvent, _, _| this.edit_weather_place()),
        ))
        .child(row(
            "settings-weather-location",
            format!("Windows location: {windows}"),
            cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_weather_location(cx)),
        ));
    if weather_key_id(&provider).is_some() {
        column = column.child(row(
            "settings-weather-key",
            format!("{provider} key: {key_state}  ·  click to enter"),
            cx.listener(|this, _: &ClickEvent, _, _| this.edit_weather_key()),
        ));
    }
    for (index, (slot, label)) in [
        ("gpu", "GPU"),
        ("system-disk", "System disk"),
        ("data-disk", "Data disk"),
    ]
    .into_iter()
    .enumerate()
    {
        let chosen = match slot {
            "gpu" => dialog.devices.gpu.as_deref(),
            "system-disk" => dialog.devices.system_disk.as_deref(),
            _ => dialog.devices.data_disk.as_deref(),
        };
        let state = match chosen {
            None if slot == "data-disk" => "not assigned".to_owned(),
            None if slot == "gpu" => "first card this PC reports".to_owned(),
            None => "first drive this PC reports".to_owned(),
            Some(id) => dialog
                .options
                .iter()
                .find(|device| device.id == id)
                .map(|device| dialog.devices.name_of(id, &device.name).to_owned())
                .unwrap_or_else(|| format!("{id} (not present)")),
        };
        column = column.child(
            div()
                .id(("settings-device-slot", index))
                .w_full()
                .px_3()
                .py_2()
                .rounded_sm()
                .cursor_pointer()
                .text_color(rgb(0xe8e8ea))
                .bg(rgb(0x22222a))
                .child(format!("{label}: {state}  ·  click to change"))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.cycle_device_slot(slot, cx);
                })),
        );
    }
    for (index, (id, set)) in dialog.source_credentials.iter().enumerate() {
        let state = if *set { "stored" } else { "not set" };
        let label = format!("Source credential {id}: {state}  ·  click to enter");
        let credential = id.clone();
        column = column.child(
            div()
                .id(("settings-source-credential", index))
                .w_full()
                .px_3()
                .py_2()
                .rounded_sm()
                .cursor_pointer()
                .text_color(rgb(0xe8e8ea))
                .bg(rgb(0x22222a))
                .child(label)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, _| {
                    this.edit_credential(credential.clone());
                })),
        );
    }
    for (index, monitor) in dialog.monitors.iter().enumerate() {
        let uuid = monitor.uuid.clone();
        let state = if monitor.shown {
            "showing the active theme"
        } else {
            "not used"
        };
        let label = format!("{}  ·  {state}", monitor.label);
        column = column.child(
            div()
                .id(("settings-monitor", index))
                .w_full()
                .px_3()
                .py_2()
                .rounded_sm()
                .cursor_pointer()
                .text_color(rgb(0xe8e8ea))
                .bg(rgb(0x22222a))
                .child(label)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.toggle_monitor(&uuid, cx);
                })),
        );
    }
    let mut footer = div()
        .flex()
        .flex_col()
        .gap_1()
        .px_3()
        .text_color(rgb(0x8a8a94));
    footer = footer.child(format!("Settings: {}", dialog.settings.display()));
    if let Some(message) = &dialog.message {
        footer = footer.child(message.clone());
    }
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .px_3()
                .pt_3()
                .text_color(rgb(0xe8e8ea))
                .child("This PC"),
        )
        .child(column.p_1())
        .child(footer)
        .child(
            div()
                .id("settings-cancel")
                .px_3()
                .py_2()
                .cursor_pointer()
                .text_color(rgb(0x8a8a94))
                .child("Close  ·  Esc")
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close_overlay(cx))),
        )
}

#[cfg(test)]
mod tests {
    // Named imports, not a glob: GPUI exports its own `test` attribute.
    use std::path::PathBuf;

    use yunari_host::hosting::Hosting;

    use super::{
        read_weather, read_weather_key_set, set_device_slot, set_lan, set_weather_key,
        weather_key_id, write_weather,
    };

    fn temp_settings(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yunari-settings-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_millis())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).ok();
        dir
    }

    #[test]
    fn the_lan_switch_lands_where_the_host_reads_it() {
        let settings = temp_settings("lan");
        assert!(!Hosting::load(&settings).lan);
        assert!(set_lan(&settings, true));
        assert!(Hosting::load(&settings).lan);
        assert!(!set_lan(&settings, false));
        std::fs::remove_dir_all(&settings).ok();
    }

    #[test]
    fn a_weather_place_and_backend_survive_a_write() {
        let settings = temp_settings("weather");
        let mut weather = read_weather(&settings);
        assert_eq!(weather.provider, "open-meteo");
        weather.provider = "weatherapi".to_owned();
        weather.label = "Berlin".to_owned();
        weather.latitude = 52.52;
        weather.longitude = 13.405;
        weather.use_windows_location = false;
        write_weather(&settings, &weather).expect("a written setting");
        assert_eq!(read_weather(&settings), weather);
        // A file this build refuses reads as the defaults, not a failed host.
        std::fs::write(super::weather_path(&settings), "not json").unwrap();
        assert_eq!(read_weather(&settings).provider, "open-meteo");
        std::fs::remove_dir_all(&settings).ok();
    }

    #[test]
    fn a_backend_key_goes_to_the_protected_store() {
        let settings = temp_settings("weather-key");
        let mut weather = read_weather(&settings);
        assert!(!read_weather_key_set(&settings, &weather));
        set_weather_key(&settings, "weatherapi", "secret").expect("a stored key");
        weather.provider = "weatherapi".to_owned();
        assert!(read_weather_key_set(&settings, &weather));
        // The keyless backend needs none, and the key itself never reaches the
        // dialog's own file.
        assert!(weather_key_id("open-meteo").is_none());
        set_weather_key(&settings, "weatherapi", "").expect("a forgotten key");
        assert!(!read_weather_key_set(&settings, &weather));
        std::fs::remove_dir_all(&settings).ok();
    }

    #[test]
    fn a_source_credential_is_listed_and_stored_under_its_id() {
        let settings = temp_settings("source-credential");
        std::fs::write(
            settings.join("sources.json"),
            serde_json::json!({
                "sources": [
                    {"id": "one", "url": "https://a.example/x",
                     "auth": {"kind": "bearer", "credentialId": "lab_key"},
                     "fields": [{"id": "x", "pointer": "", "type": "number"}]},
                    {"id": "two", "url": "https://b.example/x",
                     "auth": {"kind": "bearer", "credentialId": "lab_key"},
                     "fields": [{"id": "x", "pointer": "", "type": "number"}]}
                ]
            })
            .to_string(),
        )
        .unwrap();
        // Two sources, one secret: one row, not two.
        assert_eq!(
            super::read_source_credentials(&settings),
            vec![("lab_key".to_owned(), false)]
        );
        super::set_credential(&settings, "lab_key", "secret").expect("a stored credential");
        assert_eq!(
            super::read_source_credentials(&settings),
            vec![("lab_key".to_owned(), true)]
        );
        super::set_credential(&settings, "lab_key", "").expect("a forgotten credential");
        assert_eq!(
            super::read_source_credentials(&settings),
            vec![("lab_key".to_owned(), false)]
        );
        std::fs::remove_dir_all(&settings).ok();
    }

    #[test]
    fn a_device_slot_round_trips_and_refuses_an_unknown_slot() {
        let settings = temp_settings("device-slot");
        let devices =
            set_device_slot(&settings, "system-disk", Some("wd-blue")).expect("a saved choice");
        assert_eq!(devices.system_disk.as_deref(), Some("wd-blue"));
        assert_eq!(
            yunari_providers::devices::Assignment::load(&settings)
                .system_disk
                .as_deref(),
            Some("wd-blue")
        );
        // The next slot is untouched by the first.
        let devices =
            set_device_slot(&settings, "data-disk", Some("samsung-ssd")).expect("a saved choice");
        assert_eq!(devices.system_disk.as_deref(), Some("wd-blue"));
        assert_eq!(devices.data_disk.as_deref(), Some("samsung-ssd"));
        // Clearing a slot writes it as unassigned.
        let devices = set_device_slot(&settings, "data-disk", None).expect("a cleared slot");
        assert_eq!(devices.data_disk, None);
        assert!(
            set_device_slot(&settings, "gpu-cooler", None).is_err(),
            "an unknown slot"
        );
        std::fs::remove_dir_all(&settings).ok();
    }
}
