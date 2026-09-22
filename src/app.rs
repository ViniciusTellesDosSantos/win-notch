use egui::{Pos2, Rect, Vec2};

use crate::config::Settings;
use crate::notch::shape;
use crate::notch::{Notch, NotchState};
use crate::usage::{UsageStatus, UsageWatcher};
use crate::{autostart, screenshot, tray};

/// Fallback monitor size used until egui reports the real one for the monitor the notch
/// is on. Known limitation: monitor position is assumed to start at (0, 0), so edge
/// snapping on a secondary monitor that isn't at the origin will be off — fine for the
/// common single-monitor case this v1 targets.
const FALLBACK_MONITOR_SIZE: Vec2 = Vec2::new(1920.0, 1080.0);

pub struct App {
    notch: Notch,
    settings: Settings,
    usage: UsageWatcher,
    tray: tray::Tray,
    last_window_pos: Pos2,
    last_window_size: Vec2,
    screenshot_message: Option<String>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings = Settings::load();
        let notch = Notch::new(settings.edge, settings.offset_along_edge);
        let usage = UsageWatcher::spawn(crate::usage::claude_code::default_projects_dir());
        let tray = tray::Tray::new(settings.start_with_windows);

        let initial_size = notch.current_size();
        let initial_pos =
            notch.window_position(Rect::from_min_size(Pos2::ZERO, FALLBACK_MONITOR_SIZE));
        cc.egui_ctx
            .send_viewport_cmd(egui::ViewportCommand::OuterPosition(initial_pos));
        cc.egui_ctx
            .send_viewport_cmd(egui::ViewportCommand::InnerSize(initial_size));

        Self {
            notch,
            settings,
            usage,
            tray,
            last_window_pos: initial_pos,
            last_window_size: initial_size,
            screenshot_message: None,
        }
    }

    fn monitor_rect(&self, ctx: &egui::Context) -> Rect {
        let size = ctx
            .input(|i| i.viewport().monitor_size)
            .unwrap_or(FALLBACK_MONITOR_SIZE);
        Rect::from_min_size(Pos2::ZERO, size)
    }

    fn handle_tray_events(&mut self) {
        while let Some(event) = self.tray.poll_event() {
            match event {
                tray::TrayEvent::ToggleAutostart => {
                    let enabled = !self.settings.start_with_windows;
                    match autostart::set_enabled(enabled) {
                        Ok(()) => {
                            self.settings.start_with_windows = enabled;
                            self.tray.set_autostart_checked(enabled);
                            let _ = self.settings.save();
                        }
                        Err(err) => log::warn!("falha ao alternar autostart: {err}"),
                    }
                }
                tray::TrayEvent::Quit => std::process::exit(0),
            }
        }
    }

    fn apply_window_geometry(&mut self, ctx: &egui::Context, monitor: Rect) {
        let size = self.notch.current_size();
        let pos = self.notch.window_position(monitor);

        if size != self.last_window_size {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            self.last_window_size = size;
        }
        if pos != self.last_window_pos {
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
            self.last_window_pos = pos;
        }
    }

    fn draw_collapsed(&self, ui: &mut egui::Ui, rect: Rect) {
        let radius = shape::pill_corner_radius(rect.size());
        shape::paint_panel(ui.painter(), rect, radius);

        let (dot_color, _label) = usage_indicator(&self.usage.snapshot().status);
        let center = rect.left_center() + egui::vec2(14.0, 0.0);
        ui.painter().circle_filled(center, 4.0, dot_color);
        ui.painter().text(
            rect.center() + egui::vec2(6.0, 0.0),
            egui::Align2::CENTER_CENTER,
            "win-notch",
            egui::FontId::proportional(11.0),
            shape::MUTED_TEXT,
        );
    }

    fn draw_expanded(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, rect: Rect) {
        let radius = 18.0;
        shape::paint_panel(ui.painter(), rect, radius);

        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(rect.shrink(14.0)), |ui| {
            ui.label(
                egui::RichText::new("Claude Code")
                    .strong()
                    .color(shape::MUTED_TEXT),
            );
            self.draw_usage_section(ui);

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(6.0);

            if ui.button("📷  Capturar região").clicked() {
                match screenshot::capture_region_to_clipboard(ctx) {
                    Ok(true) => {
                        self.screenshot_message =
                            Some("Copiado para a área de transferência".into())
                    }
                    Ok(false) => self.screenshot_message = None,
                    Err(err) => self.screenshot_message = Some(format!("Erro: {err}")),
                }
            }

            if let Some(message) = &self.screenshot_message {
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(message)
                        .small()
                        .color(shape::MUTED_TEXT),
                );
            }

            ui.add_space(8.0);
            ui.label(
                egui::RichText::new("Agenda: em breve")
                    .small()
                    .color(shape::MUTED_TEXT),
            );
        });
    }

    fn draw_usage_section(&self, ui: &mut egui::Ui) {
        let snapshot = self.usage.snapshot();
        match snapshot.status {
            UsageStatus::Loading => {
                ui.label(egui::RichText::new("Carregando uso local…").color(shape::MUTED_TEXT));
            }
            UsageStatus::Unavailable(reason) => {
                ui.label(egui::RichText::new(reason).color(shape::MUTED_TEXT));
            }
            UsageStatus::Idle => {
                ui.label(
                    egui::RichText::new("Sem sessão ativa nas últimas 5h").color(shape::MUTED_TEXT),
                );
            }
            UsageStatus::Active {
                tokens,
                started_at,
                resets_at,
            } => {
                let remaining = resets_at - chrono::Utc::now();
                let minutes = remaining.num_minutes().max(0);
                ui.horizontal(|ui| {
                    ui.label(format!("{tokens} tokens"));
                    ui.label(
                        egui::RichText::new(format!(
                            "· reinicia em {}h{:02}min",
                            minutes / 60,
                            minutes % 60
                        ))
                        .color(shape::MUTED_TEXT),
                    );
                });
                ui.label(
                    egui::RichText::new(format!(
                        "Janela iniciada às {} · estimativa derivada dos logs locais, não é o limite oficial do plano",
                        started_at.with_timezone(&chrono::Local).format("%H:%M")
                    ))
                    .small()
                    .color(shape::MUTED_TEXT),
                );
            }
        }

        let age_secs = (chrono::Utc::now() - snapshot.last_updated)
            .num_seconds()
            .max(0);
        ui.label(
            egui::RichText::new(format!("atualizado há {age_secs}s"))
                .small()
                .color(shape::MUTED_TEXT),
        );
    }
}

fn usage_indicator(status: &UsageStatus) -> (egui::Color32, &'static str) {
    match status {
        UsageStatus::Loading => (shape::MUTED_TEXT, "carregando"),
        UsageStatus::Unavailable(_) => (shape::MUTED_TEXT, "indisponível"),
        UsageStatus::Idle => (shape::MUTED_TEXT, "ocioso"),
        UsageStatus::Active { .. } => (shape::ACCENT_COLOR, "ativo"),
    }
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::TRANSPARENT.to_array()
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_tray_events();

        let monitor = self.monitor_rect(ctx);

        let panel = egui::CentralPanel::default().frame(egui::Frame::none());
        let response = panel
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                let interact = ui.interact(
                    rect,
                    ui.id().with("notch-drag"),
                    egui::Sense::click_and_drag(),
                );

                if interact.drag_started() {
                    self.notch.begin_drag();
                }
                if interact.dragged() {
                    if let Some(local_pos) = interact.interact_pointer_pos() {
                        let screen_pos = self.last_window_pos + local_pos.to_vec2();
                        self.notch.drag_to(monitor, screen_pos);
                    }
                }
                if interact.drag_stopped() {
                    self.notch.end_drag();
                    self.settings.edge = self.notch.edge;
                    self.settings.offset_along_edge = self.notch.offset_along_edge;
                    let _ = self.settings.save();
                }

                match self.notch.state {
                    NotchState::Collapsed => self.draw_collapsed(ui, rect),
                    NotchState::Expanded => self.draw_expanded(ctx, ui, rect),
                }

                interact.hovered()
            })
            .inner;

        if self.notch.update_hover(response) {
            // State just changed (collapsed <-> expanded); geometry below picks it up.
        }

        self.apply_window_geometry(ctx, monitor);
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}
