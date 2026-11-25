//! Écran de chargement : un œil en ASCII qui apparaît, cligne, puis dans
//! lequel on plonge avant que l'interface n'apparaisse.
//!
//! L'œil est calculé à chaque image (pas d'image fixe) : chaque caractère
//! reçoit une luminosité, convertie en caractère de la palette RAMP.

use eframe::egui::{self, Align2, Color32, FontId, Id, Order, Pos2, Sense, Vec2};

/// Du plus sombre au plus clair.
const RAMP: &[u8] = b" .:-=+*#%@";

// Chronologie (secondes)
const APPEAR_END: f32 = 1.5;
/// Clignements : (début, intensité). Fermeture rapide, réouverture plus lente.
const BLINKS: &[(f32, f32)] = &[(2.2, 1.0), (3.75, 0.35)];
const BLINK_CLOSE: f32 = 0.09;
const BLINK_HOLD: f32 = 0.05;
const BLINK_OPEN: f32 = 0.22;
/// Saccades du regard : (instant, x, y). Le regard saute vite puis se fixe.
const SACCADES: &[(f32, f32, f32)] = &[
    (0.0, 0.0, 0.0),
    (1.25, -0.07, 0.012),
    (1.85, -0.025, 0.022),
    (2.75, 0.075, -0.022),
    (3.25, 0.035, 0.006),
    (3.9, 0.0, 0.0),
];
const SACCADE_LEN: f32 = 0.075;
const DILATE: (f32, f32) = (3.95, 4.5);
const ZOOM_START: f32 = 4.35;
const ZOOM_LEN: f32 = 1.3;
const REVEAL: (f32, f32) = (5.2, 6.25);
pub const DURATION: f32 = 6.25;

const GREY: Color32 = Color32::from_rgb(201, 206, 216);
const ACCENT: Color32 = Color32::from_rgb(88, 132, 255);

pub struct Splash {
    start: Option<f64>,
}

fn hash(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(374_761_393)
        ^ (y as u32).wrapping_mul(668_265_263)
        ^ seed.wrapping_mul(2_246_822_519);
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    h ^= h >> 16;
    (h & 0x00FF_FFFF) as f32 / 16_777_216.0
}

fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// État de l'œil à un instant donné.
struct Pose {
    open: f32,
    gaze: (f32, f32),
    pupil: f32,
    appear: f32,
    zoom: f32,
    cam: (f32, f32),
}

/// Ouverture des paupières (1 = ouvert, 0 = fermé) au fil des clignements.
fn eyelids(t: f32) -> f32 {
    let mut open: f32 = 1.0;
    for &(b, depth) in BLINKS {
        let u = t - b;
        let closed = if u < 0.0 {
            0.0
        } else if u < BLINK_CLOSE {
            smooth(0.0, BLINK_CLOSE, u)
        } else if u < BLINK_CLOSE + BLINK_HOLD {
            1.0
        } else {
            1.0 - smooth(BLINK_CLOSE + BLINK_HOLD, BLINK_CLOSE + BLINK_HOLD + BLINK_OPEN, u)
        };
        open = open.min(1.0 - closed * depth);
    }
    open
}

/// Direction du regard : saccades + micro-tremblements permanents.
fn gaze(t: f32) -> (f32, f32) {
    let mut g = (SACCADES[0].1, SACCADES[0].2);
    for w in SACCADES.windows(2) {
        let (k, x, y) = w[1];
        let m = smooth(k, k + SACCADE_LEN, t);
        // léger dépassement puis retour, comme un vrai mouvement d'œil
        let overshoot = 1.0 + 0.12 * (smooth(k, k + SACCADE_LEN, t) - smooth(k + SACCADE_LEN, k + SACCADE_LEN * 2.5, t));
        g.0 = lerp(g.0, x, m * overshoot.min(1.12));
        g.1 = lerp(g.1, y, m * overshoot.min(1.12));
    }
    let tremor = 0.0035;
    g.0 += tremor * ((t * 23.0).sin() + 0.6 * (t * 37.0 + 1.3).sin());
    g.1 += tremor * ((t * 29.0 + 0.7).sin() + 0.5 * (t * 41.0).sin());
    g
}

impl Pose {
    fn at(t: f32) -> Self {
        let appear = smooth(0.0, APPEAR_END, t);
        let open = eyelids(t);
        let zp = ((t - ZOOM_START) / ZOOM_LEN).clamp(0.0, 1.0);
        // Pendant la plongée, le regard se fige au centre.
        let freeze = smooth(ZOOM_START - 0.3, ZOOM_START, t);
        let g = gaze(t);
        let gaze = (g.0 * (1.0 - freeze), g.1 * (1.0 - freeze));
        // Pupille vivante (« hippus ») qui se dilate avant la plongée,
        // et se contracte un peu juste après le grand clignement.
        let hippus = 0.035 * (t * 2.1).sin() + 0.02 * (t * 3.3 + 1.0).sin();
        let after_blink = 0.08 * (smooth(BLINKS[0].0 + 0.2, BLINKS[0].0 + 0.45, t) - smooth(BLINKS[0].0 + 0.6, BLINKS[0].0 + 1.4, t));
        let pupil = 0.115 * (1.0 + hippus - after_blink + 0.32 * smooth(DILATE.0, DILATE.1, t));
        let zoom = (3.6 * zp * zp * zp).exp() * (1.0 + 2.2 * zp * zp);
        Pose { open, gaze, pupil, appear, zoom, cam: (0.0, 0.0) }
    }
}

/// Luminosité (0 à 1) d'un point du monde, et s'il appartient à l'iris.
fn shade(wx: f32, wy: f32, p: &Pose) -> (f32, bool) {
    let f = (1.0 - wx * wx).max(0.0).powf(0.85);
    let (bu, bl) = (0.42, 0.34);
    let meet = bl * f * 0.3;
    // La paupière supérieure suit le regard vertical, l'inférieure un peu.
    let follow = p.gaze.1 * f;
    let top = lerp(meet, -bu * f + follow * 1.6, p.open);
    let bottom = lerp(meet, bl * f + follow * 0.6, p.open);
    let inside = wx.abs() < 1.0 && wy > top && wy < bottom;

    if inside {
        let (gx, gy) = p.gaze;
        let (dx, dy) = (wx - gx, wy - gy);
        let d = (dx * dx + dy * dy).sqrt();
        let iris_r = 0.31;
        let dist_lid = (wy - top).min(bottom - wy);
        let lid_shadow = 0.28 * (1.0 - smooth(0.0, 0.07, dist_lid));
        let mut b;
        let mut accent = false;
        if d < p.pupil {
            b = 0.035;
            // reflet (disparaît pendant la plongée)
            let (hx, hy) = (wx - (gx * 0.45 - 0.055), wy - (gy * 0.45 - 0.06));
            if p.zoom < 1.25 && (hx * hx + hy * hy).sqrt() < 0.032 {
                b = 0.97;
            }
        } else if d < iris_r {
            let ang = dy.atan2(dx);
            let stri = 0.5 + 0.5 * (ang * 15.0 + 2.2 * (ang * 4.0).sin() + d * 18.0).sin();
            let limbal = smooth(iris_r * 0.78, iris_r, d);
            let collar = 1.0 - smooth(p.pupil, p.pupil * 1.5, d);
            b = 0.34 + 0.46 * stri * (1.0 - limbal) + 0.14 * collar - 0.12 * limbal;
            accent = true;
        } else {
            b = 0.80 - 0.30 * wx.abs().powi(3);
        }
        b -= lid_shadow;
        // Contour des paupières
        if (wy - top) < 0.02 || (bottom - wy) < 0.014 {
            b = 0.86;
            accent = false;
        }
        (b.clamp(0.0, 1.0), accent)
    } else {
        // Halo de points épars, comme une poussière autour de l'œil
        let e = ((wx / 1.04).powi(2) + (wy / 0.46).powi(2)).sqrt() - 1.0;
        let qx = (wx * 70.0).floor() as i32;
        let qy = (wy * 70.0).floor() as i32;
        let prob = 0.55 * (-e.max(0.0) * 4.2).exp();
        let h = hash(qx, qy, 5);
        if h < prob {
            (0.22 + 0.35 * hash(qx, qy, 9), false)
        } else {
            (0.0, false)
        }
    }
}

impl Splash {
    pub fn new() -> Self {
        Splash { start: None }
    }

    /// Dessine l'écran de chargement. Renvoie `false` quand il est terminé.
    pub fn show(&mut self, ctx: &egui::Context) -> bool {
        let now = ctx.input(|i| i.time);
        let start = *self.start.get_or_insert(now);
        let mut t = (now - start) as f32;
        if let Some(f) = std::env::var("LYRA_SPLASH_T").ok().and_then(|v| v.parse::<f32>().ok()) {
            t = f; // test : animation figée à cet instant
        }

        // Passer l'animation : on saute directement à la plongée finale.
        let key = ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. })));
        if key && t < ZOOM_START + ZOOM_LEN * 0.6 {
            self.skip(now, &mut t);
        }
        if t >= DURATION {
            return false;
        }
        ctx.request_repaint();

        let screen = ctx.screen_rect();
        let pose = Pose::at(t);
        // Révélation finale : l'écran se « décode » case par case.
        let reveal_k = smooth(REVEAL.0, REVEAL.1, t);
        let alpha = pose.appear;

        let mut clicked = false;
        egui::Area::new(Id::new("lyra_splash"))
            .order(Order::Foreground)
            .fixed_pos(screen.min)
            .interactable(true)
            .show(ctx, |ui| {
                let (rect, resp) = ui.allocate_exact_size(screen.size(), Sense::click());
                clicked = resp.clicked();
                if resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                let painter = ui.painter();
                let font = FontId::monospace(11.0);
                let cw = ctx.fonts(|f| f.glyph_width(&font, '@'));
                let ch = ctx.fonts(|f| f.row_height(&font));
                let cols = (rect.width() / cw).ceil() as i32;
                let rows = (rect.height() / ch).ceil() as i32;
                let center = rect.center();
                let a = rect.width() * 0.34; // demi-largeur de l'œil en pixels
                let x0 = center.x - cols as f32 * cw / 2.0;
                let y0 = center.y - rows as f32 * ch / 2.0;
                let max_d = (rect.size() / 2.0).length();

                // Progression de la révélation pour une case : un front irrégulier
                // part du centre ; chaque case s'efface sur une courte durée.
                let cell_reveal = |c: i32, r: i32, px: f32, py: f32| -> f32 {
                    if reveal_k <= 0.0 {
                        return 0.0;
                    }
                    let d = (Pos2::new(px, py) - center).length() / max_d;
                    let threshold = 0.68 * d + 0.32 * hash(c, r, 21);
                    ((reveal_k * 1.4 - threshold) / 0.4).clamp(0.0, 1.0)
                };

                // Fond noir, case par case pendant la révélation
                if reveal_k <= 0.0 {
                    painter.rect_filled(rect, 0.0, Color32::BLACK);
                } else {
                    let mut mesh = egui::Mesh::default();
                    for r in 0..rows {
                        let py = y0 + (r as f32 + 0.5) * ch;
                        for c in 0..cols {
                            let px = x0 + (c as f32 + 0.5) * cw;
                            let lp = cell_reveal(c, r, px, py);
                            if lp < 1.0 {
                                let o = 1.0 - smooth(0.25, 1.0, lp);
                                let cell = egui::Rect::from_min_size(
                                    Pos2::new(x0 + c as f32 * cw, y0 + r as f32 * ch),
                                    Vec2::new(cw + 0.5, ch + 0.5),
                                );
                                mesh.add_colored_rect(cell, Color32::BLACK.gamma_multiply(o));
                            }
                        }
                    }
                    painter.add(egui::Shape::mesh(mesh));
                }

                // Zone réservée au texte d'aide : aucun caractère derrière.
                let hint_alpha = pose.appear * (1.0 - smooth(ZOOM_START - 0.2, ZOOM_START + 0.2, t)) * 0.6;
                let hint_text = "Clique ou appuie sur une touche pour passer";
                let hint_font = FontId::proportional(12.5);
                let hint_pos = rect.center_bottom() - Vec2::new(0.0, 28.0);
                let hint_size = ctx.fonts(|f| f.layout_no_wrap(hint_text.to_string(), hint_font.clone(), GREY).size());
                let hint_zone = egui::Rect::from_center_size(hint_pos, hint_size + Vec2::new(56.0, 22.0));

                let grey = GREY.gamma_multiply(alpha);
                let accent = ACCENT.gamma_multiply(alpha);
                let decode = ACCENT.gamma_multiply(0.95);
                let flicker = (t * 26.0) as u32;
                let mut line_g = String::with_capacity(cols as usize);
                let mut line_a = String::with_capacity(cols as usize);
                let mut line_d = String::with_capacity(cols as usize);
                for r in 0..rows {
                    line_g.clear();
                    line_a.clear();
                    line_d.clear();
                    let py = y0 + (r as f32 + 0.5) * ch;
                    for c in 0..cols {
                        let px = x0 + (c as f32 + 0.5) * cw;
                        let lp = cell_reveal(c, r, px, py);
                        let hidden = hint_alpha > 0.01 && hint_zone.contains(Pos2::new(px, py));
                        if lp >= 1.0 || hidden {
                            line_g.push(' ');
                            line_a.push(' ');
                            line_d.push(' ');
                            continue;
                        }
                        if lp > 0.0 {
                            // Case en cours de « décodage » : caractères bleus qui
                            // scintillent puis s'amenuisent avant de disparaître.
                            let energy = (1.0 - lp) * (0.55 + 0.45 * hash(c, r, flicker));
                            let i = (energy * (RAMP.len() - 1) as f32).round().clamp(1.0, (RAMP.len() - 1) as f32) as usize;
                            line_g.push(' ');
                            line_a.push(' ');
                            line_d.push(RAMP[i] as char);
                            continue;
                        }
                        line_d.push(' ');
                        // Apparition : les points se rassemblent progressivement
                        let visible = hash(c, r, 1) < pose.appear * 1.15 - 0.05;
                        let (mut b, is_iris) = if visible {
                            let nx = (px - center.x) / a;
                            let ny = (py - center.y) / a;
                            shade(pose.cam.0 + nx / pose.zoom, pose.cam.1 + ny / pose.zoom, &pose)
                        } else {
                            (0.0, false)
                        };
                        if b > 0.0 {
                            b += (hash(c, r, 3) - 0.5) * 0.12;
                        }
                        let i = (b * (RAMP.len() - 1) as f32).round().clamp(0.0, (RAMP.len() - 1) as f32) as usize;
                        let chr = RAMP[i] as char;
                        if is_iris {
                            line_g.push(' ');
                            line_a.push(chr);
                        } else {
                            line_g.push(chr);
                            line_a.push(' ');
                        }
                    }
                    let pos = Pos2::new(x0, y0 + r as f32 * ch);
                    if !line_g.trim().is_empty() {
                        painter.text(pos, Align2::LEFT_TOP, &line_g, font.clone(), grey);
                    }
                    if !line_a.trim().is_empty() {
                        painter.text(pos, Align2::LEFT_TOP, &line_a, font.clone(), accent);
                    }
                    if !line_d.trim().is_empty() {
                        painter.text(pos, Align2::LEFT_TOP, &line_d, font.clone(), decode);
                    }
                }

                // Indication discrète, sur une zone dégagée
                if hint_alpha > 0.01 {
                    painter.text(hint_pos, Align2::CENTER_CENTER, hint_text, hint_font, GREY.gamma_multiply(hint_alpha));
                }
            });
        if clicked && t < ZOOM_START + ZOOM_LEN * 0.6 {
            self.skip(now, &mut t);
        }
        true
    }

    /// Avance directement à la fin de la plongée.
    fn skip(&mut self, now: f64, t: &mut f32) {
        let target = ZOOM_START + ZOOM_LEN * 0.6;
        self.start = Some(now - target as f64);
        *t = target;
    }
}
