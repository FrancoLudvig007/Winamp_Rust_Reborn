//! Winamp RS — clone do Winamp clássico em Rust com egui/eframe.
//! Janela sem moldura do sistema: barras de título, arrasto, minimizar e fechar são desenhados em egui.
//! Janelas acopladas: Player, Equalizador, Playlist e (opcional) Visualizador. Apenas .mp3 e .wav.

mod audio;
mod pyrsutils;
mod skin;

use std::collections::BTreeSet;
use std::f32::consts::TAU;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, pos2, vec2, Align2, Color32, FontId, Key, Painter, Pos2, Rect, Sense, Shape, Stroke, Ui,
};

use audio::{Engine, BANDS};
use pyrsutils::prelude::{basename, enumerate, isfile, lower, print, randint, range, shuffle, splitext};
use skin::{rect, Icon, S};

/// Ícone embutido no executável (arquivo na raiz do projeto).
const ICON_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/winamp.ico"));

const W: f32 = 275.0;
const MAIN_H: f32 = 128.0;
const EQ_H: f32 = 116.0;
const PL_H: f32 = 232.0;
const VIS_W: f32 = 520.0;
const ROW: f32 = 11.0;
const NBARS: usize = 19;
const EXTS: [&str; 2] = [".mp3", ".wav"];
const VIS_NAMES: [&str; 4] = ["POLÍGONOS", "ESPIRAL", "RADIAL", "LISSAJOUS"];
const EQ_LABELS: [&str; BANDS] = ["70", "180", "320", "600", "1K", "3K", "6K", "12K", "14K", "16K"];

const PRESETS: [(&str, [f32; BANDS]); 10] = [
    ("Flat", [0.0; BANDS]),
    ("Classical", [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, -4.0, -4.0, -4.0, -5.0]),
    ("Club", [0.0, 0.0, 3.0, 5.0, 5.0, 5.0, 3.0, 0.0, 0.0, 0.0]),
    ("Dance", [6.0, 4.0, 2.0, 0.0, 0.0, -3.0, -4.0, -4.0, 0.0, 0.0]),
    ("Full Bass", [5.0, 5.0, 5.0, 3.0, 1.0, -2.0, -4.0, -5.0, -5.0, -5.0]),
    ("Full Treble", [-5.0, -5.0, -5.0, -2.0, 1.0, 4.0, 6.0, 8.0, 8.0, 8.0]),
    ("Jazz", [4.0, 2.0, 1.0, 2.0, -2.0, -2.0, 0.0, 1.0, 3.0, 4.0]),
    ("Pop", [-1.0, 2.0, 4.0, 5.0, 3.0, 0.0, -1.0, -1.0, -1.0, -1.0]),
    ("Rock", [5.0, 3.0, 1.0, -3.0, -4.0, -1.0, 3.0, 5.0, 6.0, 6.0]),
    ("Techno", [5.0, 4.0, 0.0, -4.0, -3.0, 0.0, 5.0, 6.0, 6.0, 5.0]),
];

#[derive(Clone, Copy, PartialEq)]
enum PlayState {
    Stopped,
    Playing,
    Paused,
}

#[derive(Clone, Copy, PartialEq)]
enum PopKind {
    Misc,
    ListOpts,
    Presets,
}

enum Act {
    Clear,
    SortName,
    ShuffleList,
    Prune,
    ToggleShuffle,
    ToggleRepeat,
    Preset(usize),
}

struct Track {
    path: PathBuf,
    title: String,
}

struct App {
    engine: Option<Engine>,
    engine_err: Option<String>,
    state: PlayState,

    tracks: Vec<Track>,
    current: Option<usize>,
    selected: BTreeSet<usize>,
    scroll: f32,

    volume: f32,
    balance: f32,
    remaining: bool,
    shuffle: bool,
    repeat: bool,
    seek_drag: Option<f32>,

    eq_on: bool,
    eq_auto: bool,
    preamp: f32,
    bands: [f32; BANDS],

    bars: Vec<f32>,
    peaks: Vec<f32>,
    wave: Vec<f32>,

    status: String,
    status_at: Option<Instant>,
    popup: Option<(PopKind, Pos2, Align2)>,
    popup_fresh: bool,
    show_vis: bool,
    vis_preset: usize,
    vis_switch: Instant,
    t0: Instant,
}

fn is_media(path: &Path) -> bool {
    let ext = lower(&splitext(path).1);
    EXTS.contains(&ext.as_str())
}

fn hsv(h: f32, s: f32, v: f32, a: u8) -> Color32 {
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - f * s), v * (1.0 - (1.0 - f) * s));
    let (r, g, b) = match (i as i32).rem_euclid(6) {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    Color32::from_rgba_unmultiplied((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, a)
}

fn win_size(show_vis: bool) -> egui::Vec2 {
    vec2(W * S + if show_vis { VIS_W } else { 0.0 }, (MAIN_H + EQ_H + PL_H) * S)
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let (engine, engine_err) = match Engine::new() {
            Ok(e) => (Some(e), None),
            Err(m) => (None, Some(m)),
        };
        let mut app = App {
            engine,
            engine_err: engine_err.clone(),
            state: PlayState::Stopped,
            tracks: Vec::new(), // playlist começa vazia
            current: None,
            selected: BTreeSet::new(),
            scroll: 0.0,
            volume: 0.8,
            balance: 0.0,
            remaining: false,
            shuffle: false,
            repeat: false,
            seek_drag: None,
            eq_on: true,
            eq_auto: false,
            preamp: 0.0,
            bands: [0.0; BANDS],
            bars: vec![0.0; NBARS],
            peaks: vec![0.0; NBARS],
            wave: Vec::new(),
            status: String::new(),
            status_at: None,
            popup: None,
            popup_fresh: false,
            show_vis: false,
            vis_preset: 0,
            vis_switch: Instant::now(),
            t0: Instant::now(),
        };
        if let Some(m) = engine_err {
            app.err(m);
        }
        if let Some(e) = app.engine.as_mut() {
            e.set_volume(app.volume);
        }
        app.push_eq();
        app
    }

    // ------------------------------------------------------------------ estado

    fn err(&mut self, m: impl Into<String>) {
        let m = m.into();
        print(format!("[winamp] {m}"));
        self.status = m;
        self.status_at = Some(Instant::now());
    }

    fn add_files(&mut self, paths: Vec<PathBuf>) -> usize {
        let mut n = 0;
        for p in paths {
            if p.is_file() && is_media(&p) {
                let title = splitext(basename(&p)).0;
                self.tracks.push(Track { path: p, title });
                n += 1;
            } else {
                self.err(format!("Ignorado (apenas .mp3 e .wav): {}", p.display()));
            }
        }
        n
    }

    fn pick_files() -> Vec<PathBuf> {
        rfd::FileDialog::new()
            .set_title("Adicionar arquivos")
            .add_filter("Áudio (mp3, wav)", &["mp3", "wav"])
            .pick_files()
            .unwrap_or_default()
    }

    fn add_and_play_new(&mut self, files: Vec<PathBuf>) {
        let n = self.add_files(files);
        if n > 0 {
            let first = self.tracks.len() - n;
            self.play_index(first, 0.0);
        }
    }

    fn fix_current(&mut self, cur: Option<PathBuf>) {
        self.current = cur.and_then(|cp| self.tracks.iter().position(|t| t.path == cp));
    }

    fn cur_path(&self) -> Option<PathBuf> {
        self.current.and_then(|c| self.tracks.get(c)).map(|t| t.path.clone())
    }

    fn halt(&mut self) {
        if let Some(e) = self.engine.as_mut() {
            e.stop();
        }
        self.state = PlayState::Stopped;
    }

    fn load_audio(&mut self, path: &Path, start: f64) -> Result<(), String> {
        let Some(e) = self.engine.as_mut() else {
            return Err(self.engine_err.clone().unwrap_or_else(|| "Áudio indisponível".into()));
        };
        e.load(path, start)
    }

    fn play_index(&mut self, i: usize, start: f64) {
        if i >= self.tracks.len() {
            return;
        }
        self.halt();
        let path = self.tracks[i].path.clone();
        match self.load_audio(&path, start) {
            Ok(()) => {
                self.state = PlayState::Playing;
                self.current = Some(i);
                if start == 0.0 {
                    self.auto_eq(i);
                }
            }
            Err(m) => {
                let t = self.tracks[i].title.clone();
                self.err(format!("{t}: {m}"));
            }
        }
    }

    fn auto_eq(&mut self, i: usize) {
        if !self.eq_auto {
            return;
        }
        let tl = lower(&self.tracks[i].title);
        if let Some((_, b)) = PRESETS.iter().skip(1).find(|(n, _)| tl.contains(&lower(n))) {
            self.bands = *b;
            self.push_eq();
        }
    }

    /// Envia o estado do EQ ao thread de áudio; efeito imediato (em ~6 ms).
    fn push_eq(&self) {
        if let Some(e) = &self.engine {
            e.sh.set_eq(self.eq_on, self.preamp, &self.bands);
        }
    }

    fn set_volume(&mut self, v: f32) {
        self.volume = v;
        if let Some(e) = self.engine.as_mut() {
            e.set_volume(v);
        }
    }

    fn press_play(&mut self) {
        if self.state == PlayState::Paused {
            self.resume();
            return;
        }
        let i = self
            .current
            .or_else(|| self.selected.iter().next().copied())
            .or(if self.tracks.is_empty() { None } else { Some(0) });
        if let Some(i) = i {
            self.play_index(i, 0.0);
        }
    }

    fn resume(&mut self) {
        if let Some(e) = self.engine.as_mut() {
            e.resume();
        }
        self.state = PlayState::Playing;
    }

    fn toggle_pause(&mut self) {
        match self.state {
            PlayState::Playing => {
                if let Some(e) = self.engine.as_mut() {
                    e.pause();
                }
                self.state = PlayState::Paused;
            }
            PlayState::Paused => self.resume(),
            PlayState::Stopped => {}
        }
    }

    fn step(&mut self, dir: i64, auto: bool) {
        let n = self.tracks.len();
        if n == 0 {
            return;
        }
        let cur = self.current;
        let next = if self.shuffle && n > 1 {
            loop {
                let r = randint(0, n as i64 - 1) as usize;
                if Some(r) != cur {
                    break Some(r);
                }
            }
        } else {
            match cur {
                None => Some(0),
                Some(c) => {
                    let t = c as i64 + dir;
                    if (0..n as i64).contains(&t) {
                        Some(t as usize)
                    } else if self.repeat || !auto {
                        Some(t.rem_euclid(n as i64) as usize)
                    } else {
                        None
                    }
                }
            }
        };
        match next {
            Some(i) => self.play_index(i, 0.0),
            None => self.halt(),
        }
    }

    fn position(&self) -> f64 {
        if self.state == PlayState::Stopped {
            return 0.0;
        }
        self.engine.as_ref().map_or(0.0, |e| e.position())
    }

    fn duration(&self) -> Option<f64> {
        if self.state == PlayState::Stopped {
            return None;
        }
        self.engine.as_ref().and_then(|e| e.duration)
    }

    fn seek_to(&mut self, secs: f64) {
        let Some(path) = self.cur_path() else { return };
        let paused = self.state == PlayState::Paused;
        match self.load_audio(&path, secs) {
            Ok(()) => {
                if paused {
                    if let Some(e) = self.engine.as_mut() {
                        e.pause();
                    }
                }
            }
            Err(m) => {
                self.err(m);
                self.halt();
            }
        }
    }

    fn remove_selected(&mut self) {
        let cur = self.cur_path();
        let mut i = 0;
        self.tracks.retain(|_| {
            let keep = !self.selected.contains(&i);
            i += 1;
            keep
        });
        self.selected.clear();
        self.fix_current(cur);
    }

    fn apply(&mut self, a: Act) {
        let cur = self.cur_path();
        match a {
            Act::Clear => {
                self.halt();
                self.tracks.clear();
                self.current = None;
                self.selected.clear();
            }
            Act::SortName => {
                self.tracks.sort_by_cached_key(|t| lower(&t.title));
                self.selected.clear();
                self.fix_current(cur);
            }
            Act::ShuffleList => {
                shuffle(&mut self.tracks);
                self.selected.clear();
                self.fix_current(cur);
            }
            Act::Prune => {
                self.tracks.retain(|t| isfile(&t.path));
                self.selected.clear();
                self.fix_current(cur);
            }
            Act::ToggleShuffle => self.shuffle = !self.shuffle,
            Act::ToggleRepeat => self.repeat = !self.repeat,
            Act::Preset(i) => {
                self.bands = PRESETS[i].1;
                self.push_eq();
            }
        }
    }

    // ------------------------------------------------------------------ por quadro

    fn tick(&mut self) {
        if self.state == PlayState::Playing && self.engine.as_ref().map_or(false, |e| e.finished()) {
            self.step(1, true);
        }
    }

    fn update_viz(&mut self, dt: f32) {
        let target = match (&self.engine, self.state) {
            (Some(e), PlayState::Playing) => {
                let snap = e.snapshot(1024);
                self.wave = snap[snap.len().saturating_sub(512)..].to_vec();
                audio::spectrum(&snap, NBARS, e.sample_rate as f32)
            }
            _ => {
                self.wave.clear();
                vec![0.0; NBARS]
            }
        };
        for (i, t) in enumerate(target) {
            let b = self.bars[i];
            self.bars[i] = if t > b { t } else { (b - dt * 1.2).max(t).max(0.0) };
            let pk = self.peaks[i];
            self.peaks[i] = if self.bars[i] > pk { self.bars[i] } else { (pk - dt * 0.5).max(0.0) };
        }
        if self.vis_switch.elapsed() > Duration::from_secs(25) {
            self.vis_preset += 1;
            self.vis_switch = Instant::now();
        }
    }

    fn handle_input(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> =
            ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if !dropped.is_empty() {
            let was_stopped = self.state == PlayState::Stopped;
            let n = self.add_files(dropped);
            if n > 0 && was_stopped {
                let first = self.tracks.len() - n;
                self.play_index(first, 0.0);
            }
        }
        let (z, x, c, v, b, del) = ctx.input(|i| {
            (
                i.key_pressed(Key::Z),
                i.key_pressed(Key::X),
                i.key_pressed(Key::C),
                i.key_pressed(Key::V),
                i.key_pressed(Key::B),
                i.key_pressed(Key::Delete),
            )
        });
        if z {
            self.step(-1, false);
        }
        if x {
            self.press_play();
        }
        if c {
            self.toggle_pause();
        }
        if v {
            self.halt();
        }
        if b {
            self.step(1, false);
        }
        if del {
            self.remove_selected();
        }
    }

    // ------------------------------------------------------------------ desenho

    fn title(&self, ui: &Ui, ctx: &egui::Context, r: Rect, text: &str, btns: bool) {
        skin::title_bar(ui.painter(), r, text);
        let drag = ui.interact(r, ui.id().with(("title", text)), Sense::drag());
        if drag.drag_started() {
            ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        if btns {
            let bw = r.height();
            let cl = Rect::from_min_size(r.right_top() - vec2(bw, 0.0), vec2(bw, r.height()));
            let mn = cl.translate(vec2(-bw, 0.0));
            if skin::text_button(ui, cl, "x", false, "close").clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            if skin::text_button(ui, mn, "_", false, "min").clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            }
        }
    }

    fn ticker_text(&self) -> (String, Color32) {
        if let Some(t) = self.status_at {
            if t.elapsed() < Duration::from_secs(8) && !self.status.is_empty() {
                return (format!("{}  ***  ", self.status), Color32::from_rgb(255, 120, 80));
            }
        }
        if let (Some(i), true) = (self.current, self.state != PlayState::Stopped) {
            let d = self
                .duration()
                .map_or(String::new(), |d| format!(" ({}:{:02})", d as u64 / 60, d as u64 % 60));
            return (format!("{}. {}{}  ***  ", i + 1, self.tracks[i].title, d), skin::TXT);
        }
        ("WINAMP RS  ***  ADD ou arraste arquivos .mp3 e .wav  ***  ".into(), skin::TXT)
    }

    fn paint_ticker(&self, p: &Painter, r: Rect) {
        let (text, col) = self.ticker_text();
        let pp = p.with_clip_rect(r);
        let g = pp.layout_no_wrap(text, FontId::monospace(8.0 * S), col);
        let y = r.center().y - g.size().y / 2.0;
        let gw = g.size().x;
        if gw <= r.width() {
            pp.galley(pos2(r.left(), y), g, col);
        } else {
            let off = (self.t0.elapsed().as_secs_f32() * 40.0 * S) % gw;
            pp.galley(pos2(r.left() - off, y), g.clone(), col);
            pp.galley(pos2(r.left() - off + gw, y), g, col);
        }
    }

    fn info(&self) -> (String, String, Option<bool>) {
        match (&self.engine, self.state) {
            (Some(e), PlayState::Playing | PlayState::Paused) => (
                e.kbps.map_or("---".into(), |k| k.to_string()),
                (e.sample_rate / 1000).to_string(),
                Some(e.channels >= 2),
            ),
            _ => ("---".into(), "--".into(), None),
        }
    }

    fn draw_main(&mut self, ui: &Ui, ctx: &egui::Context, o: Pos2) {
        let p = ui.painter();
        skin::bevel(p, rect(o, 0.0, 0.0, W, MAIN_H), skin::BG, true);
        self.title(ui, ctx, rect(o, 3.0, 3.0, W - 6.0, 11.0), "WINAMP", true);

        // Visor de tempo
        let lcd = rect(o, 8.0, 20.0, 96.0, 40.0);
        skin::well(p, lcd);
        let (pos, dur) = (self.position(), self.duration());
        let rem = self.remaining && dur.is_some();
        let secs = if rem { (dur.unwrap_or(0.0) - pos).max(0.0) } else { pos };
        skin::led_time(p, rect(o, 14.0, 33.0, 84.0, 20.0), secs, rem);
        let label = match self.state {
            PlayState::Playing => "PLAY",
            PlayState::Paused => "PAUSE",
            PlayState::Stopped => "STOP",
        };
        p.text(lcd.left_top() + vec2(4.0, 3.0) * S, Align2::LEFT_TOP, label, FontId::monospace(6.5 * S), skin::TXT);
        if ui.interact(lcd, ui.id().with("time"), Sense::click()).clicked() {
            self.remaining = !self.remaining;
        }

        // Analisador de espectro
        let sp = rect(o, 108.0, 20.0, 84.0, 40.0);
        skin::well(p, sp);
        paint_spectrum(p, sp.shrink(S * 2.0), &self.bars, &self.peaks);

        // Bitrate / sample rate
        let inf = rect(o, 196.0, 20.0, 71.0, 40.0);
        skin::well(p, inf);
        let (kb, khz, st) = self.info();
        let f = FontId::monospace(8.0 * S);
        p.text(inf.left_top() + vec2(5.0, 3.0) * S, Align2::LEFT_TOP, format!("{kb} kbps"), f.clone(), skin::TXT);
        p.text(inf.left_top() + vec2(5.0, 15.0) * S, Align2::LEFT_TOP, format!("{khz} kHz"), f, skin::TXT);
        let small = FontId::monospace(6.5 * S);
        let lit = |on: bool| if on { skin::LED } else { skin::LED_DIM };
        p.text(inf.left_top() + vec2(5.0, 28.0) * S, Align2::LEFT_TOP, "MONO", small.clone(), lit(st == Some(false)));
        p.text(inf.left_top() + vec2(30.0, 28.0) * S, Align2::LEFT_TOP, "STEREO", small, lit(st == Some(true)));

        // Título rolante
        let tk = rect(o, 8.0, 64.0, 259.0, 12.0);
        skin::well(p, tk);
        self.paint_ticker(p, tk.shrink2(vec2(S * 2.0, 0.0)));

        // Barra de busca
        let sk = rect(o, 8.0, 80.0, 259.0, 8.0);
        let mut t = match (self.seek_drag, dur) {
            (Some(t), _) => t,
            (None, Some(d)) if d > 0.0 => (pos / d).clamp(0.0, 1.0) as f32,
            _ => 0.0,
        };
        let (changed, resp) = skin::hslider(ui, sk, &mut t, "seek", None);
        if let Some(d) = dur {
            if changed {
                self.seek_drag = Some(t);
            }
            if resp.drag_stopped() || resp.clicked() {
                if let Some(t) = self.seek_drag.take() {
                    self.seek_to(t as f64 * d);
                }
            }
        }

        // Volume, balanço, shuffle, repeat
        let la = |x: f32, s: &str| {
            p.text(rect(o, x, 92.0, 20.0, 10.0).left_center(), Align2::LEFT_CENTER, s, FontId::monospace(6.5 * S), skin::TXT);
        };
        la(8.0, "VOL");
        la(118.0, "BAL");
        let mut v = self.volume;
        if skin::hslider(ui, rect(o, 28.0, 92.0, 80.0, 10.0), &mut v, "vol", None).0 {
            self.set_volume(v);
        }
        let mut b = (self.balance + 1.0) / 2.0;
        if skin::hslider(ui, rect(o, 140.0, 92.0, 56.0, 10.0), &mut b, "bal", Some(0.5)).0 {
            if (b - 0.5).abs() < 0.04 {
                b = 0.5;
            }
            self.balance = b * 2.0 - 1.0;
            if let Some(e) = &self.engine {
                e.sh.set_balance(self.balance);
            }
        }
        if skin::text_button(ui, rect(o, 204.0, 91.0, 30.0, 12.0), "SHUF", self.shuffle, "shuf").clicked() {
            self.shuffle = !self.shuffle;
        }
        if skin::text_button(ui, rect(o, 236.0, 91.0, 30.0, 12.0), "REP", self.repeat, "rep").clicked() {
            self.repeat = !self.repeat;
        }

        // Transporte
        let y = 106.0;
        let playing = self.state == PlayState::Playing;
        if skin::icon_button(ui, rect(o, 8.0, y, 24.0, 18.0), Icon::Prev, false, "prev").clicked() {
            self.step(-1, false);
        }
        if skin::icon_button(ui, rect(o, 33.0, y, 24.0, 18.0), Icon::Play, playing, "play").clicked() {
            self.press_play();
        }
        if skin::icon_button(ui, rect(o, 58.0, y, 24.0, 18.0), Icon::Pause, self.state == PlayState::Paused, "pause").clicked() {
            self.toggle_pause();
        }
        if skin::icon_button(ui, rect(o, 83.0, y, 24.0, 18.0), Icon::Stop, false, "stop").clicked() {
            self.halt();
        }
        if skin::icon_button(ui, rect(o, 108.0, y, 24.0, 18.0), Icon::Next, false, "next").clicked() {
            self.step(1, false);
        }
        if skin::icon_button(ui, rect(o, 137.0, y, 22.0, 18.0), Icon::Eject, false, "eject").clicked() {
            let files = Self::pick_files();
            self.add_and_play_new(files);
        }
        if skin::text_button(ui, rect(o, 230.0, 108.0, 36.0, 14.0), "VIS", self.show_vis, "vis").clicked() {
            self.show_vis = !self.show_vis;
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(win_size(self.show_vis)));
        }
    }

    fn draw_eq(&mut self, ui: &Ui, ctx: &egui::Context, o: Pos2) {
        let p = ui.painter();
        skin::bevel(p, rect(o, 0.0, 0.0, W, EQ_H), skin::BG, true);
        self.title(ui, ctx, rect(o, 3.0, 3.0, W - 6.0, 11.0), "WINAMP EQUALIZER", false);

        if skin::text_button(ui, rect(o, 8.0, 19.0, 26.0, 12.0), "ON", self.eq_on, "eq_on").clicked() {
            self.eq_on = !self.eq_on;
            self.push_eq();
        }
        if skin::text_button(ui, rect(o, 36.0, 19.0, 32.0, 12.0), "AUTO", self.eq_auto, "eq_auto").clicked() {
            self.eq_auto = !self.eq_auto;
        }
        let pr = rect(o, 214.0, 19.0, 52.0, 12.0);
        if skin::text_button(ui, pr, "PRESETS", false, "eq_presets").clicked() {
            self.popup = Some((PopKind::Presets, pr.left_bottom(), Align2::LEFT_TOP));
            self.popup_fresh = true;
        }

        skin::well(p, rect(o, 8.0, 35.0, 259.0, 76.0));
        let small = FontId::monospace(6.0 * S);

        // Preamp
        let mut t = (self.preamp / 12.0 + 1.0) / 2.0;
        if skin::vslider(ui, rect(o, 18.0, 40.0, 14.0, 58.0), &mut t, "preamp", Some(0.5)) {
            let d = (t * 2.0 - 1.0) * 12.0;
            self.preamp = if d.abs() < 0.5 { 0.0 } else { d };
            self.push_eq();
        }
        p.text(rect(o, 16.0, 99.0, 18.0, 10.0).center(), Align2::CENTER_CENTER, "PREAMP", small.clone(), skin::TXT);
        for (txt, y) in [("+12 db", 40.0), ("+0 db", 66.0), ("-12 db", 92.0)] {
            p.text(rect(o, 36.0, y, 36.0, 8.0).left_center(), Align2::LEFT_CENTER, txt, small.clone(), skin::TXT);
        }

        // Bandas
        for (i, lab) in enumerate(EQ_LABELS) {
            let x = 78.0 + i as f32 * 18.5;
            let mut t = (self.bands[i] / 12.0 + 1.0) / 2.0;
            if skin::vslider(ui, rect(o, x, 40.0, 14.0, 58.0), &mut t, &format!("band{i}"), Some(0.5)) {
                let d = (t * 2.0 - 1.0) * 12.0;
                self.bands[i] = if d.abs() < 0.5 { 0.0 } else { d };
                self.push_eq();
            }
            p.text(rect(o, x - 2.0, 99.0, 18.0, 10.0).center(), Align2::CENTER_CENTER, lab, small.clone(), skin::TXT);
        }

        // Curva de resposta sobre as bandas
        let pts: Vec<Pos2> = (0..BANDS)
            .map(|i| {
                let x = 78.0 + i as f32 * 18.5 + 7.0;
                let t = (self.bands[i] / 12.0 + 1.0) / 2.0;
                rect(o, x, 40.0 + (1.0 - t) * (58.0 - 10.5) + 5.25, 0.0, 0.0).min
            })
            .collect();
        let curve = if self.eq_on { skin::LED.gamma_multiply(0.6) } else { skin::HI.gamma_multiply(0.5) };
        p.add(Shape::line(pts, Stroke::new(S * 0.7, curve)));
    }

    fn draw_playlist(&mut self, ui: &Ui, ctx: &egui::Context, o: Pos2) {
        let p = ui.painter();
        skin::bevel(p, rect(o, 0.0, 0.0, W, PL_H), skin::BG, true);
        self.title(ui, ctx, rect(o, 3.0, 3.0, W - 6.0, 11.0), "WINAMP PLAYLIST EDITOR", false);

        let list = rect(o, 8.0, 18.0, 243.0, 184.0);
        skin::well(p, list);
        let rows = (list.height() / (ROW * S)) as usize;
        let total = self.tracks.len();
        let maxs = total.saturating_sub(rows) as f32;

        let resp = ui.interact(list, ui.id().with("pl_list"), Sense::click());
        if resp.hovered() {
            let dy = ui.input(|i| i.smooth_scroll_delta.y);
            self.scroll -= dy / (ROW * S);
        }
        if resp.clicked() || resp.double_clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let idx = self.scroll as usize + ((pos.y - list.top()) / (ROW * S)) as usize;
                if idx < total {
                    if resp.double_clicked() {
                        self.selected.clear();
                        self.selected.insert(idx);
                        self.play_index(idx, 0.0);
                    } else if ui.input(|i| i.modifiers.command) {
                        if !self.selected.remove(&idx) {
                            self.selected.insert(idx);
                        }
                    } else {
                        self.selected.clear();
                        self.selected.insert(idx);
                    }
                }
            }
        }

        // Barra de rolagem
        let sb = rect(o, 254.0, 18.0, 13.0, 184.0);
        skin::well(p, sb);
        let thumb_h = if total > rows {
            (rows as f32 / total as f32).clamp(0.08, 1.0) * sb.height()
        } else {
            sb.height()
        };
        let travel = (sb.height() - thumb_h).max(1.0);
        let sr = ui.interact(sb, ui.id().with("pl_sb"), Sense::click_and_drag());
        if sr.is_pointer_button_down_on() && maxs > 0.0 {
            if let Some(pos) = sr.interact_pointer_pos() {
                self.scroll = ((pos.y - sb.top() - thumb_h / 2.0) / travel).clamp(0.0, 1.0) * maxs;
            }
        }
        self.scroll = self.scroll.clamp(0.0, maxs);
        let ty = sb.top() + if maxs > 0.0 { self.scroll / maxs * travel } else { 0.0 };
        skin::bevel(p, Rect::from_min_size(pos2(sb.left() + 1.0, ty), vec2(sb.width() - 2.0, thumb_h)), skin::FACE, true);

        // Linhas
        let pl = p.with_clip_rect(list);
        let first = self.scroll as usize;
        for r in range(rows as i64 + 1) {
            let i = first + r as usize;
            if i >= total {
                break;
            }
            let rr = Rect::from_min_size(
                pos2(list.left(), list.top() + r as f32 * ROW * S),
                vec2(list.width(), ROW * S),
            );
            if self.selected.contains(&i) {
                pl.rect_filled(rr, 0.0, Color32::from_rgb(0x0a, 0x2a, 0x66));
            }
            let col = if self.current == Some(i) { Color32::WHITE } else { skin::TXT };
            pl.text(
                pos2(rr.left() + S * 3.0, rr.center().y),
                Align2::LEFT_CENTER,
                format!("{}. {}", i + 1, self.tracks[i].title),
                FontId::monospace(7.5 * S),
                col,
            );
        }

        // Botões inferiores
        let y = 208.0;
        if skin::text_button(ui, rect(o, 8.0, y, 30.0, 16.0), "ADD", false, "pl_add").clicked() {
            let files = Self::pick_files();
            self.add_files(files);
        }
        if skin::text_button(ui, rect(o, 40.0, y, 30.0, 16.0), "REM", false, "pl_rem").clicked() {
            self.remove_selected();
        }
        if skin::text_button(ui, rect(o, 72.0, y, 30.0, 16.0), "SEL", false, "pl_sel").clicked() {
            if self.selected.len() == total {
                self.selected.clear();
            } else {
                self.selected = (0..total).collect();
            }
        }
        let misc = rect(o, 104.0, y, 34.0, 16.0);
        if skin::text_button(ui, misc, "MISC", false, "pl_misc").clicked() {
            self.popup = Some((PopKind::Misc, misc.left_top(), Align2::LEFT_BOTTOM));
            self.popup_fresh = true;
        }
        p.text(rect(o, 140.0, y, 54.0, 16.0).center(), Align2::CENTER_CENTER, format!("{total} ITENS"), FontId::monospace(6.5 * S), skin::TXT);
        let lo = rect(o, 196.0, y, 71.0, 16.0);
        if skin::text_button(ui, lo, "LIST OPTS", false, "pl_opts").clicked() {
            self.popup = Some((PopKind::ListOpts, lo.right_top(), Align2::RIGHT_BOTTOM));
            self.popup_fresh = true;
        }
    }

    fn draw_vis(&mut self, ui: &Ui, ctx: &egui::Context) {
        let full = ui.max_rect();
        let p = ui.painter();
        skin::bevel(p, full, skin::BG, true);
        let tb = Rect::from_min_size(full.min + vec2(S * 3.0, S * 3.0), vec2(full.width() - S * 6.0, 11.0 * S));
        self.title(ui, ctx, tb, "WINAMP VISUALIZER", false);
        let view = Rect::from_min_max(tb.left_bottom() + vec2(0.0, S * 3.0), full.right_bottom() - vec2(S * 3.0, S * 3.0));
        skin::well(p, view);
        let inner = view.shrink(S);
        self.paint_milk(p, inner);
        if ui.interact(inner, ui.id().with("vis_click"), Sense::click()).clicked() {
            self.vis_preset += 1;
            self.vis_switch = Instant::now();
        }
    }

    fn paint_milk(&self, p: &Painter, r: Rect) {
        let t = self.t0.elapsed().as_secs_f32();
        let avg = |a: usize, b: usize| self.bars[a..b].iter().sum::<f32>() / (b - a) as f32;
        let (bass, mid, tre) = (avg(0, 4), avg(4, 11), avg(11, NBARS));
        let pp = p.with_clip_rect(r);
        pp.rect_filled(
            r,
            0.0,
            Color32::from_rgb((6.0 + bass * 50.0) as u8, (4.0 + mid * 20.0) as u8, (14.0 + tre * 40.0) as u8),
        );
        let c = r.center();
        let rad = r.size().min_elem() * 0.45;
        let preset = self.vis_preset % 4;
        const TR: usize = 10;
        for k in (0..TR).rev() {
            let f = k as f32 / TR as f32;
            let alpha = ((1.0 - f) * 110.0) as u8;
            let zoom = 1.0 + f * (0.7 + bass * 1.2);
            let rot = t * (0.35 + mid) + f * 0.4 * (preset as f32 + 1.0);
            let col = hsv((t * 0.07 + f * 0.25 + preset as f32 * 0.2 + bass * 0.2).fract(), 0.85, 1.0, alpha);
            match preset {
                0 => {
                    let sides = 3 + (tre * 8.0) as usize;
                    let rr = rad * 0.28 * zoom * (1.0 + bass * 0.5);
                    let pts: Vec<Pos2> = (0..sides)
                        .map(|i| {
                            let a = rot + i as f32 * TAU / sides as f32;
                            c + vec2(a.cos(), a.sin()) * rr
                        })
                        .collect();
                    pp.add(Shape::closed_line(pts, Stroke::new(2.0 + bass * 4.0, col)));
                }
                1 => {
                    for i in 0..36 {
                        let a = rot * 2.0 + i as f32 * 0.55;
                        let d = i as f32 / 36.0 * rad * 0.95 * zoom;
                        pp.circle_filled(c + vec2(a.cos(), a.sin()) * d, 2.0 + mid * 8.0 * (1.0 - f * 0.5), col);
                    }
                }
                2 => {
                    for i in 0..38 {
                        let a = rot + i as f32 * TAU / 38.0;
                        let dir = vec2(a.cos(), a.sin());
                        let d0 = rad * 0.15 * zoom;
                        let d1 = d0 + (self.bars[i % NBARS] * rad * 0.7 + rad * 0.05) * zoom;
                        pp.line_segment([c + dir * d0, c + dir * d1], Stroke::new(3.0, col));
                    }
                }
                _ => {
                    let pts: Vec<Pos2> = (0..120)
                        .map(|i| {
                            let u = i as f32 / 120.0 * TAU;
                            c + vec2((u * 3.0 + rot).sin(), (u * 2.0 + rot * 0.7).cos())
                                * rad
                                * 0.55
                                * zoom
                                * (1.0 + bass * 0.4)
                        })
                        .collect();
                    pp.add(Shape::line(pts, Stroke::new(2.0, col)));
                }
            }
        }
        if self.wave.len() > 1 {
            let n = self.wave.len();
            let pts: Vec<Pos2> = self
                .wave
                .iter()
                .enumerate()
                .map(|(i, s)| pos2(r.left() + i as f32 / (n - 1) as f32 * r.width(), c.y + s * r.height() * 0.35))
                .collect();
            pp.add(Shape::line(pts, Stroke::new(1.5, Color32::WHITE.gamma_multiply(0.85))));
        }
        pp.text(
            r.left_bottom() + vec2(S * 4.0, -S * 4.0),
            Align2::LEFT_BOTTOM,
            format!("MILKDROP  [{}/4] {}   (clique: próximo)", preset + 1, VIS_NAMES[preset]),
            FontId::monospace(7.0 * S),
            skin::TXT,
        );
    }

    fn draw_popup(&mut self, ctx: &egui::Context) {
        let Some((kind, anchor, pivot)) = self.popup else { return };
        let fresh = std::mem::take(&mut self.popup_fresh);
        let (sh, rp) = (self.shuffle, self.repeat);
        let mut act = None;
        let area = egui::Area::new(egui::Id::new("winamp_popup"))
            .order(egui::Order::Foreground)
            .fixed_pos(anchor)
            .pivot(pivot)
            .show(ctx, |ui| {
                egui::Frame::none()
                    .fill(Color32::BLACK)
                    .stroke(Stroke::new(1.0, skin::HI))
                    .inner_margin(6.0)
                    .show(ui, |ui| {
                        ui.style_mut().visuals.override_text_color = Some(skin::TXT);
                        match kind {
                            PopKind::Misc => {
                                if ui.button("Ordenar por nome").clicked() {
                                    act = Some(Act::SortName);
                                }
                                if ui.button("Embaralhar lista").clicked() {
                                    act = Some(Act::ShuffleList);
                                }
                                if ui.button("Remover arquivos ausentes").clicked() {
                                    act = Some(Act::Prune);
                                }
                                if ui.button("Limpar lista").clicked() {
                                    act = Some(Act::Clear);
                                }
                            }
                            PopKind::ListOpts => {
                                if ui.selectable_label(sh, "Shuffle").clicked() {
                                    act = Some(Act::ToggleShuffle);
                                }
                                if ui.selectable_label(rp, "Repeat").clicked() {
                                    act = Some(Act::ToggleRepeat);
                                }
                            }
                            PopKind::Presets => {
                                for (i, (name, _)) in PRESETS.iter().enumerate() {
                                    if ui.selectable_label(false, *name).clicked() {
                                        act = Some(Act::Preset(i));
                                    }
                                }
                            }
                        }
                    });
            });
        let rc = area.response.rect;
        let outside = !fresh
            && ctx.input(|i| {
                i.pointer.any_pressed() && i.pointer.interact_pos().map_or(false, |p| !rc.contains(p))
            });
        if act.is_some() || outside || ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.popup = None;
        }
        if let Some(a) = act {
            self.apply(a);
        }
    }
}

fn paint_spectrum(p: &Painter, r: Rect, bars: &[f32], peaks: &[f32]) {
    const CELLS: usize = 10;
    let n = bars.len();
    let bw = r.width() / n as f32;
    let ch = r.height() / CELLS as f32;
    for i in range(n as i64) {
        let i = i as usize;
        let x = r.left() + i as f32 * bw;
        let filled = ((bars[i] * CELLS as f32).round() as usize).min(CELLS);
        for c in 0..filled {
            let col = if c < 5 {
                skin::LED
            } else if c < 8 {
                Color32::from_rgb(0xe8, 0xe0, 0x20)
            } else {
                Color32::from_rgb(0xff, 0x40, 0x20)
            };
            let y = r.bottom() - (c + 1) as f32 * ch;
            p.rect_filled(Rect::from_min_size(pos2(x + 1.0, y + 1.0), vec2(bw - 2.0, ch - 1.5)), 0.0, col);
        }
        if peaks[i] > 0.02 {
            let pk = ((peaks[i] * CELLS as f32) as usize).min(CELLS - 1);
            let y = r.bottom() - (pk + 1) as f32 * ch;
            p.rect_filled(Rect::from_min_size(pos2(x + 1.0, y + 1.0), vec2(bw - 2.0, 1.5)), 0.0, skin::ICON);
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let dt = ctx.input(|i| i.unstable_dt).min(0.1);
        self.handle_input(ctx);
        self.tick();
        self.update_viz(dt);

        if self.show_vis {
            egui::SidePanel::right("vis")
                .exact_width(VIS_W)
                .resizable(false)
                .frame(egui::Frame::none().fill(skin::BG))
                .show(ctx, |ui| self.draw_vis(ui, ctx));
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(skin::BG))
            .show(ctx, |ui| {
                let o = ui.max_rect().min;
                self.draw_main(ui, ctx, o);
                self.draw_eq(ui, ctx, o + vec2(0.0, MAIN_H * S));
                self.draw_playlist(ui, ctx, o + vec2(0.0, (MAIN_H + EQ_H) * S));
            });

        self.draw_popup(ctx);
        ctx.request_repaint_after(Duration::from_millis(if self.state == PlayState::Playing { 16 } else { 80 }));
    }
}

fn decode_icon() -> Result<image::RgbaImage, String> {
    if ICON_BYTES.is_empty() {
        return Err("winamp.ico não estava na raiz do projeto durante o build".into());
    }
    image::load_from_memory_with_format(ICON_BYTES, image::ImageFormat::Ico)
        .map(|i| i.into_rgba8())
        .map_err(|e| format!("winamp.ico embutido inválido: {e}"))
}

fn load_icon() -> Result<egui::IconData, String> {
    let img = decode_icon()?;
    let (width, height) = img.dimensions();
    Ok(egui::IconData { rgba: img.into_raw(), width, height })
}

/// `winamp-rs --export-icon arquivo.png` — usado pelo install.sh para registrar o ícone no sistema.
fn export_icon(dest: &str) -> Result<(), String> {
    decode_icon()?
        .save_with_format(dest, image::ImageFormat::Png)
        .map_err(|e| format!("Falha ao gravar {dest}: {e}"))
}

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--export-icon") {
        let Some(dest) = args.get(1) else {
            eprintln!("uso: winamp-rs --export-icon <arquivo.png>");
            std::process::exit(2);
        };
        if let Err(e) = export_icon(dest) {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    let size = win_size(false);
    // Sem moldura do sistema: toda a "janela" (título, arrasto, fechar, minimizar) é desenhada em egui.
    let mut vb = egui::ViewportBuilder::default()
        .with_inner_size([size.x, size.y])
        .with_decorations(false)
        .with_resizable(false)
        .with_title("Winamp RS")
        .with_app_id("winamp-rs");
    match load_icon() {
        Ok(icon) => vb = vb.with_icon(Arc::new(icon)),
        Err(e) => print(format!("[winamp] aviso: {e}")),
    }
    let opts = eframe::NativeOptions { viewport: vb, ..Default::default() };
    let files: Vec<PathBuf> = args.into_iter().map(PathBuf::from).collect();
    eframe::run_native(
        "Winamp RS",
        opts,
        Box::new(move |cc| {
            let mut app = App::new(cc);
            app.add_and_play_new(files);
            Ok(Box::new(app))
        }),
    )
}