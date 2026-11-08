use crate::{
    catalog::{self, Sound},
    store,
};
use anyhow::{bail, Context, Result};
use gtk::{
    gdk_pixbuf::{InterpType, Pixbuf, PixbufLoader},
    prelude::*,
};
use std::{io::Read, path::PathBuf};

pub const WIDTH: i32 = 192;
pub const HEIGHT: i32 = 192;
const MAX_DOWNLOAD: u64 = 1024 * 1024;
const MAX_CACHE: u64 = 30 * 1024 * 1024;

pub fn cached(id: &str) -> Result<PathBuf> {
    if !catalog::valid_id(id) {
        bail!("Identifiant de pochette incorrect");
    }
    Ok(store::cache_dir()
        .join("thumbnails-square")
        .join(format!("{id}.png")))
}

fn scaled(bytes: &[u8]) -> Result<Pixbuf> {
    if bytes.len() as u64 > MAX_DOWNLOAD {
        bail!("Image trop grande");
    }
    let loader = PixbufLoader::new();
    // Redimensionner dès la lecture plutôt que conserver une image en pleine résolution.
    loader.connect_size_prepared(|l, w, h| {
        if w > 0 && h > 0 {
            let factor = (WIDTH as f64 / w as f64).max(HEIGHT as f64 / h as f64);
            l.set_size(
                (w as f64 * factor).round().clamp(1.0, 2048.0) as i32,
                (h as f64 * factor).round().clamp(1.0, 2048.0) as i32,
            );
        }
    });
    loader.write(bytes)?;
    loader.close()?;
    let p = loader.pixbuf().context("Pochette illisible")?;
    let factor = (WIDTH as f64 / p.width() as f64).max(HEIGHT as f64 / p.height() as f64);
    let w = (p.width() as f64 * factor).ceil() as i32;
    let h = (p.height() as f64 * factor).ceil() as i32;
    // Éviter les panoramas ou images verticales extrêmes qui gonfleraient la mémoire.
    if w > 2048 || h > 2048 {
        bail!("Format d’image extrême");
    }
    let p = p
        .scale_simple(w.max(WIDTH), h.max(HEIGHT), InterpType::Bilinear)
        .context("Pochette illisible")?;
    let cropped = Pixbuf::new(p.colorspace(), p.has_alpha(), 8, WIDTH, HEIGHT)
        .context("Impossible de recadrer la pochette")?;
    p.copy_area(
        (p.width() - WIDTH) / 2,
        (p.height() - HEIGHT) / 2,
        WIDTH,
        HEIGHT,
        &cropped,
        0,
        0,
    );
    Ok(cropped)
}

pub fn fetch(sound: &Sound) -> Result<PathBuf> {
    let path = cached(&sound.id)?;
    if path.is_file() {
        return Ok(path);
    }
    // Anciennes bibliothèques : retrouver une seule fois la pochette depuis la page publique.
    let image = if sound.image.is_empty() {
        if sound.url.is_empty() {
            bail!("Ce son importé n’a pas de pochette");
        }
        catalog::search(&sound.url, None)?
            .into_iter()
            .find(|s| s.id == sound.id)
            .map(|s| s.image)
            .unwrap_or_default()
    } else {
        sound.image.clone()
    };
    if !catalog::allowed_url(&image) {
        bail!("Pochette indisponible");
    }
    let response = catalog::client()?.get(image).send()?.error_for_status()?;
    if response.content_length().is_some_and(|n| n > MAX_DOWNLOAD) {
        bail!("Pochette trop grande");
    }
    let mut bytes = Vec::new();
    response.take(MAX_DOWNLOAD + 1).read_to_end(&mut bytes)?;
    let pixbuf = scaled(&bytes)?;
    let dir = path.parent().unwrap();
    std::fs::create_dir_all(dir)?;
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    pixbuf.savev(&tmp, "png", &[("compression", "6")])?;
    std::fs::rename(tmp, &path)?;
    trim_cache(dir, &path);
    Ok(path)
}

fn trim_cache(dir: &std::path::Path, keep: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries
        .flatten()
        .filter_map(|e| {
            if e.path().extension()?.to_str()? != "png" {
                return None;
            }
            let m = e.metadata().ok()?;
            Some((e.path(), m.len(), m.modified().ok()?))
        })
        .collect();
    let mut total: u64 = entries.iter().map(|e| e.1).sum();
    entries.sort_by_key(|e| e.2);
    for (p, size, _) in entries {
        if total <= MAX_CACHE {
            break;
        }
        if p != keep && std::fs::remove_file(p).is_ok() {
            total = total.saturating_sub(size);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn thumbnails_are_small_and_bounded_for_landscape_and_portrait() {
        for (w, h) in [(1280, 720), (500, 900), (640, 640)] {
            let p = Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, false, 8, w, h).unwrap();
            p.fill(0x223344ff);
            let bytes = p.save_to_bufferv("png", &[]).unwrap();
            let small = scaled(&bytes).unwrap();
            assert_eq!((small.width(), small.height()), (WIDTH, HEIGHT));
            assert!(small.byte_length() <= (WIDTH * HEIGHT * 4) as usize);
        }
        assert!(scaled(&vec![0; MAX_DOWNLOAD as usize + 1]).is_err());
        assert!(cached("../../escape").is_err());
    }
}
