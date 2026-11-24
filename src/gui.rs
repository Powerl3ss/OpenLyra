//! Interface graphique (egui).

use crate::formats;
use crate::model::{human_size, Group, Report, Risk};
use eframe::egui::{
    self, Align, Color32, CornerRadius, FontId, Frame, Layout, Margin, RichText, Sense, Stroke, StrokeKind, TextStyle,
    Vec2,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::updater::{self, Release};

const WIN_W: f32 = 1180.0;
const WIN_H: f32 = 690.0;

pub fn run(initial: Option<PathBuf>) -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Lyra | Nettoyeur de métadonnées")
            // Taille fixe : tient sur un écran 1366 × 768 (barre des tâches comprise).
            .with_inner_size([WIN_W, WIN_H])
            .with_min_inner_size([WIN_W, WIN_H])
            .with_max_inner_size([WIN_W, WIN_H])
            .with_resizable(false)
            .with_maximize_button(false)
            .with_drag_and_drop(true)
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-256.png"))
                    .unwrap_or_default(),
            ),
        ..Default::default()
    };
    eframe::run_native(
        "Lyra",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, initial)))),
    )
}

#[derive(Clone, Copy)]
struct Pal {
    bg: Color32,
    card: Color32,
    inner: Color32,
    border: Color32,
    text: Color32,
    weak: Color32,
    accent: Color32,
    success: Color32,
}

impl Pal {
    fn new(dark: bool) -> Self {
        if dark {
            Pal {
                bg: Color32::from_rgb(13, 15, 20),
                card: Color32::from_rgb(22, 25, 33),
                inner: Color32::from_rgb(17, 19, 26),
                border: Color32::from_rgb(38, 43, 56),
                text: Color32::from_rgb(232, 235, 242),
                weak: Color32::from_rgb(140, 148, 166),
                accent: Color32::from_rgb(88, 132, 255),
                success: Color32::from_rgb(52, 199, 120),
            }
        } else {
            Pal {
                bg: Color32::from_rgb(243, 245, 249),
                card: Color32::WHITE,
                inner: Color32::from_rgb(247, 248, 251),
                border: Color32::from_rgb(222, 226, 234),
                text: Color32::from_rgb(24, 28, 38),
                weak: Color32::from_rgb(105, 112, 128),
                accent: Color32::from_rgb(52, 98, 235),
                success: Color32::from_rgb(28, 150, 85),
            }
        }
    }
}

fn risk_color(r: Risk) -> Color32 {
    match r {
        Risk::High => Color32::from_rgb(235, 87, 87),
        Risk::Medium => Color32::from_rgb(242, 163, 51),
        Risk::Low => Color32::from_rgb(64, 186, 130),
    }
}

#[derive(PartialEq, Clone)]
enum Output {
    Copy,
    Replace,
    Custom(Option<PathBuf>),
}

struct Done {
    output: PathBuf,
    removed: usize,
    remaining: usize,
    notes: Vec<String>,
}

/// État de la mise à jour, partagé avec les fils de téléchargement.
#[derive(Clone)]
enum Upd {
    Idle,
    Checking { manual: bool },
    UpToDate,
    Available(Release),
    Downloading(f32),
    Installed(String),
    Failed(String),
}

struct App {
    dark: bool,
    upd: Arc<Mutex<Upd>>,
    upd_hidden: bool,
    show_notes: bool,
    report: Option<Report>,
    selected: HashSet<String>,
    expanded: HashSet<String>,
    filter: String,
    output: Output,
    done: Option<Done>,
    error: Option<String>,
    style_applied: Option<bool>,
}

impl App {
    fn new(cc: &eframe::CreationContext, initial: Option<PathBuf>) -> Self {
        let mut app = App {
            dark: true,
            upd: Arc::new(Mutex::new(Upd::Idle)),
            upd_hidden: false,
            show_notes: false,
            report: None,
            selected: HashSet::new(),
            expanded: HashSet::new(),
            filter: String::new(),
            output: Output::Copy,
            done: None,
            error: None,
            style_applied: None,
        };
        if let Some(p) = initial {
            app.load(&p);
        }
        // Vérification silencieuse au démarrage.
        app.check_updates(&cc.egui_ctx, false);
        app
    }

    fn upd_state(&self) -> Upd {
        self.upd.lock().map(|s| s.clone()).unwrap_or(Upd::Idle)
    }

    fn check_updates(&mut self, ctx: &egui::Context, manual: bool) {
        if matches!(self.upd_state(), Upd::Checking { .. } | Upd::Downloading(_)) {
            return;
        }
        self.upd_hidden = false;
        *self.upd.lock().unwrap() = Upd::Checking { manual };
        let state = self.upd.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let next = match updater::check() {
                Ok(Some(rel)) => Upd::Available(rel),
                Ok(None) if manual => Upd::UpToDate,
                Ok(None) => Upd::Idle,
                Err(e) if manual => Upd::Failed(format!("{e:#}")),
                Err(_) => Upd::Idle,
            };
            *state.lock().unwrap() = next;
            ctx.request_repaint();
        });
    }

    fn install_update(&mut self, ctx: &egui::Context, rel: Release) {
        *self.upd.lock().unwrap() = Upd::Downloading(0.0);
        let state = self.upd.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let st = state.clone();
            let c = ctx.clone();
            let result = updater::install(&rel, &move |done, total| {
                let f = if total > 0 { done as f32 / total as f32 } else { 0.0 };
                *st.lock().unwrap() = Upd::Downloading(f.min(1.0));
                c.request_repaint();
            });
            *state.lock().unwrap() = match result {
                Ok(()) => Upd::Installed(rel.version.clone()),
                Err(e) => Upd::Failed(format!("{e:#}")),
            };
            ctx.request_repaint();
        });
    }

    /// Bandeau de mise à jour affiché en haut de la zone principale.
    fn update_banner(&mut self, ui: &mut egui::Ui, p: &Pal) {
        if self.upd_hidden {
            return;
        }
        let state = self.upd_state();
        let (color, show) = match &state {
            Upd::Idle | Upd::Checking { manual: false } => return,
            Upd::Failed(_) => (risk_color(Risk::Medium), true),
            Upd::UpToDate | Upd::Installed(_) => (p.success, true),
            _ => (p.accent, true),
        };
        if !show {
            return;
        }
        let mut action_install: Option<Release> = None;
        let mut retry = false;
        Frame::default()
            .fill(color.gamma_multiply(0.10))
            .stroke(Stroke::new(1.0_f32, color.gamma_multiply(0.7)))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    match &state {
                        Upd::Checking { .. } => {
                            ui.spinner();
                            ui.label("Recherche de mises à jour…");
                        }
                        Upd::UpToDate => {
                            ui.label(RichText::new(format!("✔  Lyra est à jour (version {})", updater::VERSION)).strong().color(color));
                        }
                        Upd::Available(rel) => {
                            ui.label(RichText::new(format!("⬆  Nouvelle version {} disponible", rel.version)).strong().color(color));
                            ui.label(RichText::new(format!("(tu as la {})", updater::VERSION)).color(p.weak));
                        }
                        Upd::Downloading(f) => {
                            ui.label(RichText::new("Téléchargement de la mise à jour").strong());
                            ui.add(egui::ProgressBar::new(*f).show_percentage().desired_width(260.0));
                        }
                        Upd::Installed(v) => {
                            ui.label(RichText::new(format!("✔  Version {v} installée")).strong().color(color));
                            ui.label(RichText::new("Redémarre Lyra pour l'utiliser.").color(p.weak));
                        }
                        Upd::Failed(e) => {
                            ui.add(egui::Label::new(RichText::new(format!("⚠  Mise à jour impossible : {e}")).color(color)).wrap());
                        }
                        Upd::Idle => {}
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let closable = !matches!(state, Upd::Downloading(_) | Upd::Checking { .. });
                        if closable && ui.add_sized(Vec2::new(30.0, 30.0), egui::Button::new("✖")).on_hover_text("Masquer").clicked() {
                            self.upd_hidden = true;
                        }
                        match &state {
                            Upd::Available(rel) => {
                                if ui.add(egui::Button::new(RichText::new("Mettre à jour").strong().color(Color32::WHITE)).fill(p.accent)).clicked() {
                                    action_install = Some(rel.clone());
                                }
                                ui.hyperlink_to("Voir sur GitHub", &rel.page);
                                if !rel.notes.trim().is_empty() {
                                    let t = if self.show_notes { "Masquer les nouveautés" } else { "Nouveautés" };
                                    if ui.button(t).clicked() {
                                        self.show_notes = !self.show_notes;
                                    }
                                }
                            }
                            Upd::Installed(_) => {
                                if ui.add(egui::Button::new(RichText::new("↻  Redémarrer Lyra").strong().color(Color32::WHITE)).fill(p.success)).clicked() {
                                    if let Err(e) = updater::restart() {
                                        *self.upd.lock().unwrap() = Upd::Failed(format!("redémarrage impossible ({e}). Relance Lyra manuellement."));
                                    }
                                }
                            }
                            Upd::Failed(_) => {
                                if ui.button("Réessayer").clicked() {
                                    retry = true;
                                }
                            }
                            _ => {}
                        }
                    });
                });
                if let Upd::Available(rel) = &state {
                    if self.show_notes {
                        ui.separator();
                        let notes: String = rel.notes.lines().take(30).collect::<Vec<_>>().join("\n");
                        ui.add(egui::Label::new(RichText::new(notes).size(13.0)).wrap());
                    }
                }
            });
        ui.add_space(12.0);
        if let Some(rel) = action_install {
            self.install_update(ui.ctx(), rel);
        }
        if retry {
            self.check_updates(ui.ctx(), true);
        }
    }

    fn apply_style(&mut self, ctx: &egui::Context) {
        if self.style_applied == Some(self.dark) {
            return;
        }
        self.style_applied = Some(self.dark);
        let p = Pal::new(self.dark);
        let mut v = if self.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
        v.panel_fill = p.bg;
        v.window_fill = p.card;
        v.extreme_bg_color = p.inner;
        v.faint_bg_color = p.inner;
        v.override_text_color = Some(p.text);
        v.hyperlink_color = p.accent;
        v.selection.bg_fill = p.accent.gamma_multiply(0.35);
        v.selection.stroke = Stroke::new(1.0_f32, p.accent);
        v.window_corner_radius = CornerRadius::same(12);
        for w in [
            &mut v.widgets.noninteractive,
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
            &mut v.widgets.open,
        ] {
            w.corner_radius = CornerRadius::same(8);
        }
        v.widgets.inactive.bg_fill = p.inner;
        v.widgets.inactive.weak_bg_fill = p.inner;
        v.widgets.inactive.bg_stroke = Stroke::new(
            1.2_f32,
            if self.dark { p.border } else { Color32::from_rgb(190, 196, 208) },
        );
        v.widgets.hovered.weak_bg_fill = p.border;
        v.widgets.hovered.bg_fill = p.border;
        v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, p.accent.gamma_multiply(0.6));
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, p.border);
        v.window_stroke = Stroke::new(1.0_f32, p.border);
        ctx.set_visuals(v);

        let mut style = (*ctx.style()).clone();
        style.text_styles.insert(TextStyle::Heading, FontId::proportional(22.0));
        style.text_styles.insert(TextStyle::Body, FontId::proportional(14.5));
        style.text_styles.insert(TextStyle::Button, FontId::proportional(14.5));
        style.text_styles.insert(TextStyle::Small, FontId::proportional(12.5));
        style.text_styles.insert(TextStyle::Monospace, FontId::monospace(13.0));
        style.spacing.item_spacing = Vec2::new(8.0, 8.0);
        style.spacing.button_padding = Vec2::new(12.0, 6.0);
        style.spacing.interact_size.y = 28.0;
        ctx.set_style(style);
    }

    fn load(&mut self, path: &Path) {
        self.done = None;
        self.error = None;
        self.filter.clear();
        self.expanded.clear();
        match formats::analyze_file(path) {
            Ok(r) => {
                self.selected = r.groups.iter().filter(|g| g.removable && g.recommended).map(|g| g.id.clone()).collect();
                self.report = Some(r);
            }
            Err(e) => {
                self.report = None;
                self.error = Some(format!("{e:#}"));
            }
        }
    }

    fn pick(&mut self) {
        let mut exts: Vec<&str> = formats::SUPPORTED.to_vec();
        let upper: Vec<String> = exts.iter().map(|e| e.to_uppercase()).collect();
        exts.extend(upper.iter().map(|s| s.as_str()));
        if let Some(p) = rfd::FileDialog::new()
            .set_title("Choisir un document à analyser")
            .add_filter("Documents pris en charge", &exts)
            .add_filter("Tous les fichiers", &["*"])
            .pick_file()
        {
            self.load(&p);
        }
    }

    fn selected_entries(&self) -> usize {
        self.report
            .as_ref()
            .map(|r| r.groups.iter().filter(|g| self.selected.contains(&g.id)).map(|g| g.entries.len()).sum())
            .unwrap_or(0)
    }

    fn clean(&mut self) {
        let Some(report) = self.report.clone() else { return };
        self.error = None;
        let target = match &self.output {
            Output::Copy => formats::default_output(&report.path),
            Output::Replace => {
                let ok = rfd::MessageDialog::new()
                    .set_title("Remplacer le fichier d'origine ?")
                    .set_description(format!(
                        "« {} » va être remplacé par sa version nettoyée. Cette opération est définitive.",
                        report.file_name()
                    ))
                    .set_buttons(rfd::MessageButtons::YesNo)
                    .set_level(rfd::MessageLevel::Warning)
                    .show();
                if ok != rfd::MessageDialogResult::Yes {
                    return;
                }
                report.path.clone()
            }
            Output::Custom(Some(p)) => p.clone(),
            Output::Custom(None) => {
                self.error = Some("Choisis d'abord l'emplacement du fichier nettoyé.".into());
                return;
            }
        };
        let removed = self.selected_entries();
        let result = std::fs::read(&report.path)
            .map_err(anyhow::Error::from)
            .and_then(|b| formats::clean_bytes(&b, report.kind, &self.selected))
            .and_then(|(data, notes)| {
                let tmp = target.with_extension("lyra.tmp");
                std::fs::write(&tmp, &data)?;
                std::fs::rename(&tmp, &target)?;
                Ok(notes)
            });
        match result {
            Ok(notes) => {
                let remaining = formats::analyze_file(&target).map(|r| r.total_entries()).unwrap_or(0);
                self.done = Some(Done { output: target, removed, remaining, notes });
            }
            Err(e) => self.error = Some(format!("Le nettoyage a échoué : {e:#}")),
        }
    }

    fn export_report(&mut self) {
        let Some(r) = &self.report else { return };
        let stem = r.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        if let Some(p) = rfd::FileDialog::new()
            .set_title("Exporter le rapport")
            .set_file_name(format!("{stem}_rapport_metadonnees.txt"))
            .add_filter("Texte", &["txt"])
            .save_file()
        {
            if let Err(e) = std::fs::write(&p, r.to_text()) {
                self.error = Some(format!("Export impossible : {e}"));
            }
        }
    }
}

fn card(p: &Pal) -> Frame {
    Frame::default()
        .fill(p.card)
        .stroke(Stroke::new(1.0_f32, p.border))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(16))
}

/// Pastille de hauteur fixe, texte centré horizontalement et verticalement.
fn pill(ui: &mut egui::Ui, text: &str, color: Color32) {
    let galley = ui.painter().layout_no_wrap(text.to_string(), FontId::proportional(12.5), color);
    let size = Vec2::new((galley.size().x + 18.0).max(24.0), 24.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(12), color.gamma_multiply(0.16));
    let pos = rect.center() - galley.size() / 2.0;
    ui.painter().galley(pos, galley, color);
}

fn section_title(ui: &mut egui::Ui, p: &Pal, t: &str) {
    ui.add_space(4.0);
    ui.label(RichText::new(t.to_uppercase()).size(11.5).color(p.weak).strong());
}

fn primary_button(ui: &mut egui::Ui, p: &Pal, text: &str, enabled: bool) -> egui::Response {
    let fill = if enabled { p.accent } else { p.border };
    let size = Vec2::new(ui.available_width(), 44.0);
    ui.add_enabled_ui(enabled, |ui| {
        ui.add_sized(
            size,
            egui::Button::new(RichText::new(text).size(15.5).strong().color(Color32::WHITE))
                .fill(fill)
                .corner_radius(CornerRadius::same(10)),
        )
    })
    .inner
}

/// Case à cocher carrée, centrée verticalement dans sa ligne.
fn check_box(ui: &mut egui::Ui, p: &Pal, on: &mut bool) -> egui::Response {
    let (rect, mut resp) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let painter = ui.painter();
    let r = CornerRadius::same(6);
    if *on {
        painter.rect_filled(rect, r, p.accent);
        let a = rect.left_top() + Vec2::new(5.5, 11.5);
        let b = rect.left_top() + Vec2::new(9.5, 15.5);
        let c = rect.left_top() + Vec2::new(16.5, 7.0);
        let st = Stroke::new(2.4_f32, Color32::WHITE);
        painter.line_segment([a, b], st);
        painter.line_segment([b, c], st);
    } else {
        let stroke = if resp.hovered() { p.accent } else { p.weak.gamma_multiply(0.7) };
        painter.rect(rect, r, p.inner, Stroke::new(1.6_f32, stroke), StrokeKind::Inside);
    }
    resp
}


fn reveal(path: &Path) {
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer").arg(format!("/select,{}", path.display())).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    if let Some(dir) = path.parent() {
        let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
    }
}

impl eframe::App for App {
    /// Fond de la fenêtre aux couleurs du thème (pas de liseré noir entre les panneaux).
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        visuals.panel_fill.to_normalized_gamma_f32()
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.apply_style(ctx);
        let p = Pal::new(self.dark);

        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if let Some(path) = dropped.first() {
            self.load(path);
        }
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());

        self.header(ctx, &p);
        if self.report.is_some() {
            self.side_panel(ctx, &p);
        }
        egui::CentralPanel::default()
            .frame(Frame::default().fill(p.bg).inner_margin(Margin::symmetric(24, 18)))
            .show(ctx, |ui| {
                self.update_banner(ui, &p);
                if self.report.is_some() {
                    self.report_view(ui, &p);
                } else {
                    self.empty_view(ui, &p);
                }
            });

        if hovering {
            let rect = ctx.screen_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("drop")));
            painter.rect_filled(rect, 0.0, Color32::from_black_alpha(170));
            let inner = rect.shrink(40.0);
            painter.rect_stroke(inner, CornerRadius::same(20), Stroke::new(3.0_f32, p.accent), StrokeKind::Inside);
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Relâche pour analyser le fichier",
                FontId::proportional(28.0),
                Color32::WHITE,
            );
        }
    }
}

impl App {
    fn header(&mut self, ctx: &egui::Context, p: &Pal) {
        egui::TopBottomPanel::top("header")
            .show_separator_line(false)
            .frame(
                Frame::default()
                    .fill(p.card)
                    .inner_margin(Margin::symmetric(22, 12))
                    .stroke(Stroke::new(1.0_f32, p.border)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Lyra").size(18.0).strong());
                            ui.label(RichText::new(format!("v{}", updater::VERSION)).size(12.0).color(p.weak));
                        });
                        ui.label(RichText::new("Analyse et nettoyage des métadonnées cachées").size(12.0).color(p.weak));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let checking = matches!(self.upd_state(), Upd::Checking { .. } | Upd::Downloading(_));
                        if ui
                            .add_enabled(!checking, egui::Button::new("⟳  Mises à jour"))
                            .on_hover_text("Vérifier si une nouvelle version est publiée sur GitHub")
                            .clicked()
                        {
                            self.check_updates(ui.ctx(), true);
                        }
                        let icon = if self.dark { "☀  Clair" } else { "🌙  Sombre" };
                        if ui.button(icon).clicked() {
                            self.dark = !self.dark;
                        }
                        if self.report.is_some() && ui.button("📂  Ouvrir un fichier").clicked() {
                            self.pick();
                        }
                    });
                });
            });
    }

    fn empty_view(&mut self, ui: &mut egui::Ui, p: &Pal) {
        let avail = ui.available_size();
        ui.add_space((avail.y * 0.12).max(10.0));
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("Que révèle ton document ?").size(30.0).strong());
            ui.add_space(2.0);
            ui.label(
                RichText::new("Auteur, GPS, logiciel, commentaires, historique… Découvre ce qui se cache dans tes fichiers, puis supprime-le.")
                    .size(15.0)
                    .color(p.weak),
            );
            ui.add_space(22.0);

            let size = Vec2::new(avail.x.min(620.0), 230.0);
            let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
            let hovered = resp.hovered();
            let painter = ui.painter();
            painter.rect_filled(rect, CornerRadius::same(18), if hovered { p.card } else { p.inner });
            painter.rect_stroke(
                rect,
                CornerRadius::same(18),
                Stroke::new(if hovered { 2.0_f32 } else { 1.5_f32 }, if hovered { p.accent } else { p.border }),
                StrokeKind::Inside,
            );
            let c = rect.center();
            let icon_rect = egui::Rect::from_center_size(c - Vec2::new(0.0, 48.0), Vec2::splat(60.0));
            painter.rect_filled(icon_rect, CornerRadius::same(16), p.accent.gamma_multiply(0.18));
            painter.text(icon_rect.center(), egui::Align2::CENTER_CENTER, "⬆", FontId::proportional(28.0), p.accent);
            painter.text(
                c + Vec2::new(0.0, 14.0),
                egui::Align2::CENTER_CENTER,
                "Glisse un fichier ici",
                FontId::proportional(19.0),
                p.text,
            );
            painter.text(
                c + Vec2::new(0.0, 42.0),
                egui::Align2::CENTER_CENTER,
                "ou clique pour le choisir sur ton ordinateur",
                FontId::proportional(14.0),
                p.weak,
            );
            painter.text(
                c + Vec2::new(0.0, 84.0),
                egui::Align2::CENTER_CENTER,
                "JPEG · PNG · PDF · Word · Excel · PowerPoint · LibreOffice",
                FontId::proportional(12.5),
                p.weak,
            );
            if resp.clicked() {
                self.pick();
            }
            if hovered {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }

            if let Some(e) = &self.error {
                ui.add_space(14.0);
                ui.label(RichText::new(format!("⚠  {e}")).color(risk_color(Risk::High)));
            }

            ui.add_space(34.0);
            let w = avail.x.min(760.0);
            ui.allocate_ui(Vec2::new(w, 120.0), |ui| {
                ui.columns(3, |cols| {
                    let steps = [
                        ("1", "Analyse", "Le fichier est lu localement, rien n'est envoyé sur Internet."),
                        ("2", "Choisis", "Chaque catégorie est expliquée avec son niveau de risque."),
                        ("3", "Nettoie", "Une copie propre est créée, l'original reste intact."),
                    ];
                    for (i, (n, t, d)) in steps.iter().enumerate() {
                        card(p).show(&mut cols[i], |ui| {
                            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                                ui.horizontal(|ui| {
                                    pill(ui, n, p.accent);
                                    ui.label(RichText::new(*t).strong());
                                });
                                ui.add(egui::Label::new(RichText::new(*d).size(12.5).color(p.weak)).wrap());
                            });
                        });
                    }
                });
            });
        });
    }

    fn side_panel(&mut self, ctx: &egui::Context, p: &Pal) {
        egui::SidePanel::left("side")
            .show_separator_line(false)
            .resizable(false)
            .exact_width(330.0)
            .frame(
                Frame::default()
                    .fill(p.card)
                    .inner_margin(Margin::symmetric(20, 18))
                    .stroke(Stroke::new(1.0_f32, p.border)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
                    let Some(r) = self.report.clone() else { return };

                    // Fichier
                    ui.horizontal(|ui| {
                        let (rect, _) = ui.allocate_exact_size(Vec2::new(52.0, 52.0), Sense::hover());
                        ui.painter().rect_filled(rect, CornerRadius::same(12), p.accent.gamma_multiply(0.18));
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            r.kind.badge(),
                            FontId::proportional(13.0),
                            p.accent,
                        );
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.add(egui::Label::new(RichText::new(r.file_name()).strong().size(15.0)).truncate())
                                .on_hover_text(r.path.display().to_string());
                            ui.label(RichText::new(format!("{} · {}", r.kind.label(), human_size(r.size))).size(12.5).color(p.weak));
                        });
                    });

                    ui.add_space(10.0);
                    // Exposition
                    Frame::default()
                        .fill(p.inner)
                        .corner_radius(CornerRadius::same(10))
                        .inner_margin(Margin::same(12))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Niveau d'exposition").color(p.weak));
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| match r.max_risk() {
                                    Some(risk) => pill(ui, risk.label(), risk_color(risk)),
                                    None => pill(ui, "Aucun", p.success),
                                });
                            });
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(r.total_entries().to_string()).size(26.0).strong());
                                ui.label(RichText::new(format!("élément(s) dans {} catégorie(s)", r.groups.len())).color(p.weak));
                            });
                            ui.horizontal(|ui| {
                                for risk in [Risk::High, Risk::Medium, Risk::Low] {
                                    let n = r.count_risk(risk);
                                    let (dot, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                                    ui.painter().circle_filled(dot.center(), 4.0, risk_color(risk));
                                    ui.label(RichText::new(format!("{} {}", n, risk.label().to_lowercase())).size(12.5).color(p.weak));
                                }
                            });
                        });

                    if r.groups.iter().any(|g| g.removable) {
                        ui.add_space(12.0);
                        section_title(ui, p, "Sélection");
                        ui.horizontal(|ui| {
                            if ui.button("Recommandée").on_hover_text("Tout ce qui est sensible, sans toucher au rendu ni au contenu").clicked() {
                                self.selected = r.groups.iter().filter(|g| g.removable && g.recommended).map(|g| g.id.clone()).collect();
                            }
                            if ui.button("Tout").clicked() {
                                self.selected = r.groups.iter().filter(|g| g.removable).map(|g| g.id.clone()).collect();
                            }
                            if ui.button("Rien").clicked() {
                                self.selected.clear();
                            }
                        });
                        ui.label(
                            RichText::new({
                                let (c, e) = (self.selected.len(), self.selected_entries());
                                format!(
                                    "{c} catégorie{} · {e} élément{} à supprimer",
                                    if c > 1 { "s" } else { "" },
                                    if e > 1 { "s" } else { "" }
                                )
                            })
                            .size(12.5)
                            .color(p.weak),
                        );

                        ui.add_space(8.0);
                        section_title(ui, p, "Enregistrement");
                        ui.radio_value(&mut self.output, Output::Copy, "Créer une copie nettoyée")
                            .on_hover_text("Le fichier « nom_nettoye » est créé à côté de l'original");
                        let custom = matches!(self.output, Output::Custom(_));
                        if ui.radio(custom, "Choisir l'emplacement…").clicked() {
                            let ext = r.path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
                            let def = formats::default_output(&r.path);
                            let mut dlg = rfd::FileDialog::new()
                                .set_title("Enregistrer le fichier nettoyé")
                                .set_file_name(def.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                            if let Some(dir) = r.path.parent() {
                                dlg = dlg.set_directory(dir);
                            }
                            if !ext.is_empty() {
                                dlg = dlg.add_filter(ext.to_uppercase(), &[ext.as_str()]);
                            }
                            if let Some(path) = dlg.save_file() {
                                self.output = Output::Custom(Some(path));
                            }
                        }
                        if let Output::Custom(Some(path)) = &self.output {
                            ui.indent("custom_path", |ui| {
                                ui.add(egui::Label::new(RichText::new(path.display().to_string()).size(12.0).color(p.weak)).wrap());
                            });
                        }
                        ui.radio_value(&mut self.output, Output::Replace, "Remplacer l'original")
                            .on_hover_text("Une confirmation sera demandée");

                        ui.add_space(14.0);
                        let label = format!("Nettoyer la sélection ({})", self.selected.len());
                        if primary_button(ui, p, &label, !self.selected.is_empty()).clicked() {
                            self.clean();
                        }
                    }

                    if let Some(e) = &self.error {
                        ui.add_space(8.0);
                        ui.label(RichText::new(format!("⚠  {e}")).color(risk_color(Risk::High)));
                    }

                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(2.0);
                    let gap = ui.spacing().item_spacing.x;
                    let bw = ((ui.available_width() - 2.0 * gap) / 3.0).floor();
                    ui.horizontal(|ui| {
                        if ui
                            .add_sized(Vec2::new(bw, 34.0), egui::Button::new("Rapport"))
                            .on_hover_text("Exporter le rapport en fichier texte")
                            .clicked()
                        {
                            self.export_report();
                        }
                        if ui
                            .add_sized(Vec2::new(bw, 34.0), egui::Button::new("Relancer"))
                            .on_hover_text("Analyser à nouveau le fichier")
                            .clicked()
                        {
                            let path = r.path.clone();
                            self.load(&path);
                        }
                        if ui
                            .add_sized(Vec2::new(bw, 34.0), egui::Button::new("Fermer"))
                            .on_hover_text("Fermer le fichier")
                            .clicked()
                        {
                            self.report = None;
                            self.done = None;
                            self.error = None;
                        }
                    });
                });
            });
    }

    fn report_view(&mut self, ui: &mut egui::Ui, p: &Pal) {
        let Some(r) = self.report.clone() else { return };

        if let Some(done) = &self.done {
            let mut close = false;
            let mut open_clean: Option<PathBuf> = None;
            Frame::default()
                .fill(p.success.gamma_multiply(0.10))
                .stroke(Stroke::new(1.0_f32, p.success.gamma_multiply(0.7)))
                .corner_radius(CornerRadius::same(12))
                .inner_margin(Margin::same(16))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("✔  Nettoyage terminé").size(17.0).strong().color(p.success));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.add_sized(Vec2::new(30.0, 30.0), egui::Button::new("✖")).on_hover_text("Masquer").clicked() {
                                close = true;
                            }
                        });
                    });
                    ui.label(format!(
                        "{} élément(s) supprimé(s). Vérification du nouveau fichier : {} élément(s) restant(s).",
                        done.removed, done.remaining
                    ));
                    ui.add(egui::Label::new(RichText::new(done.output.display().to_string()).size(12.5).color(p.weak)).wrap());
                    for n in &done.notes {
                        ui.label(RichText::new(format!("•  {n}")).size(12.5).color(p.weak));
                    }
                    ui.horizontal(|ui| {
                        if ui.button("📂  Ouvrir le dossier").clicked() {
                            reveal(&done.output);
                        }
                        if ui.button("🔍  Analyser le fichier nettoyé").clicked() {
                            open_clean = Some(done.output.clone());
                        }
                    });
                });
            if close {
                self.done = None;
            }
            if let Some(path) = open_clean {
                self.load(&path);
                return;
            }
            ui.add_space(12.0);
        }

        ui.horizontal(|ui| {
            ui.label(RichText::new("Métadonnées détectées").size(22.0).strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_sized(
                    Vec2::new(260.0, 30.0),
                    egui::TextEdit::singleline(&mut self.filter)
                        .hint_text("🔍  Rechercher (nom, GPS, date…)")
                        .vertical_align(Align::Center),
                );
                if !r.groups.is_empty() {
                    if ui.add_sized(Vec2::new(110.0, 30.0), egui::Button::new("Tout replier")).clicked() {
                        self.expanded.clear();
                    }
                    if ui.add_sized(Vec2::new(110.0, 30.0), egui::Button::new("Tout déplier")).clicked() {
                        self.expanded = r.groups.iter().map(|g| g.id.clone()).collect();
                    }
                }
            });
        });
        ui.label(
            RichText::new("Coche les catégories à supprimer. Clique sur une catégorie pour voir le détail de chaque élément.")
                .color(p.weak),
        );
        ui.add_space(8.0);

        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            if r.groups.is_empty() {
                card(p).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.vertical_centered(|ui| {
                        ui.add_space(20.0);
                        ui.label(RichText::new("✔").size(40.0).color(p.success));
                        ui.label(RichText::new("Aucune métadonnée cachée détectée").size(18.0).strong());
                        ui.label(RichText::new("Ce fichier est propre.").color(p.weak));
                        ui.add_space(20.0);
                    });
                });
            }
            let q = self.filter.trim().to_lowercase();
            let mut shown = 0;
            for g in &r.groups {
                let matches: Vec<usize> = (0..g.entries.len())
                    .filter(|&i| {
                        q.is_empty()
                            || g.entries[i].key.to_lowercase().contains(&q)
                            || g.entries[i].value.to_lowercase().contains(&q)
                    })
                    .collect();
                let title_match = !q.is_empty() && (g.title.to_lowercase().contains(&q) || g.description.to_lowercase().contains(&q));
                if !q.is_empty() && matches.is_empty() && !title_match {
                    continue;
                }
                shown += 1;
                let visible: Vec<usize> = if title_match && matches.is_empty() { (0..g.entries.len()).collect() } else { matches };
                group_card(ui, p, g, &visible, !q.is_empty(), &mut self.selected, &mut self.expanded);
                ui.add_space(10.0);
            }
            if shown == 0 && !r.groups.is_empty() {
                ui.label(RichText::new("Aucun résultat pour cette recherche.").color(p.weak));
            }
            if !r.warnings.is_empty() {
                card(p).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("⚠  À savoir").strong().color(risk_color(Risk::Medium)));
                    for w in &r.warnings {
                        ui.add(egui::Label::new(RichText::new(w).color(p.weak)).wrap());
                    }
                });
            }
        });
    }
}

fn group_card(
    ui: &mut egui::Ui,
    p: &Pal,
    g: &Group,
    visible: &[usize],
    force_open: bool,
    selected: &mut HashSet<String>,
    expanded: &mut HashSet<String>,
) {
    let is_sel = selected.contains(&g.id);
    let border = if is_sel { p.accent.gamma_multiply(0.8) } else { p.border };
    Frame::default()
        .fill(p.card)
        .stroke(Stroke::new(if is_sel { 1.5_f32 } else { 1.0_f32 }, border))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                // Bande de couleur du risque
                let (bar, _) = ui.allocate_exact_size(Vec2::new(4.0, 40.0), Sense::hover());
                ui.painter().rect_filled(bar, CornerRadius::same(2), risk_color(g.risk));

                if g.removable {
                    let mut on = is_sel;
                    if check_box(ui, p, &mut on).on_hover_text("Supprimer cette catégorie").changed() {
                        if on {
                            selected.insert(g.id.clone());
                        } else {
                            selected.remove(&g.id);
                        }
                    }
                } else {
                    ui.add_space(26.0);
                }
                let right_w = 190.0;
                ui.vertical(|ui| {
                    ui.set_max_width((ui.available_width() - right_w).max(120.0));
                    ui.spacing_mut().item_spacing.y = 3.0;
                    ui.label(RichText::new(&g.title).size(15.5).strong());
                    ui.add(egui::Label::new(RichText::new(&g.description).size(12.5).color(p.weak)).wrap());
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    pill(ui, &format!("Risque {}", g.risk.label().to_lowercase()), risk_color(g.risk));
                    if !g.removable {
                        pill(ui, "Info", p.weak);
                    }
                    ui.label(RichText::new(format!("{}", g.entries.len())).strong().color(p.weak))
                        .on_hover_text("Nombre d'éléments");
                });
            });

            if let Some(n) = &g.note {
                ui.horizontal(|ui| {
                    ui.add_space(42.0);
                    ui.add(egui::Label::new(RichText::new(format!("⚠  {n}")).size(12.5).color(risk_color(Risk::Medium))).wrap());
                });
            }

            if g.entries.is_empty() {
                return;
            }
            let open = force_open || expanded.contains(&g.id);
            ui.horizontal(|ui| {
                ui.add_space(38.0);
                let txt = if open {
                    "−  Masquer le détail".to_string()
                } else {
                    format!("+  Voir le détail ({} élément{})", g.entries.len(), if g.entries.len() > 1 { "s" } else { "" })
                };
                let resp = ui.add(egui::Label::new(RichText::new(txt).size(13.0).color(p.accent)).sense(Sense::click()));
                if resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if resp.clicked() && !force_open {
                    if open {
                        expanded.remove(&g.id);
                    } else {
                        expanded.insert(g.id.clone());
                    }
                }
            });
            if open {
                ui.horizontal(|ui| {
                    ui.add_space(38.0);
                    Frame::default()
                        .fill(p.inner)
                        .corner_radius(CornerRadius::same(8))
                        .inner_margin(Margin::symmetric(12, 8))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.vertical(|ui| {
                            let key_w = (ui.available_width() * 0.34).clamp(140.0, 280.0);
                            for (n, &i) in visible.iter().enumerate() {
                                let e = &g.entries[i];
                                if n > 0 {
                                    ui.painter().hline(
                                        ui.max_rect().x_range(),
                                        ui.cursor().top() - 4.0,
                                        Stroke::new(1.0_f32, p.border.gamma_multiply(0.6)),
                                    );
                                }
                                ui.horizontal_top(|ui| {
                                    ui.allocate_ui_with_layout(Vec2::new(key_w, 0.0), Layout::top_down(Align::Min), |ui| {
                                        ui.set_width(key_w);
                                        ui.add(egui::Label::new(RichText::new(&e.key).size(13.0).color(p.weak)).wrap());
                                    });
                                    ui.vertical(|ui| {
                                        ui.add(
                                            egui::Label::new(RichText::new(&e.value).size(13.5))
                                                .wrap()
                                                .sense(Sense::click()),
                                        )
                                        .on_hover_text("Clic droit pour copier")
                                        .context_menu(|ui| {
                                            if ui.button("Copier la valeur").clicked() {
                                                ui.ctx().copy_text(e.value.clone());
                                                ui.close_menu();
                                            }
                                        });
                                        if let Some(link) = &e.link {
                                            ui.hyperlink_to(RichText::new("📍  Voir sur la carte").size(12.5), link);
                                        }
                                    });
                                });
                            }
                            });
                        });
                });
            }
        });
}

