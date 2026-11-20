//! Primitivas visuais skeuomórficas: bisel metálico, LED de 7 segmentos, botões e sliders.

use eframe::egui::{
    vec2, Align2, Color32, FontId, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2,
};

/// Fator de escala: 1 unidade do layout clássico = `S` pixels lógicos.
pub const S: f32 = 1.8;

pub const BG: Color32 = Color32::from_rgb(0x2a, 0x2b, 0x34);
pub const FACE: Color32 = Color32::from_rgb(0x3b, 0x3d, 0x4a);
pub const HI: Color32 = Color32::from_rgb(0x6a, 0x6d, 0x7e);
pub const LO: Color32 = Color32::from_rgb(0x0b, 0x0c, 0x10);
pub const LCD: Color32 = Color32::from_rgb(0x03, 0x0a, 0x05);
pub const LED: Color32 = Color32::from_rgb(0x2a, 0xff, 0x3a);
pub const LED_DIM: Color32 = Color32::from_rgb(0x0b, 0x2e, 0x10);
pub const TXT: Color32 = Color32::from_rgb(0x1f, 0xd8, 0x3a);
pub const ICON: Color32 = Color32::from_rgb(0xc8, 0xca, 0xd6);
pub const TITLE: Color32 = Color32::from_rgb(0x4a, 0x4d, 0x62);

/// Retângulo em unidades do layout clássico relativo à origem da janela.
pub fn rect(o: Pos2, x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::from_min_size(o + vec2(x, y) * S, vec2(w, h) * S)
}

pub fn bevel(p: &Painter, r: Rect, fill: Color32, raised: bool) {
    let (a, b) = if raised { (HI, LO) } else { (LO, HI) };
    let st = |c| Stroke::new((S * 0.8).max(1.0), c);
    p.rect_filled(r, 0.0, fill);
    p.line_segment([r.left_top(), r.right_top()], st(a));
    p.line_segment([r.left_top(), r.left_bottom()], st(a));
    p.line_segment([r.left_bottom(), r.right_bottom()], st(b));
    p.line_segment([r.right_top(), r.right_bottom()], st(b));
}

pub fn well(p: &Painter, r: Rect) {
    bevel(p, r, LCD, false);
}

pub fn title_bar(p: &Painter, r: Rect, title: &str) {
    p.rect_filled(r, 0.0, TITLE);
    let n = (r.height() / (S * 2.0)) as i32;
    for i in 0..n {
        let y = r.top() + S + i as f32 * S * 2.0;
        p.line_segment([Pos2::new(r.left() + 2.0, y), Pos2::new(r.right() - 2.0, y)], Stroke::new(S * 0.6, HI));
    }
    let g = p.layout_no_wrap(title.to_owned(), FontId::monospace(r.height() * 0.7), ICON);
    let pos = r.center() - g.size() / 2.0;
    p.rect_filled(Rect::from_min_size(pos, g.size()).expand2(vec2(S * 4.0, 0.0)), 0.0, TITLE);
    p.galley(pos, g, ICON);
}

const SEG: [u8; 10] = [
    0b0111111, 0b0000110, 0b1011011, 0b1001111, 0b1100110, 0b1101101, 0b1111101, 0b0000111,
    0b1111111, 0b1101111,
];

/// Desenha um dígito de 7 segmentos; `None` apaga todos. Retorna a largura do dígito.
fn digit(p: &Painter, x: f32, y: f32, h: f32, d: Option<u8>) -> f32 {
    let w = h * 0.55;
    let t = h * 0.13;
    let vh = (h - 3.0 * t) / 2.0;
    let mask = d.map_or(0, |d| SEG[(d % 10) as usize]);
    let segs = [
        (x + t, y, w - 2.0 * t, t),                         // a
        (x + w - t, y + t, t, vh),                          // b
        (x + w - t, y + 2.0 * t + vh, t, vh),               // c
        (x + t, y + h - t, w - 2.0 * t, t),                 // d
        (x, y + 2.0 * t + vh, t, vh),                       // e
        (x, y + t, t, vh),                                  // f
        (x + t, y + t + vh, w - 2.0 * t, t),                // g
    ];
    for (i, (sx, sy, sw, sh)) in segs.iter().enumerate() {
        let on = mask & (1 << i) != 0;
        p.rect_filled(
            Rect::from_min_size(Pos2::new(*sx, *sy), vec2(*sw, *sh)),
            0.0,
            if on { LED } else { LED_DIM },
        );
    }
    w
}

/// Visor de tempo "−MM:SS" em LED verde, centralizado em `r`.
pub fn led_time(p: &Painter, r: Rect, secs: f64, minus: bool) {
    let total = secs.max(0.0) as u64;
    let (m, s) = ((total / 60) % 100, total % 60);
    let h = r.height();
    let dw = h * 0.55;
    let gap = h * 0.16;
    let colon = h * 0.22;
    let width = dw * 4.0 + gap * 5.0 + colon + dw * 0.7 + gap;
    let mut x = r.center().x - width / 2.0;
    let y = r.top();

    let t = h * 0.13;
    let mc = if minus { LED } else { LED_DIM };
    p.rect_filled(
        Rect::from_min_size(Pos2::new(x, y + h / 2.0 - t / 2.0), vec2(dw * 0.7, t)),
        0.0,
        mc,
    );
    x += dw * 0.7 + gap;

    for d in [m / 10, m % 10] {
        x += digit(p, x, y, h, Some(d as u8)) + gap;
    }
    let cs = h * 0.12;
    for cy in [y + h * 0.3, y + h * 0.7] {
        p.rect_filled(Rect::from_min_size(Pos2::new(x + colon / 2.0 - cs / 2.0, cy), vec2(cs, cs)), 0.0, LED);
    }
    x += colon + gap;
    for d in [s / 10, s % 10] {
        x += digit(p, x, y, h, Some(d as u8)) + gap;
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Icon {
    Prev,
    Play,
    Pause,
    Stop,
    Next,
    Eject,
}

fn poly(p: &Painter, pts: Vec<Pos2>, col: Color32) {
    p.add(Shape::convex_polygon(pts, col, Stroke::NONE));
}
// `poly` e `bar` retornam `()` para que todos os braços do `match` de `draw_icon` tenham o mesmo tipo.

fn draw_icon(p: &Painter, c: Pos2, s: f32, icon: Icon, col: Color32) {
    let bar = |x: f32, y: f32, w: f32, h: f32| {
        p.rect_filled(Rect::from_min_size(Pos2::new(x, y), vec2(w, h)), 0.0, col);
    };
    let right = |cx: f32| vec![Pos2::new(cx - s * 0.6, c.y - s), Pos2::new(cx - s * 0.6, c.y + s), Pos2::new(cx + s * 0.9, c.y)];
    let left = |cx: f32| vec![Pos2::new(cx + s * 0.6, c.y - s), Pos2::new(cx + s * 0.6, c.y + s), Pos2::new(cx - s * 0.9, c.y)];
    match icon {
        Icon::Play => poly(p, right(c.x), col),
        Icon::Prev => {
            bar(c.x - s, c.y - s, s * 0.35, 2.0 * s);
            poly(p, left(c.x + s * 0.3), col);
        }
        Icon::Next => {
            bar(c.x + s - s * 0.35, c.y - s, s * 0.35, 2.0 * s);
            poly(p, right(c.x - s * 0.3), col);
        }
        Icon::Pause => {
            bar(c.x - s * 0.8, c.y - s, s * 0.6, 2.0 * s);
            bar(c.x + s * 0.2, c.y - s, s * 0.6, 2.0 * s);
        }
        Icon::Stop => bar(c.x - s * 0.8, c.y - s * 0.8, s * 1.6, s * 1.6),
        Icon::Eject => {
            poly(
                p,
                vec![Pos2::new(c.x - s, c.y + s * 0.1), Pos2::new(c.x + s, c.y + s * 0.1), Pos2::new(c.x, c.y - s)],
                col,
            );
            bar(c.x - s, c.y + s * 0.5, 2.0 * s, s * 0.45);
        }
    }
}

pub fn icon_button(ui: &Ui, r: Rect, icon: Icon, lit: bool, id: &str) -> Response {
    let resp = ui.interact(r, ui.id().with(id), Sense::click());
    let down = resp.is_pointer_button_down_on();
    let p = ui.painter();
    bevel(p, r, FACE, !down);
    let off = if down { vec2(S * 0.5, S * 0.5) } else { Vec2::ZERO };
    draw_icon(p, r.center() + off, r.height() * 0.26, icon, if lit { LED } else { ICON });
    resp
}

pub fn text_button(ui: &Ui, r: Rect, label: &str, lit: bool, id: &str) -> Response {
    let resp = ui.interact(r, ui.id().with(id), Sense::click());
    let down = resp.is_pointer_button_down_on();
    let p = ui.painter();
    bevel(p, r, FACE, !down);
    p.text(
        r.center() + if down { vec2(S * 0.5, S * 0.5) } else { Vec2::ZERO },
        Align2::CENTER_CENTER,
        label,
        FontId::monospace(r.height() * 0.52),
        if lit { LED } else { ICON },
    );
    resp
}

fn lerp_col(a: Color32, b: Color32, t: f32) -> Color32 {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

/// Slider horizontal (0..1). Retorna (alterou, resposta). Duplo clique restaura `dbl`.
pub fn hslider(ui: &Ui, r: Rect, v: &mut f32, id: &str, dbl: Option<f32>) -> (bool, Response) {
    let resp = ui.interact(r, ui.id().with(id), Sense::click_and_drag());
    let kw = r.height() * 1.1;
    let mut changed = false;
    if resp.is_pointer_button_down_on() {
        if let Some(pos) = resp.interact_pointer_pos() {
            let t = ((pos.x - r.left() - kw / 2.0) / (r.width() - kw)).clamp(0.0, 1.0);
            if (t - *v).abs() > f32::EPSILON {
                *v = t;
                changed = true;
            }
        }
    }
    if let (true, Some(d)) = (resp.double_clicked(), dbl) {
        *v = d;
        changed = true;
    }
    let p = ui.painter();
    let groove = Rect::from_center_size(r.center(), vec2(r.width(), r.height() * 0.45));
    well(p, groove);
    let kx = r.left() + *v * (r.width() - kw);
    let fill = Rect::from_min_max(groove.left_top() + vec2(1.0, 1.0), Pos2::new(kx + kw / 2.0, groove.bottom() - 1.0));
    p.rect_filled(fill, 0.0, lerp_col(LED_DIM, LED, *v));
    bevel(p, Rect::from_min_size(Pos2::new(kx, r.top()), vec2(kw, r.height())), FACE, !resp.is_pointer_button_down_on());
    (changed, resp)
}

/// Slider vertical (0..1, 1 = topo).
pub fn vslider(ui: &Ui, r: Rect, v: &mut f32, id: &str, dbl: Option<f32>) -> bool {
    let resp = ui.interact(r, ui.id().with(id), Sense::click_and_drag());
    let kh = r.width() * 0.75;
    let mut changed = false;
    if resp.is_pointer_button_down_on() {
        if let Some(pos) = resp.interact_pointer_pos() {
            let t = 1.0 - ((pos.y - r.top() - kh / 2.0) / (r.height() - kh)).clamp(0.0, 1.0);
            if (t - *v).abs() > f32::EPSILON {
                *v = t;
                changed = true;
            }
        }
    }
    if let (true, Some(d)) = (resp.double_clicked(), dbl) {
        *v = d;
        changed = true;
    }
    let p = ui.painter();
    let groove = Rect::from_center_size(r.center(), vec2(r.width() * 0.28, r.height()));
    well(p, groove);
    let ky = r.top() + (1.0 - *v) * (r.height() - kh);
    let mid = r.center().y;
    p.line_segment([Pos2::new(r.left(), mid), Pos2::new(groove.left(), mid)], Stroke::new(1.0, HI));
    p.line_segment([Pos2::new(groove.right(), mid), Pos2::new(r.right(), mid)], Stroke::new(1.0, HI));
    let fill_top = ky.min(mid);
    let fill_bot = (ky + kh).max(mid);
    p.rect_filled(
        Rect::from_min_max(Pos2::new(groove.left() + 1.0, fill_top), Pos2::new(groove.right() - 1.0, fill_bot)),
        0.0,
        lerp_col(LED_DIM, LED, (*v - 0.5).abs() * 2.0),
    );
    bevel(p, Rect::from_min_size(Pos2::new(r.left(), ky), vec2(r.width(), kh)), FACE, !resp.is_pointer_button_down_on());
    changed
}