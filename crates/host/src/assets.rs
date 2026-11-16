//! The asset finder's stock sources: colour and gradient catalogues, and the
//! local stores beside them.
//!
//! Every remote call is this host's: the browser and the agent hold no key,
//! and a catalogue is fetched once per process so a search never re-reaches
//! the network. A failed fetch is not remembered, so the next search retries.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

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

/// Searches one stock source with the host's own HTTP client.
pub fn search(id: &str, query: &str, page: u32) -> Result<Page, String> {
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

/// The bytes behind one result: an icon's own SVG, or nothing when the row is
/// a preset rather than a file.
pub fn bytes(id: &str, result: &str) -> Option<(Vec<u8>, &'static str, String)> {
    let icons = icons(id)?;
    let icon = icons.iter().find(|icon| icon.name == result)?;
    Some((
        icon.svg.as_bytes().to_vec(),
        "image/svg+xml",
        format!("{result}.svg"),
    ))
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
        let (svg, mime, name) = bytes("lucide", "arrow-down").expect("the icon's bytes");
        assert_eq!(mime, "image/svg+xml");
        assert_eq!(name, "arrow-down.svg");
        assert!(String::from_utf8(svg).unwrap().contains("<svg"));
        assert!(bytes("lucide", "not-an-icon").is_none());
        assert!(icons("lineicons").is_some_and(|icons| !icons.is_empty()));
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
