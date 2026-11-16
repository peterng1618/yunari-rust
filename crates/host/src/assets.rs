//! The asset finder's stock sources: colour and gradient catalogues, and the
//! local stores beside them.
//!
//! Every remote call is this host's: the browser and the agent hold no key,
//! and a catalogue is fetched once per process so a search never re-reaches
//! the network. A failed fetch is not remembered, so the next search retries.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

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
}

pub const STOCKS: &[Stock] = &[
    Stock {
        id: "fontawesome-free",
        label: "Font Awesome Free",
        category: "icon",
        preset: None,
        requires: None,
    },
    Stock {
        id: "lucide",
        label: "Lucide",
        category: "icon",
        preset: None,
        requires: None,
    },
    Stock {
        id: "lineicons",
        label: "Lineicons",
        category: "icon",
        preset: None,
        requires: None,
    },
    Stock {
        id: "webgradients",
        label: "WebGradients",
        category: "colour",
        preset: Some("gradient"),
        requires: None,
    },
    Stock {
        id: "uigradients",
        label: "UI Gradients",
        category: "colour",
        preset: Some("gradient"),
        requires: None,
    },
    Stock {
        id: "colorswoop",
        label: "ColorSwoop",
        category: "colour",
        preset: Some("palette"),
        requires: None,
    },
    Stock {
        id: "unsplash",
        label: "Unsplash",
        category: "image",
        preset: None,
        requires: Some("unsplash"),
    },
    Stock {
        id: "colorhunt",
        label: "Color Hunt",
        category: "colour",
        preset: Some("palette"),
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
}
