use regex::Regex;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};

pub struct IconLib {
    pub name: &'static str,
    pub fill_type: &'static str, // "none", "fill", "stroke"
    pub url_fn: fn(&str) -> String,
}

pub static ICON_LIBRARIES: &[IconLib] = &[
    IconLib {
        name: "ionicons",
        fill_type: "none",
        url_fn: |n| format!("https://unpkg.com/ionicons@7.4.0/dist/svg/{}.svg", n),
    },
    IconLib {
        name: "fluent",
        fill_type: "fill",
        url_fn: |n| {
            format!(
                "https://unpkg.com/@fluentui/svg-icons/icons/{}_24_filled.svg",
                n.replace('-', "_")
            )
        },
    },
    IconLib {
        name: "bootstrap",
        fill_type: "fill",
        url_fn: |n| {
            format!(
                "https://unpkg.com/bootstrap-icons@1.11.3/icons/{}-fill.svg",
                n
            )
        },
    },
    IconLib {
        name: "phosphor",
        fill_type: "fill",
        url_fn: |n| {
            format!(
                "https://unpkg.com/@phosphor-icons/core@latest/assets/fill/{}-fill.svg",
                n
            )
        },
    },
    IconLib {
        name: "tabler-filled",
        fill_type: "fill",
        url_fn: |n| {
            format!(
                "https://unpkg.com/@tabler/icons@3.24.0/icons/filled/{}.svg",
                n
            )
        },
    },
    IconLib {
        name: "tabler",
        fill_type: "stroke",
        url_fn: |n| {
            format!(
                "https://unpkg.com/@tabler/icons@3.24.0/icons/outline/{}.svg",
                n
            )
        },
    },
    IconLib {
        name: "lucide",
        fill_type: "stroke",
        url_fn: |n| format!("https://unpkg.com/lucide-static@0.577.0/icons/{}.svg", n),
    },
];

pub fn get_material_to_ionicons_map() -> HashMap<&'static str, &'static str> {
    let mut m = HashMap::new();
    m.insert("local_cafe", "cafe");
    m.insert("local_bar", "wine");
    m.insert("local_pizza", "pizza");
    m.insert("local_dining", "restaurant");
    m.insert("spa", "leaf");
    m.insert("grass", "leaf");
    m.insert("nature", "leaf");
    m.insert("park", "leaf");
    m.insert("eco", "leaf");
    m.insert("notifications", "notifications");
    m.insert("alarm", "alarm");
    m.insert("schedule", "time");
    m.insert("access_time", "time");
    m.insert("favorite", "heart");
    m.insert("thumb_up", "thumbs-up");
    m.insert("thumb_down", "thumbs-down");
    m.insert("visibility", "eye");
    m.insert("visibility_off", "eye-off");
    m.insert("arrow_back", "arrow-back");
    m.insert("arrow_forward", "arrow-forward");
    m.insert("arrow_upward", "arrow-up");
    m.insert("arrow_downward", "arrow-down");
    m.insert("chevron_left", "chevron-back");
    m.insert("chevron_right", "chevron-forward");
    m.insert("expand_more", "chevron-down");
    m.insert("expand_less", "chevron-up");
    m.insert("close", "close");
    m.insert("check", "checkmark");
    m.insert("check_circle", "checkmark-circle");
    m.insert("error", "close-circle");
    m.insert("add", "add");
    m.insert("remove", "remove");
    m.insert("edit", "create");
    m.insert("delete", "trash");
    m.insert("shopping_cart", "cart");
    m.insert("shopping_bag", "bag");
    m.insert("settings", "settings");
    m.insert("account_circle", "person-circle");
    m.insert("person", "person");
    m.insert("group", "people");
    m.insert("search", "search");
    m.insert("filter_list", "filter");
    m.insert("sort", "swap-vertical");
    m.insert("menu", "menu");
    m.insert("more_horiz", "ellipsis-horizontal");
    m.insert("more_vert", "ellipsis-vertical");
    m.insert("home", "home");
    m.insert("star", "star");
    m.insert("bookmark", "bookmark");
    m.insert("lock", "lock-closed");
    m.insert("lock_open", "lock-open");
    m.insert("email", "mail");
    m.insert("phone", "call");
    m.insert("chat", "chatbubble");
    m.insert("message", "chatbox");
    m.insert("share", "share");
    m.insert("download", "download");
    m.insert("upload", "cloud-upload");
    m.insert("play_arrow", "play");
    m.insert("pause", "pause");
    m.insert("stop", "stop");
    m.insert("skip_next", "play-skip-forward");
    m.insert("skip_previous", "play-skip-back");
    m.insert("volume_up", "volume-high");
    m.insert("volume_off", "volume-mute");
    m.insert("camera_alt", "camera");
    m.insert("photo", "image");
    m.insert("videocam", "videocam");
    m.insert("attach_file", "attach");
    m.insert("link", "link");
    m.insert("refresh", "refresh");
    m.insert("warning", "warning");
    m.insert("info", "information-circle");
    m.insert("help", "help-circle");
    m
}

#[derive(Clone)]
struct RawIcon {
    svg: Arc<str>,
    fill_type: &'static str,
}
type IconCache = Mutex<VecDeque<(String, RawIcon)>>;
const ICON_CACHE_ENTRIES: usize = 128;
const ICON_CACHE_ENTRY_BYTES: usize = 64 * 1024;
static ICON_CACHE: OnceLock<IconCache> = OnceLock::new();

fn remember_icon(cache: &IconCache, name: &str, icon: &RawIcon) {
    if name.len() > 256 || icon.svg.len() > ICON_CACHE_ENTRY_BYTES {
        return;
    }
    let mut entries = cache.lock().unwrap_or_else(|error| error.into_inner());
    entries.retain(|(key, _)| key != name);
    if entries.len() >= ICON_CACHE_ENTRIES {
        entries.pop_front();
    }
    entries.push_back((name.to_string(), icon.clone()));
}

pub async fn fetch_svg_icon(
    client: &reqwest::Client,
    icon_name: &str,
    size: f64,
    fill: &str,
) -> Result<String, String> {
    let cache = ICON_CACHE.get_or_init(|| Mutex::new(VecDeque::new()));
    let raw = load_icon(client, icon_name, ICON_LIBRARIES, cache).await?;
    Ok(render_icon(&raw.svg, raw.fill_type, size, fill))
}

async fn load_icon(
    client: &reqwest::Client,
    icon_name: &str,
    libraries: &[IconLib],
    cache: &IconCache,
) -> Result<RawIcon, String> {
    {
        let mut entries = cache.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(position) = entries.iter().position(|(name, _)| name == icon_name) {
            let entry = entries.remove(position).unwrap();
            let icon = entry.1.clone();
            entries.push_back(entry);
            return Ok(icon);
        }
    }
    for lib in libraries {
        let url = (lib.url_fn)(icon_name);
        if let Ok(res) = client.get(&url).send().await {
            if res.status().is_success() {
                if let Ok(text) = res.text().await {
                    if text.contains("<svg") {
                        let icon = RawIcon {
                            svg: Arc::from(text),
                            fill_type: lib.fill_type,
                        };
                        remember_icon(cache, icon_name, &icon);
                        return Ok(icon);
                    }
                }
            }
        }
    }
    let map = get_material_to_ionicons_map();
    let hint = if let Some(suggestion) = map.get(icon_name) {
        format!(" Did you mean \"{}\"? (Material Icons names like \"{}\" are not supported — use Ionicons names instead.)", suggestion, icon_name)
    } else if icon_name.contains('_') {
        format!(" Hint: snake_case names (e.g. \"{}\") are typical of Material Icons — Ionicons uses kebab-case (e.g. \"{}\"). See figma_docs {{ section: \"icons\" }}.", icon_name, icon_name.replace('_', "-"))
    } else {
        String::new()
    };

    let lib_names: Vec<&str> = libraries.iter().map(|l| l.name).collect();
    Err(format!(
        "Icon \"{}\" not found in any library (tried: {}).{}",
        icon_name,
        lib_names.join(", "),
        hint
    ))
}

fn render_icon(text: &str, fill_type: &str, size: f64, fill: &str) -> String {
    static PATTERNS: OnceLock<[Regex; 7]> = OnceLock::new();
    let [re_comment, re_class, re_curr_fill, re_curr_stroke, re_viewbox, re_stroke_width, re_svg] =
        PATTERNS.get_or_init(|| {
            [
                Regex::new(r"<!--[\s\S]*?-->").unwrap(),
                Regex::new(r#"class="[^"]*""#).unwrap(),
                Regex::new(r#"fill="currentColor""#).unwrap(),
                Regex::new(r#"stroke="currentColor""#).unwrap(),
                Regex::new(r#"viewBox="([^"]*)""#).unwrap(),
                Regex::new(r#"stroke-width="([^"]+)""#).unwrap(),
                Regex::new(r"<svg([^>]*)>").unwrap(),
            ]
        });
    let mut svg = re_comment.replace_all(text, "").to_string();
    svg = re_class.replace_all(&svg, "").to_string();
    svg = re_curr_fill
        .replace_all(&svg, format!(r#"fill="{}""#, fill))
        .to_string();
    svg = re_curr_stroke
        .replace_all(&svg, format!(r#"stroke="{}""#, fill))
        .to_string();

    if fill_type == "none" {
        if let Some(tag_end) = svg.find('>') {
            let tag_start = &svg[..tag_end];
            if !tag_start.contains(r#"fill=""#) {
                svg = re_svg
                    .replace(&svg, format!(r#"<svg$1 fill="{}">"#, fill))
                    .to_string();
            }
        }
    }

    if let Some(caps) = re_viewbox.captures(&svg) {
        if let Some(vb_val) = caps.get(1) {
            let parts: Vec<&str> = vb_val.as_str().split_whitespace().collect();
            if parts.len() == 4 {
                if let Ok(vb_w) = parts[2].parse::<f64>() {
                    if vb_w > 0.0 && (vb_w - size).abs() > 0.01 {
                        let scale = size / vb_w;
                        svg = re_stroke_width
                            .replace_all(&svg, |sw_caps: &regex::Captures| {
                                if let Ok(w) = sw_caps[1].parse::<f64>() {
                                    let norm = (w * scale).max(0.5);
                                    format!(r#"stroke-width="{:.2}""#, norm)
                                } else {
                                    sw_caps[0].to_string()
                                }
                            })
                            .to_string();
                    }
                }
            }
        }
    }
    svg
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn cached_raw_icon_reuses_download_without_reusing_color_or_size() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let app = axum::Router::new().route("/icon.svg", axum::routing::get(move || {
            let calls = counted.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                r#"<svg viewBox="0 0 24 24"><!--note--><path class="icon" fill="currentColor" stroke="currentColor" stroke-width="2"/></svg>"#
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let client = reqwest::Client::new();
        let cache = Mutex::new(VecDeque::new());
        let libraries = [IconLib {
            name: "test",
            fill_type: "none",
            url_fn: |name| format!("{name}/icon.svg"),
        }];
        let first = load_icon(&client, &base, &libraries, &cache).await.unwrap();
        let red = render_icon(&first.svg, first.fill_type, 24.0, "#ff0000");
        let second = load_icon(&client, &base, &libraries, &cache).await.unwrap();
        let blue = render_icon(&second.svg, second.fill_type, 48.0, "#0000ff");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(red.contains("#ff0000") && red.contains("stroke-width=\"2\""));
        assert!(blue.contains("#0000ff") && blue.contains("stroke-width=\"4.00\""));
        assert!(!blue.contains("#ff0000") && !blue.contains("class=") && !blue.contains("<!--"));
        let missing = format!("{base}/missing");
        assert!(load_icon(&client, &missing, &libraries, &cache)
            .await
            .is_err());
        assert_eq!(
            cache.lock().unwrap().len(),
            1,
            "failed downloads are not cached"
        );
        server.abort();
    }

    #[test]
    fn cache_evicts_old_entries_and_skips_oversized_icons() {
        let cache = Mutex::new(VecDeque::new());
        let raw = RawIcon {
            svg: Arc::from("<svg/>"),
            fill_type: "none",
        };
        for i in 0..=ICON_CACHE_ENTRIES {
            remember_icon(&cache, &i.to_string(), &raw);
        }
        assert_eq!(cache.lock().unwrap().len(), ICON_CACHE_ENTRIES);
        assert!(!cache.lock().unwrap().iter().any(|(key, _)| key == "0"));
        let huge = RawIcon {
            svg: Arc::from("x".repeat(ICON_CACHE_ENTRY_BYTES + 1)),
            fill_type: "none",
        };
        remember_icon(&cache, "oversized", &huge);
        assert!(!cache
            .lock()
            .unwrap()
            .iter()
            .any(|(key, _)| key == "oversized"));
    }
}
