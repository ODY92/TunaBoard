use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashSet,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};
use url::Url;

const MAX_AUDIO: u64 = 20 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sound {
    pub id: String,
    pub name: String,
    pub author: String,
    pub url: String,
    pub audio: String,
    pub duration_ms: u64,
    #[serde(default)]
    pub image: String,
}

pub fn valid_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}

pub fn allowed_url(raw: &str) -> bool {
    Url::parse(raw).is_ok_and(|u| {
        u.scheme() == "https"
            && u.port_or_known_default() == Some(443)
            && u.username().is_empty()
            && u.password().is_none()
            && u.host_str()
                .is_some_and(|h| h == "tuna.voicemod.net" || h.ends_with(".voicemod.net"))
    })
}

fn text(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(160)
        .collect()
}

fn collect(v: &Value, depth: usize, out: &mut Vec<Sound>, seen: &mut HashSet<String>) {
    if depth > 8 {
        return;
    }
    match v {
        Value::Object(o) => {
            if o.get("type").and_then(Value::as_str) == Some("sound") {
                let id = text(v, "id");
                let audio = text(&v["audio"], "url");
                let duration_ms = v["duration"].as_u64().unwrap_or(0);
                if valid_id(&id)
                    && allowed_url(&audio)
                    && duration_ms <= 480_000
                    && v["locked"] != true
                    && v["publicStatus"] == "published"
                    && seen.insert(id.clone())
                {
                    out.push(Sound {
                        url: format!("https://tuna.voicemod.net/sound/{id}"),
                        id,
                        name: text(v, "name"),
                        author: text(&v["publicInfo"], "ownerName"),
                        audio,
                        duration_ms,
                        image: text(&v["icon"], "url"),
                    });
                }
                return;
            }
            for x in o.values() {
                collect(x, depth + 1, out, seen);
            }
        }
        Value::Array(a) => {
            for x in a {
                collect(x, depth + 1, out, seen);
            }
        }
        Value::String(s)
            if s.contains("SoundItem") || s.starts_with('{') || s.starts_with("{\\\"") =>
        {
            if let Ok(decoded) = serde_json::from_str::<Value>(s) {
                collect(&decoded, depth + 1, out, seen);
            } else if let Ok(decoded) = serde_json::from_str::<String>(&format!("\"{s}\"")) {
                if let Ok(decoded) = serde_json::from_str::<Value>(&decoded) {
                    collect(&decoded, depth + 1, out, seen);
                }
            }
        }
        _ => (),
    }
}

pub fn parse_page(html: &str) -> Result<Vec<Sound>> {
    let doc = Html::parse_document(html);
    let selector = Selector::parse("script#__NUXT_DATA__").unwrap();
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for el in doc.select(&selector) {
        let v: Value =
            serde_json::from_str(&el.inner_html()).context("La page Tuna a changé de format")?;
        collect(&v, 0, &mut out, &mut seen);
    }
    // Les pages individuelles fournissent aussi leurs métadonnées publiques en JSON-LD.
    if out.is_empty() {
        for el in doc.select(&Selector::parse("script[type='application/ld+json']").unwrap()) {
            let Ok(v) = serde_json::from_str::<Value>(&el.inner_html()) else {
                continue;
            };
            if let Some(graph) = v["@graph"].as_array() {
                for x in graph {
                    if x["@type"] != "AudioObject" {
                        continue;
                    }
                    let url = text(x, "url");
                    let audio = text(x, "contentUrl");
                    let id = url
                        .trim_end_matches('/')
                        .rsplit('/')
                        .next()
                        .unwrap_or_default()
                        .to_string();
                    if valid_id(&id) && allowed_url(&audio) && seen.insert(id.clone()) {
                        out.push(Sound {
                            id,
                            name: text(x, "name"),
                            author: text(&x["author"], "name"),
                            url,
                            audio,
                            duration_ms: 0,
                            image: graph
                                .iter()
                                .find(|n| n["@type"] == "ImageObject")
                                .map(|n| text(n, "url"))
                                .unwrap_or_default(),
                        });
                    }
                }
            }
        }
    }
    if out.is_empty() && !html.contains("__NUXT_DATA__") {
        bail!("Tuna ne fournit pas ses sons sur cette page. Ouvrez-la dans le navigateur pour vérifier.");
    }
    Ok(out)
}

pub fn client() -> Result<Client> {
    Ok(Client::builder()
        .user_agent("TunaBoard/0.2 (Linux; public community soundboard)")
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(35))
        .redirect(reqwest::redirect::Policy::custom(|a| {
            if a.previous().len() >= 5 || !allowed_url(a.url().as_str()) {
                a.stop()
            } else {
                a.follow()
            }
        }))
        .build()?)
}

pub fn page_url(query: &str, category: Option<&str>) -> Result<String> {
    let mut u = Url::parse("https://tuna.voicemod.net/")?;
    let mut path = u.path_segments_mut().unwrap();
    path.clear();
    if query.trim().starts_with("https://") {
        let raw = Url::parse(query.trim())?;
        if raw.host_str() != Some("tuna.voicemod.net")
            || !raw.path().starts_with("/sound/")
            || !allowed_url(raw.as_str())
        {
            bail!("Collez un lien https://tuna.voicemod.net/sound/…");
        }
        return Ok(raw.to_string());
    }
    if let Some(category) = category {
        path.extend(["sounds", "tag", category, ""]);
    } else if query.trim().is_empty() {
        path.extend(["sounds", ""]);
    } else {
        let slug = query
            .trim()
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join("-");
        path.extend(["search", "sounds", &slug, ""]);
    }
    drop(path);
    Ok(u.to_string())
}

pub fn search(query: &str, category: Option<&str>) -> Result<Vec<Sound>> {
    let u = page_url(query, category)?;
    let r = client()?.get(u).send()?.error_for_status()?;
    let mut html = String::new();
    r.take(3 * 1024 * 1024).read_to_string(&mut html)?;
    parse_page(&html)
}

pub fn cached_path(cache: &Path, sound: &Sound) -> Result<PathBuf> {
    if !valid_id(&sound.id) {
        bail!("Identifiant de son incorrect");
    }
    Ok(cache.join(format!("{}.wav", sound.id)))
}

pub fn download(sound: &Sound, cache: &Path) -> Result<PathBuf> {
    let dest = cached_path(cache, sound)?;
    if dest.is_file() && dest.metadata()?.len() > 44 {
        return Ok(dest);
    }
    // Les sons transférés restent disponibles même si le cache WAV est vidé.
    let imported = crate::store::data_dir()
        .join("imported")
        .join(format!("{}.audio", sound.id));
    if !imported.is_file() && !allowed_url(&sound.audio) {
        bail!("Adresse audio extérieure à Voicemod refusée");
    }
    std::fs::create_dir_all(cache)?;
    let mut bytes = Vec::new();
    if imported.is_file() {
        std::fs::File::open(imported)?
            .take(MAX_AUDIO + 1)
            .read_to_end(&mut bytes)?;
    } else {
        let r = client()?.get(&sound.audio).send()?.error_for_status()?;
        if r.content_length().is_some_and(|n| n > MAX_AUDIO) {
            bail!("Ce son dépasse 20 Mo");
        }
        r.take(MAX_AUDIO + 1).read_to_end(&mut bytes)?;
    }
    if bytes.len() as u64 > MAX_AUDIO {
        bail!("Ce son dépasse 20 Mo");
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let tmp = cache.join(format!("{}-{nonce}.audio", sound.id));
    let wav = cache.join(format!("{}-{nonce}.wav", sound.id));
    std::fs::write(&tmp, bytes)?;
    let result = Command::new("ffmpeg")
        .args(["-nostdin", "-v", "error", "-y", "-i"])
        .arg(&tmp)
        .args([
            "-vn",
            "-t",
            "480",
            "-ar",
            "48000",
            "-ac",
            "2",
            "-c:a",
            "pcm_s16le",
        ])
        .arg(&wav)
        .stdin(Stdio::null())
        .output();
    let _ = std::fs::remove_file(tmp);
    let output = result.context("ffmpeg est nécessaire pour préparer les sons")?;
    if !output.status.success() {
        let _ = std::fs::remove_file(&wav);
        bail!(
            "Audio illisible : {}",
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(240)
                .collect::<String>()
        );
    }
    std::fs::rename(wav, &dest)?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_sound_only_and_deduplicated() {
        let sound = serde_json::json!({"__key":"SoundItem","id":"86a2d033-dfae-4c87-9d0c-d68f11993dea","type":"sound","locked":false,"publicStatus":"published","name":"Vine boom","duration":1000,"audio":{"url":"https://us-tuna-sounds-files.voicemod.net/clip.mp3"},"publicInfo":{"ownerName":"Creator"}});
        let voice = serde_json::json!({"type":"voice","id":"115cbe95-e71a-459a-8f9d-b098c028751d"});
        let escaped = serde_json::to_string(&sound).unwrap().replace('"', "\\\"");
        let data = serde_json::json!([escaped, sound, voice]);
        let page = format!("<script id='__NUXT_DATA__' type='application/json'>{data}</script>");
        let list = parse_page(&page).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Vine boom");
    }
    #[test]
    fn urls_and_cache_cannot_escape() {
        assert!(!allowed_url(
            "http://us-tuna-sounds-files.voicemod.net/test.mp3"
        ));
        assert!(!allowed_url("https://voicemod.net.evil.test/test.mp3"));
        assert!(!allowed_url("https://127.0.0.1/test.mp3"));
        assert!(!valid_id("../../../../etc/passwd"));
        assert!(page_url("https://example.org/sound/hello", None).is_err());
        assert_eq!(
            page_url("vine boom", None).unwrap(),
            "https://tuna.voicemod.net/search/sounds/vine-boom/"
        );
    }
}
