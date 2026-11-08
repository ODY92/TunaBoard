mod hotkeys;

use anyhow::{bail, Context, Result};
use gtk::{gdk::prelude::GdkContextExt, gio, glib, prelude::*};
use std::{
    cell::RefCell,
    collections::HashSet,
    os::unix::{fs::PermissionsExt, net::UnixDatagram},
    path::PathBuf,
    rc::Rc,
    sync::mpsc::{self, Sender},
    time::Duration,
};
use tuna_board::{
    audio::{self, Audio},
    catalog::{self, Sound},
    store::{self, Library},
    thumbnails,
};

type AppState = Rc<RefCell<State>>;
enum Event {
    Results(u64, Result<Vec<Sound>>),
    Ready(u64, Sound, Result<PathBuf>, bool),
    Saved(Sound, Result<PathBuf>),
    Thumbnail(String, Result<PathBuf>),
}
struct ThumbnailSlot {
    sound: Sound,
    area: gtk::DrawingArea,
    pixels: Rc<RefCell<Option<gtk::gdk_pixbuf::Pixbuf>>>,
}
struct State {
    _keep_alive: Option<gio::ApplicationHoldGuard>,
    library: Library,
    library_ok: bool,
    results: Vec<Sound>,
    community: bool,
    generation: u64,
    playback: u64,
    audio: Audio,
    hotkeys: hotkeys::Hotkeys,
    volume: f64,
    monitor: bool,
    output: String,
    tx: Sender<Event>,
    cards: gtk::FlowBox,
    status: gtk::Label,
    summary: gtk::Label,
    heading: gtk::Label,
    search: gtk::Entry,
    window: gtk::ApplicationWindow,
    scroll: gtk::ScrolledWindow,
    thumbnails: Vec<ThumbnailSlot>,
    thumbnail_pending: HashSet<String>,
    thumbnail_attempted: HashSet<String>,
    nav_community: gtk::Button,
    nav_library: gtk::Button,
}

fn class<W: IsA<gtk::Widget>>(w: &W, name: &str) {
    w.style_context().add_class(name);
}
fn label(text: &str, style: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.0);
    class(&l, style);
    l
}
fn button(text: &str, style: &str) -> gtk::Button {
    let b = gtk::Button::with_label(text);
    class(&b, style);
    b
}
fn compact_device_picker(picker: &gtk::ComboBoxText) {
    picker.set_size_request(215, -1);
    for cell in picker.cells() {
        if let Ok(text) = cell.downcast::<gtk::CellRendererText>() {
            text.set_ellipsize(gtk::pango::EllipsizeMode::End);
            text.set_max_width_chars(25);
            text.set_width_chars(18);
        }
    }
}

fn status(s: &AppState, text: &str) {
    s.borrow().status.set_text(text);
}
fn failure(s: &AppState, error: anyhow::Error) {
    status(s, &format!("Erreur : {error:#}"));
}

fn artwork(sound: &Sound) -> ThumbnailSlot {
    let area = gtk::DrawingArea::new();
    area.set_size_request(0, 160);
    area.connect_size_allocate(|area, allocation| {
        // La tuile suit la largeur de sa colonne et conserve un format carré.
        if allocation.width() > 1 && area.height_request() != allocation.width() {
            let area = area.clone();
            let side = allocation.width();
            // GTK doit recevoir la nouvelle demande après son allocation courante.
            glib::idle_add_local_once(move || area.set_height_request(side));
        }
    });
    let pixels: Rc<RefCell<Option<gtk::gdk_pixbuf::Pixbuf>>> = Rc::new(RefCell::new(None));
    let p = pixels.clone();
    let seed = sound.id.bytes().map(u32::from).sum::<u32>();
    area.connect_draw(move |w, cr| {
        let width = w.allocated_width() as f64;
        let height = w.allocated_height() as f64;
        let radius = 8.0;
        cr.new_sub_path();
        cr.arc(
            width - radius,
            radius,
            radius,
            -std::f64::consts::FRAC_PI_2,
            0.0,
        );
        cr.arc(
            width - radius,
            height - radius,
            radius,
            0.0,
            std::f64::consts::FRAC_PI_2,
        );
        cr.arc(
            radius,
            height - radius,
            radius,
            std::f64::consts::FRAC_PI_2,
            std::f64::consts::PI,
        );
        cr.arc(
            radius,
            radius,
            radius,
            std::f64::consts::PI,
            3.0 * std::f64::consts::FRAC_PI_2,
        );
        cr.close_path();
        cr.clip();
        if let Some(p) = p.borrow().as_ref() {
            let factor = (width / p.width() as f64).max(height / p.height() as f64);
            cr.translate(
                (width - p.width() as f64 * factor) / 2.0,
                (height - p.height() as f64 * factor) / 2.0,
            );
            cr.scale(factor, factor);
            cr.set_source_pixbuf(p, 0.0, 0.0);
            let _ = cr.paint();
        } else {
            let colors = [
                (0.16, 0.28, 0.25),
                (0.25, 0.20, 0.37),
                (0.16, 0.25, 0.37),
                (0.32, 0.23, 0.19),
            ];
            let (r, g, b) = colors[(seed % 4) as usize];
            let gradient = gtk::cairo::LinearGradient::new(0.0, 0.0, width, height);
            gradient.add_color_stop_rgb(0.0, r, g, b);
            gradient.add_color_stop_rgb(1.0, r * 0.45, g * 0.45, b * 0.45);
            let _ = cr.set_source(&gradient);
            let _ = cr.paint();
            cr.set_source_rgba(0.8, 0.96, 0.86, 0.48);
            for i in 0..9 {
                let h = 16.0 + ((seed as usize + i * 37) % 57) as f64;
                cr.rectangle(
                    width / 2.0 - 59.0 + i as f64 * 14.0,
                    (height - h) / 2.0,
                    6.0,
                    h,
                );
            }
            let _ = cr.fill();
        }
        glib::Propagation::Proceed
    });
    ThumbnailSlot {
        sound: sound.clone(),
        area,
        pixels,
    }
}

fn pump_thumbnails(state: &AppState) {
    let Ok(mut s) = state.try_borrow_mut() else {
        return;
    };
    if !s.window.is_visible() {
        return;
    }
    let adjustment = s.scroll.vadjustment();
    let top = adjustment.value();
    let bottom = top + adjustment.page_size();
    if bottom <= top {
        return;
    }
    let mut jobs = Vec::new();
    let slots: Vec<_> = s
        .thumbnails
        .iter()
        .filter_map(|slot| {
            let (_, y) = slot.area.translate_coordinates(&s.cards, 0, 0)?;
            if slot.pixels.borrow().is_none()
                && slot.area.allocated_width() > 1
                && (y as f64 + slot.area.allocated_height() as f64) > top - 100.0
                && (y as f64) < bottom + 100.0
            {
                Some((slot.sound.clone(), slot.area.clone(), slot.pixels.clone()))
            } else {
                None
            }
        })
        .collect();
    for (sound, area, pixels) in slots {
        if let Ok(path) = thumbnails::cached(&sound.id) {
            if path.is_file() {
                if let Ok(p) = gtk::gdk_pixbuf::Pixbuf::from_file(path) {
                    *pixels.borrow_mut() = Some(p);
                    area.queue_draw();
                    continue;
                }
            }
        }
        if s.thumbnail_pending.len() >= 2 {
            break;
        }
        if s.thumbnail_attempted.insert(sound.id.clone()) {
            s.thumbnail_pending.insert(sound.id.clone());
            jobs.push(sound);
        }
    }
    let tx = s.tx.clone();
    drop(s);
    for sound in jobs {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let result = thumbnails::fetch(&sound);
            let _ = tx.send(Event::Thumbnail(sound.id, result));
        });
    }
}

fn render(state: &AppState) {
    let mut s = state.borrow_mut();
    s.thumbnails.clear();
    for (b, active) in [
        (&s.nav_community, s.community),
        (&s.nav_library, !s.community),
    ] {
        if active {
            class(b, "selected");
        } else {
            b.style_context().remove_class("selected");
        }
    }
    for child in s.cards.children() {
        s.cards.remove(&child);
    }
    let filter = s.search.text().to_lowercase();
    let sounds: Vec<_> = if s.community {
        s.results.clone()
    } else {
        s.library
            .sounds
            .iter()
            .filter(|x| {
                format!("{} {}", x.name, x.author)
                    .to_lowercase()
                    .contains(&filter)
            })
            .cloned()
            .collect()
    };
    s.heading.set_text(if s.community {
        "Trouve le son parfait."
    } else {
        "Tes sons, toujours prêts."
    });
    s.summary.set_text(&format!(
        "{} son{} · {} dans ta bibliothèque",
        sounds.len(),
        if sounds.len() > 1 { "s" } else { "" },
        s.library.sounds.len()
    ));
    if sounds.is_empty() {
        let b = gtk::Box::new(gtk::Orientation::Vertical, 12);
        b.set_margin_top(70);
        b.set_margin_start(30);
        b.set_margin_end(30);
        b.pack_start(
            &label(
                if s.community {
                    "Aucun son trouvé"
                } else {
                    "Ton prochain son préféré t’attend."
                },
                "empty-title",
            ),
            false,
            false,
            0,
        );
        let text = if s.community {
            "Essaie un autre mot ou colle un lien Tuna vers un son."
        } else {
            "Ouvre Communauté, écoute un son, puis ajoute-le ici.\nTes sons ajoutés restent disponibles hors connexion."
        };
        let l = label(text, "muted");
        l.set_line_wrap(true);
        b.pack_start(&l, false, false, 0);
        s.cards.insert(&b, -1);
    }
    let limit = if s.community { 60 } else { usize::MAX };
    for sound in sounds.iter().take(limit) {
        let card = gtk::Overlay::new();
        card.set_width_request(164);
        class(&card, "sound-card");
        let saved = s.library.sounds.iter().any(|x| x.id == sound.id);
        let play = gtk::Button::new();
        class(&play, "sound-play");
        let art = artwork(sound);
        let cover = gtk::Overlay::new();
        cover.add(&art.area);
        let slot = s.library.sounds.iter().position(|x| x.id == sound.id);
        let tag = if !s.community && slot.is_some_and(|i| i < 8) {
            format!("F{}", slot.unwrap() + 1)
        } else if sound.duration_ms > 0 {
            format!("{:.1} s", sound.duration_ms as f64 / 1000.0)
        } else {
            "TUNA".into()
        };
        let badge = label(&tag, "cover-tag");
        badge.set_halign(gtk::Align::Start);
        badge.set_valign(gtk::Align::Start);
        badge.set_margin_top(6);
        badge.set_margin_start(6);
        cover.add_overlay(&badge);
        cover.set_overlay_pass_through(&badge, true);
        s.thumbnails.push(art);
        let title = label(&format!("▶  {}", sound.name), "sound-title");
        title.set_line_wrap(true);
        title.set_lines(2);
        title.set_max_width_chars(18);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        title.set_valign(gtk::Align::End);
        title.set_halign(gtk::Align::Fill);
        title.set_height_request(36);
        cover.add_overlay(&title);
        cover.set_overlay_pass_through(&title, true);
        play.add(&cover);
        play.set_tooltip_text(Some(&format!(
            "{} · {}\nCliquer sur la tuile pour diffuser vers le micro virtuel.",
            sound.name, sound.author
        )));
        let weak = Rc::downgrade(state);
        let x = sound.clone();
        play.connect_clicked(move |_| {
            if let Some(s) = weak.upgrade() {
                request_play(&s, x.clone(), false);
            }
        });
        card.add(&play);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 3);
        actions.set_halign(gtk::Align::End);
        actions.set_valign(gtk::Align::Start);
        actions.set_margin_top(6);
        actions.set_margin_end(6);
        let preview = gtk::Button::new();
        class(&preview, "tile-action");
        preview.set_image(Some(&gtk::Image::from_icon_name(Some("audio-headphones-symbolic"), gtk::IconSize::Menu)));
        preview.set_tooltip_text(Some("Casque seul : préécouter sans diffuser aux autres. Cliquer sur la tuile pour diffuser."));
        let weak = Rc::downgrade(state);
        let x = sound.clone();
        preview.connect_clicked(move |_| {
            if let Some(s) = weak.upgrade() {
                request_play(&s, x.clone(), true);
            }
        });
        actions.pack_start(&preview, false, false, 0);
        let save = button(
            if saved { "✓" } else { "+" },
            if saved { "saved" } else { "add" },
        );
        class(&save, "tile-action");
        save.set_sensitive(s.library_ok);
        save.set_tooltip_text(Some(if saved {
            "Retirer de la bibliothèque (le fichier en cache est conservé)"
        } else {
            "Télécharger et ajouter à ta bibliothèque"
        }));
        let weak = Rc::downgrade(state);
        let x = sound.clone();
        save.connect_clicked(move |_| {
            if let Some(s) = weak.upgrade() {
                if saved {
                    let mut st = s.borrow_mut();
                    let old = st.library.sounds.clone();
                    st.library.sounds.retain(|v| v.id != x.id);
                    if let Err(e) = st.library.save() {
                        st.library.sounds = old;
                        drop(st);
                        failure(&s, e);
                        return;
                    }
                    drop(st);
                    render(&s);
                    status(&s, "Son retiré de la bibliothèque.");
                } else {
                    request_save(&s, x.clone());
                }
            }
        });
        actions.pack_start(&save, false, false, 0);
        let source = button("↗", "subtle");
        class(&source, "tile-action");
        source.set_sensitive(!sound.url.is_empty());
        source.set_tooltip_text(Some("Voir ce son et son créateur sur Tuna"));
        let url = sound.url.clone();
        source.connect_clicked(move |_| {
            let _ = gio::AppInfo::launch_default_for_uri(&url, None::<&gio::AppLaunchContext>);
        });
        if !sound.url.is_empty() {
            actions.pack_end(&source, false, false, 0);
        }
        card.add_overlay(&actions);
        s.cards.insert(&card, -1);
    }
    s.cards.show_all();
    drop(s);
    let weak = Rc::downgrade(state);
    glib::idle_add_local_once(move || {
        if let Some(s) = weak.upgrade() {
            pump_thumbnails(&s);
        }
    });
}

fn request_search(s: &AppState, category: Option<String>) {
    let (tx, generation, query) = {
        let mut st = s.borrow_mut();
        st.community = true;
        st.generation += 1;
        st.status
            .set_text("Recherche dans les sons publics de Tuna…");
        (st.tx.clone(), st.generation, st.search.text().to_string())
    };
    std::thread::spawn(move || {
        let r = catalog::search(&query, category.as_deref());
        let _ = tx.send(Event::Results(generation, r));
    });
}
fn request_save(s: &AppState, sound: Sound) {
    status(s, &format!("Ajout de « {} »…", sound.name));
    let tx = s.borrow().tx.clone();
    std::thread::spawn(move || {
        let result = catalog::download(&sound, &store::cache_dir());
        let _ = tx.send(Event::Saved(sound, result));
    });
}
fn request_play(s: &AppState, sound: Sound, preview: bool) {
    let (tx, epoch) = {
        let mut st = s.borrow_mut();
        if !preview && !st.audio.connected {
            st.status.set_text("Pour diffuser aux autres, active le micro virtuel à gauche, puis choisis Tuna dans ton jeu ou Discord. « Casque seul » permet une préécoute locale.");
            return;
        }
        st.playback += 1;
        st.audio.stop();
        st.status
            .set_text(&format!("Préparation de « {} »…", sound.name));
        (st.tx.clone(), st.playback)
    };
    std::thread::spawn(move || {
        let result = catalog::download(&sound, &store::cache_dir());
        let _ = tx.send(Event::Ready(epoch, sound, result, preview));
    });
}
fn stop(s: &AppState) {
    let mut st = s.borrow_mut();
    st.playback += 1;
    st.audio.stop();
    st.status.set_text("Lecture arrêtée.");
}
fn slot(s: &AppState, n: usize) {
    let sound = n
        .checked_sub(1)
        .and_then(|i| s.borrow().library.sounds.get(i).cloned());
    if let Some(sound) = sound {
        request_play(s, sound, false);
    } else {
        status(
            s,
            "Ajoute des sons à ta bibliothèque pour utiliser ce raccourci.",
        );
    }
}

fn socket_path() -> Result<PathBuf> {
    let dir =
        PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").context("Session Linux nécessaire")?)
            .join("tuna-board");
    std::fs::create_dir_all(&dir)?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    Ok(dir.join("control.sock"))
}
fn send_control(message: &str) -> Result<()> {
    let socket = UnixDatagram::unbound()?;
    socket.connect(socket_path()?)?;
    socket.send(message.as_bytes())?;
    Ok(())
}

fn build(app: &gtk::Application, start_hidden: bool) {
    let preview_mode = std::env::var_os("TUNA_BOARD_RENDER_PATH").is_some();
    let mut audio = Audio::default();
    let restore_microphone = if preview_mode {
        None
    } else {
        std::env::var("TUNA_BOARD_RESTORE_MIC").ok()
    };
    let resumed_microphone = if preview_mode {
        None
    } else {
        audio.resume_existing().ok().flatten()
    };
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Tuna Board · Sons communautaires")
        .default_width(1180)
        .default_height(790)
        .window_position(gtk::WindowPosition::Center)
        .build();
    window.set_icon_name(Some("tuna-board"));
    if preview_mode {
        if let Some((width, height)) = std::env::var("TUNA_BOARD_RENDER_SIZE")
            .ok()
            .and_then(|s| s.split_once('x').map(|(w, h)| (w.parse::<i32>().ok(), h.parse::<i32>().ok())))
        {
            if let (Some(width), Some(height)) = (width, height) {
                window.set_default_size(width.clamp(900, 4096), height.clamp(700, 2160));
            }
        }
    }
    let css = gtk::CssProvider::new();
    if let Err(e) = css.load_from_data(include_bytes!("../assets/style.css")) {
        eprintln!("CSS: {e}");
    }
    if let Some(screen) = gtk::gdk::Screen::default() {
        gtk::StyleContext::add_provider_for_screen(
            &screen,
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
    let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    window.add(&root);
    let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 9);
    sidebar.set_width_request(246);
    class(&sidebar, "sidebar");
    let brand_row = gtk::Box::new(gtk::Orientation::Horizontal, 11);
    let icon_loader = gtk::gdk_pixbuf::PixbufLoader::new();
    icon_loader.set_size(28, 28);
    if icon_loader
        .write(include_bytes!("../assets/tuna-board.svg"))
        .is_ok()
        && icon_loader.close().is_ok()
    {
        brand_row.pack_start(
            &gtk::Image::from_pixbuf(icon_loader.pixbuf().as_ref()),
            false,
            false,
            0,
        );
    }
    brand_row.pack_start(&label("Tuna Board", "brand"), false, false, 0);
    sidebar.pack_start(&brand_row, false, false, 0);
    sidebar.pack_start(
        &label("LES SONS DE TA COMMUNAUTÉ", "eyebrow"),
        false,
        false,
        0,
    );
    let community = button("◉   Communauté", "nav");
    let library = button("▦   Ma soundboard", "nav");
    sidebar.pack_start(&community, false, false, 14);
    sidebar.pack_start(&library, false, false, 0);
    let sep = gtk::Separator::new(gtk::Orientation::Horizontal);
    sidebar.pack_start(&sep, false, false, 12);
    sidebar.pack_start(&label("DIFFUSION", "eyebrow"), false, false, 0);
    let mic_select = gtk::ComboBoxText::new();
    mic_select.append(Some(""), "Sons uniquement");
    let default_mic = resumed_microphone
        .or(restore_microphone)
        .unwrap_or_else(|| audio::default_source().unwrap_or_default());
    if let Ok(list) = audio::devices("sources") {
        for d in list {
            mic_select.append(Some(&d.name), &d.description);
        }
    }
    mic_select.set_active_id(Some(&default_mic));
    if mic_select.active_id().is_none() {
        mic_select.set_active(Some(0));
    }
    compact_device_picker(&mic_select);
    let auto_audio_error = if !preview_mode && !audio.connected {
        let selected = mic_select.active_id().unwrap_or_default();
        audio
            .connect(if selected.is_empty() {
                None
            } else {
                Some(selected.as_str())
            })
            .err()
            .map(|e| format!("Activation automatique impossible : {e:#}"))
    } else {
        None
    };
    sidebar.pack_start(
        &label("Micro à mélanger avec les sons", "muted"),
        false,
        false,
        0,
    );
    sidebar.pack_start(&mic_select, false, false, 0);
    let connect = button("Activer le micro virtuel", "primary");
    if audio.connected {
        connect.set_label("●  Micro virtuel actif");
        mic_select.set_sensitive(false);
    }
    if let Some(error) = auto_audio_error.as_deref() {
        connect.set_label("Réessayer l’activation");
        connect.set_tooltip_text(Some(error));
    }
    sidebar.pack_start(&connect, false, false, 0);
    let help = label(
        &format!(
            "Dans Discord ou ton jeu, choisis\n« {} ».",
            audio.source_description()
        ),
        "muted",
    );
    help.set_line_wrap(true);
    if let Some(error) = auto_audio_error.as_deref() {
        help.set_text(error);
    }
    sidebar.pack_start(&help, false, false, 0);
    let output_select = gtk::ComboBoxText::new();
    let output = audio::default_sink().unwrap_or_default();
    if let Ok(list) = audio::devices("sinks") {
        for d in list {
            output_select.append(Some(&d.name), &d.description);
        }
    }
    output_select.set_active_id(Some(&output));
    if output_select.active_id().is_none() {
        output_select.set_active(Some(0));
    }
    compact_device_picker(&output_select);
    sidebar.pack_start(&label("Sortie d’écoute", "muted"), false, false, 6);
    sidebar.pack_start(&output_select, false, false, 0);
    let monitor = gtk::CheckButton::with_label("Entendre les sons dans mon casque");
    monitor.set_active(true);
    class(&monitor, "muted");
    sidebar.pack_start(&monitor, false, false, 0);
    sidebar.pack_start(&label("Volume des sons", "muted"), false, false, 6);
    let volume = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 1.0);
    volume.set_value(55.0);
    volume.set_digits(0);
    volume.set_value_pos(gtk::PositionType::Right);
    sidebar.pack_start(&volume, false, false, 0);
    let globals = button("Activer les raccourcis en jeu", "subtle");
    sidebar.pack_start(&globals, false, false, 14);
    let shortcut_note = label(
        "Super + Alt + F1–F8\nSuper + Alt + Échap : arrêter",
        "muted",
    );
    sidebar.pack_start(&shortcut_note, false, false, 0);
    let foot = label(
        "Rust natif · PipeWire\nSons issus de Voicemod Tuna",
        "eyebrow",
    );
    sidebar.pack_end(&foot, false, false, 0);
    let quit_button = button("Quitter Tuna Board", "subtle");
    quit_button.set_tooltip_text(Some(
        "Arrêter complètement l’application, le micro virtuel et les sons",
    ));
    sidebar.pack_end(&quit_button, false, false, 10);
    root.pack_start(&sidebar, false, false, 0);
    let main = gtk::Box::new(gtk::Orientation::Vertical, 18);
    main.set_halign(gtk::Align::Fill);
    main.set_hexpand(true);
    class(&main, "main");
    let stage = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    root.pack_start(&stage, true, true, 0);
    stage.pack_start(&main, true, true, 0);
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let search = gtk::Entry::new();
    search.set_placeholder_text(Some("Rechercher un son ou coller un lien Tuna…"));
    class(&search, "search");
    top.pack_start(&search, true, true, 0);
    let search_btn = button("Rechercher", "primary");
    top.pack_end(&search_btn, false, false, 0);
    main.pack_start(&top, false, false, 0);
    let categories = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    main.pack_start(&categories, false, false, 0);
    let section = label("DÉCOUVRIR / VOICEMOD COMMUNITY", "eyebrow");
    main.pack_start(&section, false, false, 4);
    let heading = label("Trouve le son parfait.", "heading");
    main.pack_start(&heading, false, false, 2);
    let summary = label("Chargement…", "muted");
    main.pack_start(&summary, false, false, 0);
    let scroll = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroll.set_vexpand(true);
    let cards = gtk::FlowBox::new();
    cards.set_selection_mode(gtk::SelectionMode::None);
    cards.set_homogeneous(true);
    cards.set_min_children_per_line(1);
    cards.set_max_children_per_line(100);
    cards.set_column_spacing(10);
    cards.set_row_spacing(10);
    cards.set_valign(gtk::Align::Start);
    scroll.add(&cards);
    main.pack_start(&scroll, true, true, 0);
    let bottom = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    class(&bottom, "player");
    let status_label = label(
        "Bienvenue. Écoute un son ou construis ta soundboard.",
        "status",
    );
    status_label.set_line_wrap(true);
    status_label.set_max_width_chars(85);
    bottom.pack_start(&status_label, true, true, 0);
    let stop_btn = button("■  Tout arrêter", "stop");
    bottom.pack_end(&stop_btn, false, false, 0);
    main.pack_end(&bottom, false, false, 0);
    let (tx, rx) = mpsc::channel();
    let (lib, library_ok, load_error) = match Library::load() {
        Ok(l) => (l, true, None),
        Err(e) => (Library::default(), false, Some(e)),
    };
    let open_community = lib.sounds.is_empty();
    let state = Rc::new(RefCell::new(State {
        _keep_alive: if preview_mode { None } else { Some(app.hold()) },
        library: lib,
        library_ok,
        results: Vec::new(),
        community: open_community,
        generation: 0,
        playback: 0,
        audio,
        hotkeys: hotkeys::Hotkeys::default(),
        volume: 0.55,
        monitor: true,
        output,
        tx,
        cards,
        status: status_label,
        summary,
        heading,
        search: search.clone(),
        window: window.clone(),
        scroll: scroll.clone(),
        thumbnails: Vec::new(),
        thumbnail_pending: HashSet::new(),
        thumbnail_attempted: HashSet::new(),
        nav_community: community.clone(),
        nav_library: library.clone(),
    }));
    {
        let application = app.clone();
        quit_button.connect_clicked(move |_| application.quit());
    }
    for (name, tag) in [
        ("Tendances", None),
        ("Memes", Some("memes")),
        ("Gaming", Some("games")),
        ("Anime", Some("anime")),
        ("Effets", Some("sfx")),
    ] {
        let b = button(name, "chip");
        let s = state.clone();
        b.connect_clicked(move |_| {
            let entry = s.borrow().search.clone();
            entry.set_text("");
            request_search(&s, tag.map(str::to_owned));
        });
        categories.pack_start(&b, false, false, 0);
    }
    {
        let s = state.clone();
        search_btn.connect_clicked(move |_| request_search(&s, None));
    }
    {
        let s = state.clone();
        search.connect_activate(move |_| request_search(&s, None));
    }
    {
        let s = state.clone();
        search.connect_changed(move |_| {
            if !s.borrow().community {
                render(&s);
            }
        });
    }
    {
        let s = state.clone();
        community.connect_clicked(move |_| request_search(&s, None));
    }
    {
        let s = state.clone();
        library.connect_clicked(move |_| {
            s.borrow_mut().community = false;
            let entry = s.borrow().search.clone();
            entry.set_text("");
            render(&s);
        });
    }
    {
        let s = state.clone();
        stop_btn.connect_clicked(move |_| stop(&s));
    }
    {
        let s = state.clone();
        volume.connect_value_changed(move |v| s.borrow_mut().volume = v.value() / 100.0);
    }
    {
        let s = state.clone();
        monitor.connect_toggled(move |b| s.borrow_mut().monitor = b.is_active());
    }
    {
        let s = state.clone();
        output_select.connect_changed(move |b| {
            if let Some(id) = b.active_id() {
                s.borrow_mut().output = id.to_string();
            }
        });
    }
    {
        let s = state.clone();
        let mic = mic_select.clone();
        let help = help.clone();
        connect.connect_clicked(move |b| {
            let mut st = s.borrow_mut();
            if st.audio.connected {
                st.audio.disconnect();
                mic.set_sensitive(true);
                b.set_label("Activer le micro virtuel");
                st.status
                    .set_text("Micro virtuel désactivé. Écoute locale disponible.");
            } else {
                let selected = mic.active_id().unwrap_or_default();
                match st.audio.connect(if selected.is_empty() {
                    None
                } else {
                    Some(selected.as_str())
                }) {
                    Ok(()) => {
                        mic.set_sensitive(false);
                        b.set_label("●  Micro virtuel actif");
                        help.set_text(&format!(
                            "Dans Discord ou ton jeu, choisis\n« {} ».",
                            st.audio.source_description()
                        ));
                        st.status
                            .set_text("Prêt. Choisis « Tuna » dans ton jeu ou Discord.");
                    }
                    Err(e) => st.status.set_text(&format!("Erreur audio : {e:#}")),
                }
            }
        });
    }
    {
        let s = state.clone();
        globals.connect_clicked(move |b| {
            let mut st = s.borrow_mut();
            if st.hotkeys.enabled() { st.hotkeys.disable(); b.set_label("Activer les raccourcis en jeu"); st.status.set_text("Raccourcis globaux désactivés."); }
            else { match st.hotkeys.enable() {
                Ok(()) => { b.set_label("Désactiver les raccourcis en jeu"); st.status.set_text("Super + Alt + F1–F8 : sons de ta bibliothèque. Super + Alt + Échap : arrêter."); }
                Err(e) => st.status.set_text(&format!("Raccourcis : {e:#}")),
            } }
        });
    }
    {
        let s = state.clone();
        window.connect_key_press_event(move |_, e| {
            let name = e.keyval().name().unwrap_or_default();
            if name.as_str() == "Escape" {
                stop(&s);
                return glib::Propagation::Stop;
            }
            if let Some(n) = name
                .strip_prefix('F')
                .and_then(|x| x.parse::<usize>().ok())
                .filter(|n| (1..=8).contains(n))
            {
                slot(&s, n);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
    }
    let socket = if preview_mode {
        Ok(None)
    } else {
        socket_path().and_then(|p| {
            if p.exists() {
                std::fs::remove_file(&p)?;
            }
            let socket = UnixDatagram::bind(p)?;
            socket.set_nonblocking(true)?;
            Ok(Some(socket))
        })
    };
    if let Err(e) = &socket {
        status(&state, &format!("Contrôle local indisponible : {e}"));
    }
    let socket = socket.ok().flatten();
    {
        let weak = Rc::downgrade(&state);
        scroll.vadjustment().connect_value_changed(move |_| {
            if let Some(s) = weak.upgrade() {
                pump_thumbnails(&s);
            }
        });
        let weak = Rc::downgrade(&state);
        state.borrow().cards.connect_size_allocate(move |_, _| {
            if let Some(s) = weak.upgrade() {
                pump_thumbnails(&s);
            }
        });
    }
    {
        let s = state.clone();
        glib::timeout_add_local(Duration::from_millis(100), move || {
            for event in rx.try_iter() {
                match event {
                    Event::Results(g, result) => {
                        if g != s.borrow().generation {
                            continue;
                        }
                        match result {
                            Ok(list) => {
                                let n = list.len();
                                s.borrow_mut().results = list;
                                if s.borrow().community {
                                    render(&s);
                                }
                                status(
                                    &s,
                                    &format!("{n} sons à découvrir · Diffuser pour partager, Casque seul pour préécouter."),
                                );
                            }
                            Err(e) => failure(&s, e),
                        }
                    }
                    Event::Thumbnail(id, result) => {
                        let mut st = s.borrow_mut();
                        st.thumbnail_pending.remove(&id);
                        if let Some(path) = result.ok().filter(|_| st.window.is_visible()) {
                            if let Ok(p) = gtk::gdk_pixbuf::Pixbuf::from_file(path) {
                                for slot in &st.thumbnails {
                                    if slot.sound.id == id {
                                        *slot.pixels.borrow_mut() = Some(p.clone());
                                        slot.area.queue_draw();
                                    }
                                }
                            }
                        }
                        drop(st);
                        pump_thumbnails(&s);
                    }
                    Event::Ready(epoch, sound, result, preview) => {
                        if epoch != s.borrow().playback {
                            continue;
                        }
                        match result {
                            Ok(path) => {
                                let mut st = s.borrow_mut();
                                let out = st.output.clone();
                                let vol = st.volume;
                                let monitor = st.monitor;
                                let destination = if preview {
                                    "préécoute casque seulement"
                                } else {
                                    "diffusion vers le micro virtuel"
                                };
                                match st.audio.play(&path, &out, preview, monitor, vol) {
                                    Ok(()) => st
                                        .status
                                        .set_text(&format!("▶ {} · {}", sound.name, destination)),
                                    Err(e) => {
                                        st.status.set_text(&format!("Erreur de lecture : {e:#}"))
                                    }
                                }
                            }
                            Err(e) => failure(&s, e),
                        }
                    }
                    Event::Saved(sound, result) => match result {
                        Ok(_) => {
                            let mut st = s.borrow_mut();
                            if st.library_ok && !st.library.sounds.iter().any(|x| x.id == sound.id)
                            {
                                st.library.sounds.push(sound.clone());
                                if let Err(e) = st.library.save() {
                                    st.library.sounds.pop();
                                    drop(st);
                                    failure(&s, e);
                                    continue;
                                }
                            }
                            drop(st);
                            render(&s);
                            status(
                                &s,
                                &format!(
                                    "✓ {} ajouté à ta soundboard · disponible hors connexion",
                                    sound.name
                                ),
                            );
                        }
                        Err(e) => failure(&s, e),
                    },
                }
            }
            if let Some(socket) = &socket {
                let mut buffer = [0; 128];
                while let Ok(n) = socket.recv(&mut buffer) {
                    let msg = String::from_utf8_lossy(&buffer[..n]);
                    if msg == "stop" {
                        stop(&s);
                    } else if msg == "present" {
                        let window = s.borrow().window.clone();
                        window.show_all();
                        window.present();
                        pump_thumbnails(&s);
                    } else if msg == "hide" {
                        let window = s.borrow().window.clone();
                        window.close();
                    } else if msg == "quit" {
                        let application = s.borrow().window.application();
                        if let Some(application) = application {
                            application.quit();
                        }
                        return glib::ControlFlow::Break;
                    } else if let Some(n) = msg.strip_prefix("slot ").and_then(|v| v.parse().ok()) {
                        slot(&s, n);
                    }
                }
            }
            let error = s.borrow_mut().audio.reap();
            if let Some(error) = error {
                status(&s, &format!("Erreur audio : {error}"));
            }
            glib::ControlFlow::Continue
        });
    }
    {
        let s = state.clone();
        window.connect_delete_event(move |window, _| {
            if !preview_mode {
                window.hide();
                for thumbnail in &s.borrow().thumbnails {
                    *thumbnail.pixels.borrow_mut()=None;
                }
                status(&s,"Tuna Board reste actif en arrière-plan. Rouvre-le depuis le launcher ; « Quitter Tuna Board » l’arrête complètement.");
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
    }
    {
        let s = state.clone();
        app.connect_shutdown(move |_| {
            let mut st = s.borrow_mut();
            st.audio.disconnect();
            st.hotkeys.disable();
            if !preview_mode {
                if let Ok(path) = socket_path() {
                    let _ = std::fs::remove_file(path);
                }
            }
        });
    }
    render(&state);
    if !start_hidden || preview_mode {
        window.show_all();
    }
    if let Some(path) = std::env::var_os("TUNA_BOARD_RENDER_PATH") {
        let window = window.clone();
        let app = app.clone();
        glib::timeout_add_local_once(Duration::from_secs(6), move || {
            let w = window.allocated_width();
            let h = window.allocated_height();
            if let Ok(surface) = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, w, h)
            {
                if let Ok(context) = gtk::cairo::Context::new(&surface) {
                    window.draw(&context);
                    surface.flush();
                    if let Some(p) = gtk::gdk::pixbuf_get_from_surface(&surface, 0, 0, w, h) {
                        let _ = p.savev(PathBuf::from(path), "png", &[]);
                    }
                }
            }
            window.close();
            app.quit();
        });
    }
    if let Some(e) = load_error {
        failure(&state, e);
    } else if open_community && !start_hidden {
        request_search(&state, None);
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("Tuna Board : {e:#}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--search") => {
            let list = catalog::search(&args[1..].join(" "), None)?;
            println!("{}", serde_json::to_string_pretty(&list)?);
            return Ok(());
        }
        Some("--category") => {
            let list = catalog::search("", Some(args.get(1).context("Catégorie manquante")?))?;
            println!("{}", serde_json::to_string_pretty(&list)?);
            return Ok(());
        }
        Some("--inspect") | Some("--download") => {
            let list = catalog::search(args.get(1).context("Lien Tuna manquant")?, None)?;
            if args[0] == "--download" {
                for sound in list {
                    println!(
                        "{}",
                        catalog::download(&sound, &store::cache_dir())?.display()
                    );
                }
            } else {
                println!("{}", serde_json::to_string_pretty(&list)?);
            }
            return Ok(());
        }
        Some("--slot") => {
            let n: usize = args.get(1).context("Numéro manquant")?.parse()?;
            if !(1..=8).contains(&n) {
                bail!("Les emplacements vont de 1 à 8");
            }
            return send_control(&format!("slot {n}"))
                .context("Ouvre Tuna Board avant d’utiliser ses raccourcis");
        }
        Some("--stop") => return send_control("stop").context("Tuna Board n’est pas ouvert"),
        Some("--hide") => return send_control("hide").context("Tuna Board n’est pas ouvert"),
        Some("--quit") => return send_control("quit").context("Tuna Board n’est pas ouvert"),
        Some("--background") => (),
        Some("--help") | Some("-h") => {
            println!("Tuna Board — sons communautaires Voicemod pour Linux\n\nSans argument : ouvrir ou retrouver la fenêtre\n--background : démarrer sans afficher la fenêtre\n--search MOTS\n--category TAG\n--inspect LIEN_TUNA\n--download LIEN_TUNA\n--slot 1..8\n--stop\n--hide : garder l’application en arrière-plan\n--quit : arrêter complètement");
            return Ok(());
        }
        Some(other) => bail!("Argument inconnu : {other}. Voir --help"),
        None => (),
    }
    let start_hidden = args.first().is_some_and(|a| a == "--background");
    if std::env::var_os("TUNA_BOARD_RENDER_PATH").is_none()
        && send_control(if start_hidden { "hide" } else { "present" }).is_ok()
    {
        return Ok(());
    }
    let app = gtk::Application::new(
        Some("io.github.ODY92.TunaBoard"),
        if std::env::var_os("TUNA_BOARD_RENDER_PATH").is_some() {
            gio::ApplicationFlags::NON_UNIQUE
        } else {
            gio::ApplicationFlags::FLAGS_NONE
        },
    );
    app.connect_activate(move |app| {
        if let Some(window) = app.windows().first() {
            if !start_hidden {
                window.show_all();
                window.present();
            }
        } else {
            build(app, start_hidden);
        }
    });
    app.run_with_args(&["tuna-board"]);
    Ok(())
}
