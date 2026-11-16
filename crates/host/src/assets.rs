//! The asset finder's stock sources: colour and gradient catalogues, and the
//! local stores beside them.
//!
//! Every remote call is this host's: the browser and the agent hold no key,
//! and a catalogue is fetched once per process so a search never re-reaches
//! the network. A failed fetch is not remembered, so the next search retries.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use serde::Deserialize;
use serde_json::{Value, json};

/// One page of a finder grid, everywhere: a source that paged differently
/// would page the finder differently.
pub const PAGE_SIZE: usize = 40;

const TIMEOUT_MS: u64 = 15_000;

/// A remote stock source this build serves.
pub struct Stock {
    pub id: &'static str,
    pub label: &'static str,
    pub category: &'static str,
    /// The preset kind its rows yield, when they yield one.
    pub preset: Option<&'static str>,
    /// The credential id a search needs, when the source is keyed.
    pub requires: Option<&'static str>,
    /// "grid" answers a search; "font-picker" answers the font tab's shape.
    pub body: &'static str,
}

pub const STOCKS: &[Stock] = &[
    Stock {
        id: "fontawesome-free",
        label: "Font Awesome Free",
        category: "icon",
        preset: None,
        requires: None,
        body: "grid",
    },
    Stock {
        id: "lucide",
        label: "Lucide",
        category: "icon",
        preset: None,
        body: "grid",
        requires: None,
    },
    Stock {
        id: "lineicons",
        label: "Lineicons",
        category: "icon",
        preset: None,
        body: "grid",
        requires: None,
    },
    Stock {
        id: "webgradients",
        label: "WebGradients",
        category: "colour",
        preset: Some("gradient"),
        body: "grid",
        requires: None,
    },
    Stock {
        id: "uigradients",
        label: "UI Gradients",
        category: "colour",
        preset: Some("gradient"),
        body: "grid",
        requires: None,
    },
    Stock {
        id: "colorswoop",
        label: "ColorSwoop",
        category: "colour",
        preset: Some("palette"),
        body: "grid",
        requires: None,
    },
    Stock {
        id: "google-fonts",
        label: "Google Fonts",
        category: "font",
        preset: None,
        requires: None,
        body: "font-picker",
    },
    Stock {
        id: "curated-fonts",
        label: "Curated fonts",
        category: "font",
        preset: None,
        requires: None,
        body: "font-picker",
    },
    Stock {
        id: "unsplash",
        label: "Unsplash",
        category: "image",
        preset: None,
        body: "grid",
        requires: Some("unsplash"),
    },
    Stock {
        id: "colorhunt",
        label: "Color Hunt",
        category: "colour",
        preset: Some("palette"),
        body: "grid",
        requires: None,
    },
];

pub fn stock(id: &str) -> Option<&'static Stock> {
    STOCKS.iter().find(|stock| stock.id == id)
}

/// A result's own file: its bytes, the media type it is, and the name a
/// declaration gives it.
pub type AssetBytes = Result<(Vec<u8>, &'static str, String), String>;

/// One page of results, with how many pages the question has.
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub results: Vec<Value>,
    pub total_pages: u32,
}

#[derive(Debug, Clone, PartialEq)]
struct Gradient {
    id: String,
    name: String,
    angle: f64,
    stops: Vec<(f64, String)>,
}

#[derive(Debug, Clone, PartialEq)]
struct Palette {
    id: String,
    name: String,
    url: String,
    colours: Vec<String>,
    tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
enum Catalogue {
    Gradients(Vec<Gradient>),
    Palettes(Vec<Palette>),
}

fn cache() -> &'static Mutex<HashMap<String, Catalogue>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Catalogue>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The catalogue behind a source, fetched once. No request signal: one
/// cancelled search must not abort the load every other search waits on.
fn with_catalogue<R>(
    id: &str,
    fetch: impl FnOnce(&str) -> Result<String, String>,
    render: impl FnOnce(&Catalogue) -> R,
) -> Result<R, String> {
    let mut held = cache()
        .lock()
        .map_err(|_| "the catalogue lock is poisoned".to_owned())?;
    if !held.contains_key(id) {
        let catalogue = match id {
            "webgradients" => Catalogue::Gradients(webgradients_of(&fetch(WEBGRADIENTS_URL)?)),
            "uigradients" => Catalogue::Gradients(uigradients_of(&fetch(UIGRADIENTS_URL)?)),
            "colorswoop" => Catalogue::Palettes(colorswoop_of(&fetch(COLORSWOOP_URL)?)),
            other => return Err(format!("\"{other}\" is not a cached catalogue.")),
        };
        held.insert(id.to_owned(), catalogue);
    }
    Ok(render(held.get(id).expect("just inserted")))
}

const WEBGRADIENTS_URL: &str =
    "https://raw.githubusercontent.com/itmeo/webgradients/master/gradients.json";
const UIGRADIENTS_URL: &str =
    "https://raw.githubusercontent.com/ghosh/uiGradients/main/gradients.json";
const COLORSWOOP_URL: &str = "https://colorswoop.com/api/palettes.json";
const COLORHUNT_URL: &str = "https://colorhunt.co/php/feed.php";

/// Case- and accent-insensitive, so "cafe" finds "Café".
fn fold(value: &str) -> String {
    value
        .chars()
        .flat_map(char::to_lowercase)
        .map(|letter| match letter {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
            'ç' => 'c',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ñ' => 'n',
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            'ý' | 'ÿ' => 'y',
            other => other,
        })
        .collect()
}

fn hex_colour(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn text_of(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
}

fn page_of(page: u32) -> usize {
    usize::try_from(page.max(1)).unwrap_or(1)
}

fn slice<T>(matches: &[T], page: u32) -> &[T] {
    let offset = (page_of(page) - 1) * PAGE_SIZE;
    let start = offset.min(matches.len());
    &matches[start..(start + PAGE_SIZE).min(matches.len())]
}

fn total_pages(count: usize) -> u32 {
    u32::try_from(count.div_ceil(PAGE_SIZE))
        .unwrap_or(u32::MAX)
        .max(1)
}

fn webgradients_of(body: &str) -> Vec<Gradient> {
    let entries: Vec<Value> = serde_json::from_str(body).unwrap_or_default();
    entries.iter().filter_map(webgradient).collect()
}

fn webgradient(entry: &Value) -> Option<Gradient> {
    let name = text_of(entry.get("name"))?;
    let index = match entry.get("index")? {
        Value::String(index) if !index.is_empty() => index.clone(),
        Value::Number(index) => index.to_string(),
        _ => return None,
    };
    let deg = entry.get("deg")?.as_f64()?;
    let mut stops: Vec<(f64, String)> = Vec::new();
    for stop in entry.get("gradient")?.as_array()? {
        let Some(colour) = text_of(stop.get("color")) else {
            continue;
        };
        if !hex_colour(colour) {
            continue;
        }
        let Some(pos) = stop.get("pos").and_then(Value::as_f64) else {
            continue;
        };
        // The catalogue carries out-of-range percentages ("Slick Carbon" has
        // 150); the editor refuses an offset outside 0..1, so clamp first.
        stops.push((pos.clamp(0.0, 100.0) / 100.0, colour.to_owned()));
    }
    if stops.len() < 2 {
        return None;
    }
    // The catalogue's degrees are CSS's; the editor stores 0 for a
    // left-to-right gradient (CSS 90deg), so the two differ by a quarter turn.
    Some(Gradient {
        id: index,
        name: name.to_owned(),
        angle: (90.0 - deg).rem_euclid(360.0),
        stops,
    })
}

fn uigradients_of(body: &str) -> Vec<Gradient> {
    let entries: Vec<Value> = serde_json::from_str(body).unwrap_or_default();
    entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            let name = text_of(entry.get("name"))?;
            let colours: Vec<&str> = entry
                .get("colors")?
                .as_array()?
                .iter()
                .filter_map(Value::as_str)
                .filter(|colour| hex_colour(colour))
                .collect();
            if colours.len() < 2 {
                return None;
            }
            let last = (colours.len() - 1) as f64;
            let stops = colours
                .iter()
                .enumerate()
                .map(|(at, colour)| (at as f64 / last, (*colour).to_owned()))
                .collect();
            Some(Gradient {
                id: index.to_string(),
                name: name.to_owned(),
                // The site renders every entry `to right`.
                angle: 0.0,
                stops,
            })
        })
        .collect()
}

fn colorswoop_of(body: &str) -> Vec<Palette> {
    let held: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    held.get("palettes")
        .and_then(Value::as_array)
        .map(|entries| entries.iter().filter_map(colorswoop_palette).collect())
        .unwrap_or_default()
}

fn colorswoop_palette(entry: &Value) -> Option<Palette> {
    let id = text_of(entry.get("id"))?;
    let name = text_of(entry.get("name"))?;
    let url = text_of(entry.get("url"))?;
    let colours: Vec<String> = entry
        .get("colors")?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    if colours.len() < 2 || !colours.iter().all(|colour| hex_colour(colour)) {
        return None;
    }
    let tags = entry
        .get("tags")
        .and_then(Value::as_array)
        .map(|tags| {
            tags.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    Some(Palette {
        id: id.to_owned(),
        name: name.to_owned(),
        url: url.to_owned(),
        colours,
        tags,
    })
}

/// A Color Hunt palette is a 24-hex code, four concatenated `RRGGBB`.
fn colorhunt_of(body: &str) -> Result<Vec<Palette>, String> {
    let Value::Array(entries) =
        serde_json::from_str(body).map_err(|_| "Color Hunt feed is not a list".to_owned())?
    else {
        // The feed is undocumented; a changed shape is a failure, not an
        // empty page.
        return Err("Color Hunt feed is not a list".to_owned());
    };
    let mut palettes = Vec::new();
    for entry in &entries {
        let Some(code) = text_of(entry.get("code")) else {
            continue;
        };
        if code.len() != 24 || !code.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            continue;
        }
        let colours = code
            .as_bytes()
            .chunks(6)
            .map(|chunk| format!("#{}", String::from_utf8_lossy(chunk).to_uppercase()))
            .collect();
        palettes.push(Palette {
            id: code.to_owned(),
            name: format!("Color Hunt {}", code[..6].to_uppercase()),
            url: format!("https://colorhunt.co/palette/{code}"),
            colours,
            tags: text_of(entry.get("likes"))
                .map(|likes| vec![format!("{likes} likes")])
                .unwrap_or_default(),
        });
    }
    Ok(palettes)
}

fn gradient_result(gradient: &Gradient, licence: &str, source_url: &str) -> Value {
    json!({
        "id": gradient.id,
        "name": gradient.name,
        "kind": "gradient",
        "preview": "",
        "licence": licence,
        "sourceUrl": source_url,
        "preset": {
            "kind": "gradient",
            "angle": gradient.angle,
            "stops": gradient.stops.iter().map(|(offset, colour)| json!({
                "offset": offset,
                "color": colour,
            })).collect::<Vec<_>>(),
        },
    })
}

fn palette_result(palette: &Palette, licence: &str) -> Value {
    json!({
        "id": palette.id,
        "name": palette.name,
        "kind": "palette",
        "preview": "",
        "subtitle": if palette.tags.is_empty() { Value::Null } else {
            Value::String(palette.tags.join(", "))
        },
        "licence": licence,
        "sourceUrl": palette.url,
        "preset": {"kind": "palette", "colours": palette.colours},
    })
}

fn gradient_page(
    items: &[Gradient],
    query: &str,
    page: u32,
    licence: &str,
    source_url: &str,
) -> Page {
    let wanted = fold(query.trim());
    let matches: Vec<&Gradient> = if wanted.is_empty() {
        items.iter().collect()
    } else {
        items
            .iter()
            .filter(|gradient| fold(&gradient.name).contains(&wanted))
            .collect()
    };
    Page {
        results: slice(&matches, page)
            .iter()
            .map(|gradient| gradient_result(gradient, licence, source_url))
            .collect(),
        // No match is one empty page, not no pages.
        total_pages: total_pages(matches.len()),
    }
}

fn palette_page(items: &[Palette], query: &str, page: u32, licence: &str, by_tags: bool) -> Page {
    let wanted = fold(query.trim());
    let matches: Vec<&Palette> = if wanted.is_empty() {
        items.iter().collect()
    } else {
        items
            .iter()
            .filter(|palette| {
                fold(&palette.name).contains(&wanted)
                    || (by_tags && palette.tags.iter().any(|tag| fold(tag).contains(&wanted)))
            })
            .collect()
    };
    Page {
        results: slice(&matches, page)
            .iter()
            .map(|palette| palette_result(palette, licence))
            .collect(),
        total_pages: total_pages(matches.len()),
    }
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
struct LocalIcon {
    name: String,
    svg: String,
}

/// One grid page of a generated icon manifest.
const ICON_PAGE: usize = 50;

/// The manifest a source ships, parsed once.
fn icons(id: &str) -> Option<&'static Vec<LocalIcon>> {
    static LUCIDE: OnceLock<Vec<LocalIcon>> = OnceLock::new();
    static LINEICONS: OnceLock<Vec<LocalIcon>> = OnceLock::new();
    let (slot, text) = match id {
        "lucide" => (
            &LUCIDE,
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/lucide.json")),
        ),
        "lineicons" => (
            &LINEICONS,
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/lineicons.json")),
        ),
        _ => return None,
    };
    Some(slot.get_or_init(|| serde_json::from_str(text).unwrap_or_default()))
}

/// Spaces and hyphens are one separator, so "arrow down" finds `arrow-down`.
fn icon_fold(value: &str) -> String {
    let mut folded = String::with_capacity(value.len());
    let mut separator = false;
    for letter in value.chars().flat_map(char::to_lowercase) {
        if letter.is_whitespace() || letter == '-' {
            if !separator {
                folded.push('-');
                separator = true;
            }
        } else {
            folded.push(letter);
            separator = false;
        }
    }
    folded
}

fn icon_result(icon: &LocalIcon, licence: &str, home_url: &str) -> Value {
    use base64::Engine;
    // A data URL rather than a host round-trip: the icon is already here, and
    // a grid renders it offline. `<img>` runs no script in an SVG.
    let preview = format!(
        "data:image/svg+xml;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&icon.svg)
    );
    json!({
        "id": icon.name,
        "name": icon.name,
        "kind": "icon",
        "preview": preview,
        "licence": licence,
        "sourceUrl": format!("{home_url}{}", icon.name),
    })
}

fn icon_page(icons: &[LocalIcon], query: &str, page: u32, licence: &str, home_url: &str) -> Page {
    let wanted = icon_fold(query);
    let matches: Vec<&LocalIcon> = if wanted.is_empty() {
        icons.iter().collect()
    } else {
        icons
            .iter()
            .filter(|icon| icon_fold(&icon.name).contains(&wanted))
            .collect()
    };
    let offset = (page_of(page) - 1) * ICON_PAGE;
    let start = offset.min(matches.len());
    let page_items = &matches[start..(start + ICON_PAGE).min(matches.len())];
    Page {
        results: page_items
            .iter()
            .map(|icon| icon_result(icon, licence, home_url))
            .collect(),
        // A manifest with no match is one empty page, not no pages: the grid
        // says "no results" and the pager draws nothing.
        total_pages: u32::try_from(matches.len().div_ceil(ICON_PAGE))
            .unwrap_or(u32::MAX)
            .max(1),
    }
}

const FONTAWESOME_BASE: &str = "https://cdn.jsdelivr.net/npm/@fortawesome/fontawesome-free@6.7.2/";

#[derive(Debug, Clone)]
struct FaIcon {
    name: String,
    label: String,
    styles: Vec<String>,
}

#[derive(Debug, Clone)]
struct FaCatalogue {
    licence: String,
    icons: Vec<FaIcon>,
}

/// The free set's metadata, read once and only kept when it loaded whole.
fn fontawesome() -> Result<Arc<FaCatalogue>, String> {
    static HELD: OnceLock<Mutex<Option<Arc<FaCatalogue>>>> = OnceLock::new();
    let held = HELD.get_or_init(|| Mutex::new(None));
    let mut slot = held
        .lock()
        .map_err(|_| "the catalogue lock is poisoned".to_owned())?;
    if let Some(catalogue) = slot.as_ref() {
        return Ok(Arc::clone(catalogue));
    }
    let icons = yunari_providers::http::get(
        &format!("{FONTAWESOME_BASE}metadata/icon-families.json"),
        TIMEOUT_MS,
    )?;
    let package =
        yunari_providers::http::get(&format!("{FONTAWESOME_BASE}package.json"), TIMEOUT_MS)?;
    let package: Value = serde_json::from_str(&package).unwrap_or(Value::Null);
    let licence = package
        .get("license")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    let catalogue = Arc::new(FaCatalogue {
        licence,
        icons: fontawesome_of(&icons),
    });
    *slot = Some(Arc::clone(&catalogue));
    Ok(catalogue)
}

fn fontawesome_of(body: &str) -> Vec<FaIcon> {
    let Value::Object(entries) = serde_json::from_str(body).unwrap_or(Value::Null) else {
        return Vec::new();
    };
    let mut icons = Vec::new();
    for (name, entry) in &entries {
        let label = entry
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let Some(classic) = entry
            .get("svgs")
            .and_then(|svgs| svgs.get("classic"))
            .and_then(Value::as_object)
        else {
            continue;
        };
        let styles: Vec<String> = classic
            .keys()
            .filter(|style| {
                !style.is_empty() && style.bytes().all(|byte| byte.is_ascii_lowercase())
            })
            .cloned()
            .collect();
        if styles.is_empty() {
            continue;
        }
        let mut styles = styles;
        styles.sort();
        icons.push(FaIcon {
            name: name.clone(),
            label,
            styles,
        });
    }
    icons.sort_by(|left, right| left.name.cmp(&right.name));
    icons
}

fn fontawesome_page(catalogue: &FaCatalogue, query: &str, page: u32) -> Page {
    let strip = |value: &str| -> String {
        value
            .chars()
            .filter(|letter| !letter.is_whitespace() && *letter != '_' && *letter != '-')
            .flat_map(char::to_lowercase)
            .collect()
    };
    let wanted = strip(query);
    let mut matches = Vec::new();
    for icon in &catalogue.icons {
        if !wanted.is_empty()
            && !strip(&icon.name).contains(&wanted)
            && !strip(&icon.label).contains(&wanted)
        {
            continue;
        }
        for style in &icon.styles {
            matches.push((icon, style));
        }
    }
    let offset = (page_of(page) - 1) * ICON_PAGE;
    let start = offset.min(matches.len());
    let rows = &matches[start..(start + ICON_PAGE).min(matches.len())];
    Page {
        results: rows
            .iter()
            .map(|(icon, style)| {
                json!({
                    "id": format!("{style}:{}", icon.name),
                    "name": icon.name,
                    "kind": "icon",
                    "preview": format!("{FONTAWESOME_BASE}svgs/{style}/{}.svg", icon.name),
                    "subtitle": style,
                    "licence": catalogue.licence,
                    "sourceUrl": format!("https://fontawesome.com/icons/{}?f={style}", icon.name),
                })
            })
            .collect(),
        total_pages: u32::try_from(matches.len().div_ceil(ICON_PAGE))
            .unwrap_or(u32::MAX)
            .max(1),
    }
}

/// One Font Awesome result's file, fetched from the pinned package. Only an
/// id the catalogue itself names is ever addressed.
fn fontawesome_bytes(result: &str) -> Option<AssetBytes> {
    let (style, name) = result.split_once(':')?;
    if !style.bytes().all(|byte| byte.is_ascii_lowercase())
        || name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return None;
    }
    let Ok(catalogue) = fontawesome() else {
        return Some(Err("Font Awesome metadata is unavailable.".to_owned()));
    };
    let known = catalogue
        .icons
        .iter()
        .any(|icon| icon.name == name && icon.styles.iter().any(|held| held == style));
    if !known {
        return None;
    }
    Some(
        yunari_providers::http::get_bytes(
            &format!("{FONTAWESOME_BASE}svgs/{style}/{name}.svg"),
            TIMEOUT_MS,
            1024 * 1024,
        )
        .map(|bytes| (bytes, "image/svg+xml", format!("{name}.svg"))),
    )
}

const UNSPLASH_API: &str = "https://api.unsplash.com/";
/// The API's documented `per_page` ceiling; a larger ask is a 400.
const UNSPLASH_PER_PAGE: usize = 30;
const UNSPLASH_LICENCE: &str = "Unsplash License";

fn unsplash_url(path: &str, query: &[(&str, String)]) -> String {
    let mut url = format!("{UNSPLASH_API}{path}");
    let mut separator = '?';
    for (name, value) in query {
        url.push(separator);
        separator = '&';
        url.push_str(name);
        url.push('=');
        url.push_str(&yunari_providers::http::encode(value));
    }
    url
}

fn unsplash_text(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
}

fn unsplash_host(url: Option<&Value>) -> String {
    url.and_then(Value::as_str)
        .and_then(|url| url.split_once("://"))
        .and_then(|(_, rest)| rest.split('/').next())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default()
}

/// Unsplash+ carries one of three signs; any one drops it, so every row that
/// reaches the grid is under the Unsplash License.
fn unsplash_free(photo: &Value) -> bool {
    if photo.get("premium") == Some(&Value::Bool(true))
        || photo.get("plus") == Some(&Value::Bool(true))
    {
        return false;
    }
    let small = unsplash_host(photo.get("urls").and_then(|urls| urls.get("small")));
    let regular = unsplash_host(photo.get("urls").and_then(|urls| urls.get("regular")));
    small != "plus.unsplash.com" && regular != "plus.unsplash.com"
}

fn unsplash_result(photo: &Value) -> Option<Value> {
    let id = unsplash_text(photo.get("id"))?;
    let small = unsplash_text(photo.get("urls").and_then(|urls| urls.get("small")))?;
    let page = unsplash_text(photo.get("links").and_then(|links| links.get("html")))?;
    let name = unsplash_text(photo.get("alt_description"))
        .or_else(|| unsplash_text(photo.get("description")))
        .unwrap_or("Untitled");
    Some(json!({
        "id": id,
        "name": name,
        "kind": "image",
        "preview": small,
        "subtitle": unsplash_text(photo.get("user").and_then(|user| user.get("name"))),
        "licence": UNSPLASH_LICENCE,
        "sourceUrl": page,
    }))
}

/// The last page the Editorial feed's `Link` header names, or one more than
/// this page when it has a `next`, or the page itself when a short one ended
/// the feed.
fn unsplash_feed_pages(link: &str, page: u32, served: usize) -> u32 {
    let mut more = false;
    for entry in link.split(',') {
        let page_in = || -> Option<u32> {
            entry
                .split_once("page=")
                .and_then(|(_, rest)| rest.split(|letter: char| !letter.is_ascii_digit()).next())
                .and_then(|number| number.parse().ok())
        };
        if entry.contains("rel=\"last\"")
            && let Some(last) = page_in()
        {
            return last;
        }
        if entry.contains("rel=\"next\"") {
            more = true;
        }
    }
    if more || served >= UNSPLASH_PER_PAGE {
        page.saturating_add(1)
    } else {
        page
    }
}

fn unsplash_search(query: &str, page: u32, key: &str) -> Result<Page, String> {
    use yunari_providers::http::{Get, get_with_headers};
    let page = page.max(1);
    let request = |url: String| Get {
        url,
        headers: vec![("Authorization".to_owned(), format!("Client-ID {key}"))],
        timeout_ms: TIMEOUT_MS,
        public_only: false,
    };
    if query.trim().is_empty() {
        let url = unsplash_url(
            "photos",
            &[
                ("page", page.to_string()),
                ("per_page", UNSPLASH_PER_PAGE.to_string()),
            ],
        );
        let (body, headers) = get_with_headers(&request(url))?;
        let served: Vec<Value> = serde_json::from_str(&body).unwrap_or_default();
        let results: Vec<Value> = served
            .iter()
            .filter(|photo| unsplash_free(photo))
            .filter_map(unsplash_result)
            .collect();
        let link = headers
            .iter()
            .find(|(name, _)| name == "link")
            .map(|(_, value)| value.as_str())
            .unwrap_or_default();
        return Ok(Page {
            results,
            // A filtered page can be shorter than the one Unsplash served; the
            // page's own length, not the free results', proves another.
            total_pages: unsplash_feed_pages(link, page, served.len()),
        });
    }
    let url = unsplash_url(
        "search/photos",
        &[
            ("query", query.to_owned()),
            ("page", page.to_string()),
            ("per_page", UNSPLASH_PER_PAGE.to_string()),
        ],
    );
    let (body, _) = get_with_headers(&request(url))?;
    let answer: Value = serde_json::from_str(&body)
        .map_err(|_| "Unsplash's search answer is not JSON.".to_owned())?;
    let results: Vec<Value> = answer
        .get("results")
        .and_then(Value::as_array)
        .map(|photos| {
            photos
                .iter()
                .filter(|photo| unsplash_free(photo))
                .filter_map(unsplash_result)
                .collect()
        })
        .unwrap_or_default();
    let total_pages = answer
        .get("total_pages")
        .and_then(Value::as_u64)
        .and_then(|pages| u32::try_from(pages).ok())
        .unwrap_or(page);
    Ok(Page {
        results,
        total_pages,
    })
}

fn unsplash_bytes(result: &str, key: &str) -> Option<AssetBytes> {
    use yunari_providers::http::{Get, get_with};
    if result.is_empty()
        || !result
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return None;
    }
    let auth = || vec![("Authorization".to_owned(), format!("Client-ID {key}"))];
    let fetch = |url: &str| {
        get_with(&Get {
            url: url.to_owned(),
            headers: auth(),
            timeout_ms: TIMEOUT_MS,
            public_only: false,
        })
    };
    let details = fetch(&format!("{UNSPLASH_API}photos/{result}")).ok()?;
    let details: Value = serde_json::from_str(&details).ok()?;
    let trigger = details
        .get("links")
        .and_then(|links| links.get("download_location"))
        .and_then(Value::as_str)?;
    let full = details
        .get("urls")
        .and_then(|urls| urls.get("full"))
        .and_then(Value::as_str)?;
    let name = details
        .get("slug")
        .and_then(Value::as_str)
        .filter(|slug| {
            !slug.is_empty()
                && slug
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
        .unwrap_or(result);
    // The API guideline's download trigger, before the image itself.
    if fetch(trigger).is_err() {
        return Some(Err("Unsplash did not register the download.".to_owned()));
    }
    Some(
        yunari_providers::http::get_bytes(full, TIMEOUT_MS, 64 * 1024 * 1024)
            .map(|bytes| (bytes, "image/jpeg", format!("{name}.jpg"))),
    )
}

const FONTSOURCE: &str = "https://api.fontsource.org/v1/fonts";
const FONTSOURCE_CDN: &str = "https://cdn.jsdelivr.net/fontsource/fonts";
/// One page of family results; the registry holds about two thousand.
const FONT_PAGE: usize = 24;

fn plain_segment(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn npm_version(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes.next().is_some_and(|byte| byte.is_ascii_digit())
        && bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'+' | b'-')
        })
}

/// One cut's id names the pinned artifact path, so the bytes can be rebuilt
/// without holding the registry open between two calls.
fn font_cut_id(id: &str, version: &str, subset: &str, weight: u32, style: &str) -> String {
    format!("cut:{id}@{version}/{subset}-{weight}-{style}")
}

fn font_cut_url(cut: &str) -> Option<String> {
    let rest = cut.strip_prefix("cut:")?;
    let (id, rest) = rest.split_once('@')?;
    let (version, file) = rest.split_once('/')?;
    if !plain_segment(id) || !npm_version(version) {
        return None;
    }
    let (subset, weight, style) = split_cut_file(file)?;
    Some(format!(
        "{FONTSOURCE_CDN}/{id}@{version}/{subset}-{weight}-{style}.woff2"
    ))
}

/// `subset-weight-style`, read from the right: a subset may carry hyphens.
fn split_cut_file(file: &str) -> Option<(&str, &str, &str)> {
    let (head, style) = file.rsplit_once('-')?;
    if style != "normal" && style != "italic" {
        return None;
    }
    let (subset, weight) = head.rsplit_once('-')?;
    if !plain_segment(subset) || weight.is_empty() || !weight.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((subset, weight, style))
}

/// A family's published cuts, rebuilt from validated keys so an upstream
/// `@latest` or a foreign origin never reaches the wire.
fn fontsource_cuts(detail: &Value) -> Vec<Value> {
    let id = detail.get("id").and_then(Value::as_str).unwrap_or_default();
    let family = detail
        .get("family")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let version = detail
        .get("npmVersion")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let licence = detail
        .get("license")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    if !plain_segment(id) || !npm_version(version) {
        return Vec::new();
    }
    let Some(variants) = detail.get("variants").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut cuts = Vec::new();
    for (weight_key, styles) in variants {
        let Ok(weight) = weight_key.parse::<u32>() else {
            continue;
        };
        if weight == 0 {
            continue;
        }
        let Some(subsets) = styles.get("normal").and_then(Value::as_object) else {
            continue;
        };
        for (subset, face) in subsets {
            if !plain_segment(subset) {
                continue;
            }
            let Some(url) = face
                .get("url")
                .and_then(|url| url.get("woff2"))
                .and_then(Value::as_str)
            else {
                continue;
            };
            // The row's id advertises subset/weight/style and the fetch uses
            // the path those keys name; a published path that disagrees would
            // serve a different cut, so the pairing is not offered at all.
            let file = url.rsplit('/').next().unwrap_or_default();
            if file != format!("{subset}-{weight}-normal.woff2") {
                continue;
            }
            let cut = font_cut_id(id, version, subset, weight, "normal");
            let pinned = format!("{FONTSOURCE_CDN}/{id}@{version}/{subset}-{weight}-normal.woff2");
            cuts.push(json!({
                "id": cut,
                "source": "google-fonts",
                "family": family,
                "weight": weight,
                "style": "normal",
                "subset": subset,
                "format": "woff2",
                "previewUrl": pinned,
                "licence": licence,
                "sourceUrl": pinned,
            }));
        }
    }
    cuts
}

/// A family row is addressed by its first cut, preferring the default subset
/// at weight 400 so the row's own bytes are the face authors expect.
fn fontsource_result(detail: &Value) -> Option<Value> {
    let family = detail.get("family").and_then(Value::as_str)?;
    let category = detail
        .get("category")
        .and_then(Value::as_str)
        .unwrap_or("font");
    let default_subset = detail
        .get("defSubset")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let licence = detail
        .get("license")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let cuts = fontsource_cuts(detail);
    let lead = cuts
        .iter()
        .find(|cut| cut["subset"] == default_subset && cut["weight"] == 400)
        .or_else(|| cuts.iter().find(|cut| cut["weight"] == 400))
        .or_else(|| cuts.iter().find(|cut| cut["subset"] == default_subset))
        .or_else(|| cuts.first())?;
    Some(json!({
        "id": lead["id"],
        "name": family,
        "kind": "font",
        "preview": "",
        "subtitle": format!("{category} · {} cuts", cuts.len()),
        "licence": licence,
        "sourceUrl": lead["sourceUrl"],
        "font": {"cuts": cuts},
    }))
}

fn fontsource_search(query: &str, page: u32) -> Result<Page, String> {
    let body = yunari_providers::http::get(&format!("{FONTSOURCE}?type=google"), TIMEOUT_MS)?;
    let Value::Array(entries) = serde_json::from_str(&body)
        .map_err(|_| "Fontsource's registry is not a list.".to_owned())?
    else {
        return Err("Fontsource's registry is not a list.".to_owned());
    };
    let wanted = icon_fold(query.trim());
    let matches: Vec<(String, String)> = entries
        .iter()
        .filter(|entry| entry.get("type").and_then(Value::as_str) == Some("google"))
        .filter_map(|entry| {
            let id = entry.get("id").and_then(Value::as_str)?;
            let family = entry.get("family").and_then(Value::as_str)?;
            Some((id.to_owned(), family.to_owned()))
        })
        .filter(|(_, family)| wanted.is_empty() || icon_fold(family).contains(&wanted))
        .collect();
    let start = (page_of(page) - 1) * FONT_PAGE;
    let asked = &matches[start.min(matches.len())..(start + FONT_PAGE).min(matches.len())];
    let mut results = Vec::new();
    for (id, _) in asked {
        // One unread or malformed family is one missing row, not a failed
        // page: the others this page asked for still reach the picker.
        let Ok(body) = yunari_providers::http::get(
            &format!("{FONTSOURCE}/{}", yunari_providers::http::encode(id)),
            TIMEOUT_MS,
        ) else {
            continue;
        };
        let Ok(detail) = serde_json::from_str::<Value>(&body) else {
            continue;
        };
        if let Some(result) = fontsource_result(&detail) {
            results.push(result);
        }
    }
    Ok(Page {
        results,
        total_pages: u32::try_from(matches.len().div_ceil(FONT_PAGE))
            .unwrap_or(u32::MAX)
            .max(1),
    })
}

fn fontsource_bytes(result: &str) -> Option<AssetBytes> {
    let url = font_cut_url(result)?;
    let name = result.trim_start_matches("cut:").replace(['@', '/'], "-");
    Some(
        yunari_providers::http::get_bytes(&url, TIMEOUT_MS, 16 * 1024 * 1024)
            .map(|bytes| (bytes, "font/woff2", format!("{name}.woff2"))),
    )
}

const CURATED_SOURCE: &str = "curated-fonts";

#[derive(Debug, Deserialize)]
struct CuratedLicence {
    name: String,
}

#[derive(Debug, Deserialize)]
struct CuratedFace {
    id: String,
    family: String,
    weight: u32,
    style: String,
    format: String,
    subset: String,
    #[serde(rename = "sourceUrl")]
    source_url: String,
    license: CuratedLicence,
}

#[derive(Debug, Deserialize)]
struct CuratedTrioFace {
    #[serde(flatten)]
    face: CuratedFace,
    role: String,
}

#[derive(Debug, Deserialize)]
struct CuratedTrio {
    id: String,
    name: String,
    description: String,
    faces: Vec<CuratedTrioFace>,
}

#[derive(Debug, Deserialize)]
struct CuratedCatalogue {
    faces: Vec<CuratedFace>,
    trios: Vec<CuratedTrio>,
}

/// The catalogue ships inside this build: browsing costs no CDN call, and the
/// bytes of a face are fetched only when an author applies it.
fn curated() -> &'static CuratedCatalogue {
    static HELD: OnceLock<CuratedCatalogue> = OnceLock::new();
    HELD.get_or_init(|| {
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/curated-fonts.json"
        )))
        .expect("the curated catalogue is built in and valid")
    })
}

/// Spaces and hyphens are one separator, so "fira code" finds "Fira Code".
fn curated_fold(value: &str) -> String {
    let mut folded = String::with_capacity(value.len());
    let mut separator = false;
    for letter in value.chars() {
        if letter.is_whitespace() || letter == '-' {
            separator = true;
            continue;
        }
        if separator && !folded.is_empty() {
            folded.push('-');
        }
        separator = false;
        folded.extend(letter.to_lowercase());
    }
    folded
}

fn curated_cut(face: &CuratedFace) -> Value {
    json!({
        "id": format!("cut:{}", face.id),
        "source": CURATED_SOURCE,
        "family": face.family,
        "weight": face.weight,
        "style": face.style,
        "subset": face.subset,
        "format": face.format,
        "previewUrl": face.source_url,
        "licence": face.license.name,
        "sourceUrl": face.source_url,
    })
}

/// A family row is one cut per distinct family, addressed by the family's
/// first face.
fn curated_family_rows(catalogue: &CuratedCatalogue) -> Vec<Value> {
    let mut families: Vec<(&str, Vec<&CuratedFace>)> = Vec::new();
    for face in &catalogue.faces {
        match families.iter_mut().find(|(name, _)| *name == face.family) {
            Some((_, faces)) => faces.push(face),
            None => families.push((face.family.as_str(), vec![face])),
        }
    }
    families
        .into_iter()
        .filter_map(|(name, faces)| {
            let lead = *faces.first()?;
            Some(json!({
                "id": format!("cut:{}", lead.id),
                "name": name,
                "kind": "font",
                "preview": "",
                "subtitle": format!("{} cuts", faces.len()),
                "licence": lead.license.name,
                "sourceUrl": lead.source_url,
                "font": {"cuts": faces.iter().map(|face| curated_cut(face)).collect::<Vec<_>>()},
            }))
        })
        .collect()
}

fn curated_trio_row(trio: &CuratedTrio) -> Option<Value> {
    let heading = trio.faces.first()?;
    let mut roles = serde_json::Map::new();
    for face in &trio.faces {
        roles.insert(face.role.clone(), curated_cut(&face.face));
    }
    Some(json!({
        "id": format!("trio:{}", trio.id),
        "name": trio.name,
        "kind": "font",
        "preview": "",
        "subtitle": trio.description,
        "licence": heading.face.license.name,
        "sourceUrl": heading.face.source_url,
        "font": {
            "cuts": trio.faces.iter().map(|face| curated_cut(&face.face)).collect::<Vec<_>>(),
            "trio": {
                "id": trio.id,
                "name": trio.name,
                "description": trio.description,
                "roles": roles,
            },
        },
    }))
}

/// Trios first: a pairing is the authored decision, a single family the
/// fallback. The catalogue is small enough to answer one page at once.
fn curated_page(query: &str) -> Page {
    let catalogue = curated();
    let wanted = curated_fold(query.trim());
    let matches = |value: &str| wanted.is_empty() || curated_fold(value).contains(&wanted);
    let mut results: Vec<Value> = catalogue
        .trios
        .iter()
        .filter(|trio| {
            matches(&trio.name)
                || matches(&trio.description)
                || trio.faces.iter().any(|face| matches(&face.face.family))
        })
        .filter_map(curated_trio_row)
        .collect();
    results.extend(
        curated_family_rows(catalogue)
            .into_iter()
            .filter(|row| row["name"].as_str().is_some_and(matches)),
    );
    Page {
        results,
        total_pages: 1,
    }
}

/// A family row is addressed by its lead cut; a trio row by its heading cut.
fn curated_face_id(result: &str) -> Option<String> {
    let catalogue = curated();
    if let Some(id) = result.strip_prefix("cut:") {
        return catalogue
            .faces
            .iter()
            .find(|face| face.id == id)
            .map(|face| face.id.clone());
    }
    let id = result.strip_prefix("trio:")?;
    catalogue
        .trios
        .iter()
        .find(|trio| trio.id == id)
        .and_then(|trio| trio.faces.first())
        .map(|face| face.face.id.clone())
}

fn curated_bytes(result: &str) -> Option<AssetBytes> {
    let id = curated_face_id(result)?;
    let face = curated().faces.iter().find(|face| face.id == id)?;
    Some(
        yunari_providers::http::get_bytes(&face.source_url, TIMEOUT_MS, 16 * 1024 * 1024)
            .map(|bytes| (bytes, "font/woff2", format!("{}.woff2", face.id))),
    )
}

/// Searches one stock source with the host's own HTTP client.
pub fn search(id: &str, query: &str, page: u32) -> Result<Page, String> {
    search_with(id, query, page, None)
}

/// Searches one stock source with the credential it names, when it names one.
pub fn search_with(
    id: &str,
    query: &str,
    page: u32,
    credential: Option<&str>,
) -> Result<Page, String> {
    if let Some(stock) = stock(id)
        && stock.requires.is_some()
        && credential.is_none()
    {
        return Err(format!("{} needs an access key.", stock.label));
    }
    if id == "unsplash" {
        let key = credential.ok_or_else(|| "Unsplash needs an access key.".to_owned())?;
        return unsplash_search(query, page, key);
    }
    if id == "google-fonts" {
        return fontsource_search(query, page);
    }
    if id == "curated-fonts" {
        return Ok(curated_page(query));
    }
    if id == "fontawesome-free" {
        let catalogue = fontawesome()?;
        return Ok(fontawesome_page(&catalogue, query, page));
    }
    if let Some(icons) = icons(id) {
        return match id {
            "lucide" => Ok(icon_page(
                icons,
                query,
                page,
                "ISC",
                "https://lucide.dev/icons/",
            )),
            _ => Ok(icon_page(
                icons,
                query,
                page,
                "MIT",
                "https://lineicons.com/icons/",
            )),
        };
    }
    let get = |url: &str| yunari_providers::http::get(url, TIMEOUT_MS);
    match id {
        "webgradients" => with_catalogue(id, get, |catalogue| {
            let Catalogue::Gradients(items) = catalogue else {
                return palette_page(&[], query, page, "MIT", false);
            };
            gradient_page(items, query, page, "MIT", "https://webgradients.com/")
        }),
        "uigradients" => with_catalogue(id, get, |catalogue| {
            let Catalogue::Gradients(items) = catalogue else {
                return palette_page(&[], query, page, "MIT", false);
            };
            gradient_page(
                items,
                query,
                page,
                "MIT",
                "https://github.com/ghosh/uiGradients",
            )
        }),
        "colorswoop" => with_catalogue(id, get, |catalogue| {
            let Catalogue::Palettes(items) = catalogue else {
                return gradient_page(&[], query, page, "CC0-1.0", "");
            };
            palette_page(items, query, page, "CC0-1.0", true)
        }),
        "colorhunt" => {
            let form = [
                ("step", (page_of(page) - 1).to_string()),
                ("sort", "new".to_owned()),
                ("tags", query.trim().to_owned()),
                ("timeframe", String::new()),
            ];
            let body = yunari_providers::http::post_form(COLORHUNT_URL, TIMEOUT_MS, &form)?;
            let palettes = colorhunt_of(&body)?;
            // The feed answers a step, never a catalogue: an empty page proves
            // the end is behind, a short page is the end, and a full one says
            // only that the next page exists.
            let results: Vec<Value> = palettes
                .iter()
                .map(|palette| palette_result(palette, "unknown"))
                .collect();
            let total_pages = match palettes.len() {
                0 => page.saturating_sub(1).max(1),
                count if count < PAGE_SIZE => page,
                _ => page.saturating_add(1),
            };
            Ok(Page {
                results,
                total_pages,
            })
        }
        other => Err(format!("\"{other}\" is not an asset source.")),
    }
}

/// The bytes behind one result: an icon's own SVG or a photo, and nothing
/// when the row is a preset rather than a file.
pub fn bytes(id: &str, result: &str) -> Option<AssetBytes> {
    bytes_with(id, result, None)
}

/// The bytes behind one result, with the credential a keyed source needs.
pub fn bytes_with(id: &str, result: &str, credential: Option<&str>) -> Option<AssetBytes> {
    if id == "unsplash" {
        let Some(key) = credential else {
            return Some(Err("Unsplash needs an access key.".to_owned()));
        };
        return unsplash_bytes(result, key);
    }
    if id == "google-fonts" {
        return fontsource_bytes(result);
    }
    if id == "curated-fonts" {
        return curated_bytes(result);
    }
    if id == "fontawesome-free" {
        return fontawesome_bytes(result);
    }
    let icons = icons(id)?;
    let icon = icons.iter().find(|icon| icon.name == result)?;
    Some(Ok((
        icon.svg.as_bytes().to_vec(),
        "image/svg+xml",
        format!("{result}.svg"),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_icon_manifest_answers_by_name_and_gives_up_its_own_svg() {
        let lucide = icons("lucide").expect("the Lucide manifest");
        assert!(lucide.len() > 1000, "{} icons", lucide.len());
        let page = icon_page(lucide, "arrow down", 1, "ISC", "https://lucide.dev/icons/");
        assert!(
            page.results.iter().any(|row| row["name"] == "arrow-down"),
            "{} rows",
            page.results.len()
        );
        let (svg, mime, name) = bytes("lucide", "arrow-down")
            .expect("the icon's bytes")
            .expect("the icon reads");
        assert_eq!(mime, "image/svg+xml");
        assert_eq!(name, "arrow-down.svg");
        assert!(String::from_utf8(svg).unwrap().contains("<svg"));
        assert!(bytes("lucide", "not-an-icon").is_none());
        assert!(icons("lineicons").is_some_and(|icons| !icons.is_empty()));
    }

    #[test]
    fn unsplash_drops_every_plus_sign_and_reads_its_feed_pages() {
        let free = serde_json::json!({
            "id": "abc_1",
            "alt_description": "A harbour",
            "user": {"name": "A photographer"},
            "links": {"html": "https://unsplash.com/photos/abc_1"},
            "urls": {"small": "https://images.unsplash.com/small.jpg",
                     "regular": "https://images.unsplash.com/regular.jpg"},
        });
        assert!(unsplash_free(&free));
        let row = unsplash_result(&free).expect("a row");
        assert_eq!(row["name"], "A harbour");
        assert_eq!(row["subtitle"], "A photographer");
        for paid in [
            serde_json::json!({"plus": true, "urls": {}}),
            serde_json::json!({"premium": true, "urls": {}}),
            serde_json::json!({"urls": {"small": "https://plus.unsplash.com/x.jpg"}}),
        ] {
            assert!(!unsplash_free(&paid), "{paid}");
        }
        // A named last page wins; a next page means one more; a short feed
        // ends where it is.
        let last = r#"<https://api.unsplash.com/photos?page=2>; rel="next", <https://api.unsplash.com/photos?page=9>; rel="last""#;
        assert_eq!(unsplash_feed_pages(last, 2, 30), 9);
        let next = r#"<https://api.unsplash.com/photos?page=3>; rel="next""#;
        assert_eq!(unsplash_feed_pages(next, 2, 4), 3);
        assert_eq!(unsplash_feed_pages("", 2, 4), 2);
    }

    #[test]
    fn a_google_family_offers_only_cuts_whose_published_path_agrees() {
        let detail = serde_json::json!({
            "id": "roboto",
            "family": "Roboto",
            "defSubset": "latin",
            "category": "sans-serif",
            "license": "OFL-1.1",
            "npmVersion": "5.0.0",
            "variants": {
                "400": {"normal": {
                    "latin": {"url": {"woff2": "https://cdn.jsdelivr.net/fontsource/fonts/roboto@5.0.0/latin-400-normal.woff2"}},
                    "cyrillic": {"url": {"woff2": "https://cdn.jsdelivr.net/fontsource/fonts/roboto@5.0.0/cyrillic-400-normal.woff2"}},
                }},
                "700": {"normal": {
                    // A path that disagrees with its own keys is not offered.
                    "latin": {"url": {"woff2": "https://cdn.jsdelivr.net/fontsource/fonts/roboto@5.0.0/latin-500-normal.woff2"}},
                }},
            },
        });
        let result = fontsource_result(&detail).expect("a family row");
        assert_eq!(result["id"], "cut:roboto@5.0.0/latin-400-normal");
        assert_eq!(result["name"], "Roboto");
        assert_eq!(result["subtitle"], "sans-serif · 2 cuts");
        assert_eq!(result["licence"], "OFL-1.1");
        let url = font_cut_url("cut:roboto@5.0.0/latin-400-normal").expect("a URL");
        assert_eq!(
            url,
            "https://cdn.jsdelivr.net/fontsource/fonts/roboto@5.0.0/latin-400-normal.woff2"
        );
        assert!(font_cut_url("cut:roboto@latest/latin-400-normal").is_none());
        assert!(font_cut_url("cut:roboto@5.0.0/latin-400-medium").is_none());
        assert!(font_cut_url("https://example.com/x.woff2").is_none());
    }

    #[test]
    fn a_keyed_source_without_its_key_says_which_key_it_needs() {
        let error = search_with("unsplash", "harbour", 1, None).unwrap_err();
        assert!(error.contains("access key"), "{error}");
        assert!(
            STOCKS
                .iter()
                .any(|stock| stock.id == "unsplash" && stock.requires == Some("unsplash"))
        );
    }

    #[test]
    fn a_font_awesome_entry_offers_only_the_styles_the_free_package_ships() {
        let body = r#"{
            "house": {"label":"Home","svgs":{"classic":{"solid":{},"regular":{},"brands":{}}}},
            "brand": {"label":"Brand","svgs":{"classic":{"brands":{}}}},
            "empty": {"label":"Empty","svgs":{"classic":{}}},
            "no svgs": {"label":"None"}
        }"#;
        let icons = fontawesome_of(body);
        assert_eq!(icons.len(), 2, "{icons:?}");
        assert_eq!(icons[0].name, "brand");
        assert_eq!(icons[0].styles, ["brands"]);
        let catalogue = FaCatalogue {
            licence: "CC-BY-4.0".to_owned(),
            icons,
        };
        let page = fontawesome_page(&catalogue, "home", 1);
        assert_eq!(page.results.len(), 3, "{:?}", page.results);
        assert_eq!(page.results[0]["id"], "brands:house");
        assert_eq!(page.results[0]["licence"], "CC-BY-4.0");
        assert!(
            fontawesome_page(&catalogue, "nothing", 1)
                .results
                .is_empty()
        );
    }

    #[test]
    fn a_webgradient_is_its_own_name_angle_and_stops() {
        let body = r##"[
            {"name":"Slick Carbon","index":"001","deg":180,"gradient":[
                {"pos":0,"color":"#000000"},{"pos":150,"color":"#3a3a3a"}]},
            {"name":"Broken","index":"002","deg":45,"gradient":[
                {"pos":0,"color":"#000000"}]},
            {"name":"No stops","index":"003","deg":45,"gradient":[]}
        ]"##;
        let items = webgradients_of(body);
        assert_eq!(items.len(), 1, "{items:?}");
        assert_eq!(items[0].name, "Slick Carbon");
        // CSS 180deg is the editor's 270; the 150% stop is clamped to 1.
        assert!((items[0].angle - 270.0).abs() < 1e-9);
        assert_eq!(items[0].stops[1].0, 1.0);
    }

    #[test]
    fn a_catalogue_search_folds_accents_and_pages_at_its_own_size() {
        let items: Vec<Gradient> = (0..PAGE_SIZE + 1)
            .map(|at| Gradient {
                id: at.to_string(),
                name: if at == 0 {
                    "Café".to_owned()
                } else {
                    format!("Other {at}")
                },
                angle: 0.0,
                stops: vec![(0.0, "#000000".to_owned()), (1.0, "#ffffff".to_owned())],
            })
            .collect();
        let found = gradient_page(&items, "cafe", 1, "MIT", "https://example.com/");
        assert_eq!(found.results.len(), 1);
        assert_eq!(found.results[0]["name"], "Café");
        let all = gradient_page(&items, "", 1, "MIT", "https://example.com/");
        assert_eq!(all.results.len(), PAGE_SIZE);
        assert_eq!(all.total_pages, 2);
        let second = gradient_page(&items, "", 2, "MIT", "https://example.com/");
        assert_eq!(second.results.len(), 1);
    }

    #[test]
    fn a_palette_must_be_its_own_colours_and_a_short_page_ends_the_feed() {
        let body = r##"{"palettes":[
            {"id":"1","name":"Dusk","url":"https://colorswoop.com/p/1",
             "colors":["#112233","#445566"],"tags":["calm","night"]},
            {"id":"2","name":"Broken","url":"https://colorswoop.com/p/2",
             "colors":["#112233","sand"]}
        ]}"##;
        assert_eq!(colorswoop_of(body).len(), 1);
        let feed = r#"[{"code":"112233445566778899aabbcc","likes":"4"},
                       {"code":"not a code"}]"#;
        let palettes = colorhunt_of(feed).unwrap();
        assert_eq!(palettes.len(), 1);
        assert_eq!(
            palettes[0].colours,
            ["#112233", "#445566", "#778899", "#AABBCC"]
        );
        assert!(colorhunt_of("{}").is_err());
    }

    #[test]
    fn a_palette_search_reads_its_tags_and_no_match_is_one_empty_page() {
        let items = vec![Palette {
            id: "1".to_owned(),
            name: "Dusk".to_owned(),
            url: String::new(),
            colours: vec!["#112233".to_owned(), "#445566".to_owned()],
            tags: vec!["night".to_owned()],
        }];
        assert_eq!(
            palette_page(&items, "night", 1, "CC0-1.0", true)
                .results
                .len(),
            1
        );
        assert_eq!(
            palette_page(&items, "night", 1, "CC0-1.0", false)
                .results
                .len(),
            0
        );
        let none = palette_page(&items, "dawn", 3, "CC0-1.0", true);
        assert!(none.results.is_empty());
        assert_eq!(none.total_pages, 1);
    }

    #[test]
    fn the_curated_catalogue_ships_with_its_faces_and_trios() {
        let catalogue = curated();
        assert!(
            catalogue.faces.len() > 900,
            "{} faces",
            catalogue.faces.len()
        );
        assert!(
            catalogue.trios.len() > 300,
            "{} trios",
            catalogue.trios.len()
        );
        assert!(
            catalogue
                .faces
                .iter()
                .all(|face| face.source_url.starts_with("https://cdn.jsdelivr.net/"))
        );
    }

    #[test]
    fn a_curated_search_offers_trios_before_families_and_folds_its_separators() {
        let page = curated_page("");
        assert_eq!(page.total_pages, 1);
        assert!(page.results.len() > 400, "{} rows", page.results.len());
        let trio = &page.results[0];
        assert!(trio["id"].as_str().unwrap().starts_with("trio:"));
        assert_eq!(trio["kind"], "font");
        assert_eq!(
            trio["font"]["trio"]["roles"]["heading"]["source"],
            CURATED_SOURCE
        );
        assert!(page.results.iter().any(|row| {
            let id = row["id"].as_str().unwrap_or_default();
            id.starts_with("cut:") && id.contains("fira-code")
        }));

        let folded = curated_page("fira code");
        assert!(
            folded.results.iter().any(|row| row["name"] == "Fira Code"),
            "{} rows",
            folded.results.len()
        );
        // A family row carries every cut of that one family.
        let family = folded
            .results
            .iter()
            .find(|row| row["id"].as_str().is_some_and(|id| id.starts_with("cut:")))
            .expect("a family row");
        let cuts = family["font"]["cuts"].as_array().expect("cuts");
        assert!(cuts.iter().all(|cut| cut["family"] == family["name"]));
        assert_eq!(family["subtitle"], format!("{} cuts", cuts.len()));
    }

    #[test]
    fn a_curated_row_resolves_to_a_face_and_nothing_else_does() {
        let catalogue = curated();
        let lead = catalogue
            .faces
            .iter()
            .find(|face| face.id == "abril-fatface-400")
            .expect("a known face");
        assert_eq!(
            curated_face_id(&format!("cut:{}", lead.id)).as_deref(),
            Some("abril-fatface-400")
        );
        let trio = &catalogue.trios[0];
        assert_eq!(
            curated_face_id(&format!("trio:{}", trio.id)).as_deref(),
            Some(trio.faces[0].face.id.as_str())
        );
        assert!(curated_face_id("cut:not-a-face").is_none());
        assert!(curated_face_id("trio:not-a-trio").is_none());
        assert!(curated_face_id("abril-fatface-400").is_none());
    }
}
