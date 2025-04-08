use crate::clients::{Event, EventGroup, EventKind};
use chrono::{TimeZone, Utc};
use egui::{Align, Color32, Layout, Stroke, Ui, Vec2, Widget, epaint::CornerRadiusF32};

/// Defines which side messages should appear on
#[derive(Clone, PartialEq, Default)]
pub enum MessageSide {
    #[default]
    Left,
    Right,
}

/// Style configuration for message bubbles
#[derive(Clone)]
pub struct MessageStyle {
    pub self_bg: Color32,
    pub other_bg: Color32,
    pub text_color: Color32,
    pub time_color: Color32,
    pub name_color: Color32,
    pub corner_radius: f32,
    pub avatar_size: f32,
    pub stroke: Stroke,
    pub group_spacing: f32,
    pub self_message_side: MessageSide,
    pub bubble_margin: Vec2,
}

impl Default for MessageStyle {
    fn default() -> Self {
        let visuals = egui::Visuals::default();
        let button_bg = visuals.widgets.inactive.bg_fill;
        let (self_bg, other_bg) = Self::calculate_backgrounds(button_bg, visuals.dark_mode);

        Self {
            self_bg,
            other_bg,
            text_color: visuals.text_color(),
            time_color: visuals.weak_text_color(),
            name_color: visuals.widgets.inactive.text_color(),
            corner_radius: 12.0,
            avatar_size: 32.0,
            stroke: Stroke::new(1.0, visuals.widgets.inactive.bg_stroke.color),
            group_spacing: 8.0,
            self_message_side: MessageSide::Left,
            bubble_margin: Vec2::new(8.0, 6.0),
        }
    }
}

impl MessageStyle {
    /// Calculate background colors for self and other messages based on theme
    fn calculate_backgrounds(base: Color32, dark_mode: bool) -> (Color32, Color32) {
        if dark_mode {
            (base.linear_multiply(0.8), base.linear_multiply(0.6))
        } else {
            (base.linear_multiply(1.2), base.linear_multiply(0.9))
        }
    }
}

/// Widget for rendering message bubbles
pub struct MessageWidget {
    style: MessageStyle,
    group: EventGroup,
}

impl MessageWidget {
    /// Create a new message widget with the given style and event group
    pub fn new(style: MessageStyle, group: EventGroup) -> Self {
        Self { style, group }
    }

    /// Display the message group in the UI
    pub fn show(&self, ui: &mut Ui) {
        let event_count = self.group.events.len();

        ui.vertical(|ui| {
            for (idx, event) in self.group.events.iter().enumerate() {
                let is_first = idx == 0;
                let is_last = idx == event_count - 1;

                self.render_message_row(ui, event, is_first, is_last);
            }
            ui.add_space(self.style.group_spacing);
        });
    }

    // Rendering methods ---------------------------------------------------

    fn render_message_row(&self, ui: &mut Ui, event: &Event, is_first: bool, is_last: bool) {
        ui.horizontal(|ui| {
            let side = if self.group.from_self {
                &self.style.self_message_side
            } else {
                &MessageSide::Left
            };

            match side {
                MessageSide::Right => self.render_right_aligned(ui, event, is_first, is_last),
                MessageSide::Left => self.render_left_aligned(ui, event, is_first, is_last),
            }
        });
    }

    fn render_right_aligned(&self, ui: &mut Ui, event: &Event, is_first: bool, is_last: bool) {
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            self.message_bubble(ui, event, is_first, is_last);
            if !self.group.from_self {
                ui.add_space(8.0);
            }
        });
    }

    fn render_left_aligned(&self, ui: &mut Ui, event: &Event, is_first: bool, is_last: bool) {
        if !self.group.from_self {
            if is_last {
                self.render_avatar(ui);
            } else {
                ui.add_space(self.style.avatar_size + 8.0);
            }
        }
        self.message_bubble(ui, event, is_first, is_last);
    }

    fn message_bubble(
        &self,
        ui: &mut Ui,
        event: &Event,
        is_first: bool,
        is_last: bool,
    ) -> egui::Response {
        let bg_color = if self.group.from_self {
            self.style.self_bg
        } else {
            self.style.other_bg
        };

        let rounding = self.calculate_bubble_rounding(is_first, is_last);

        egui::Frame::new()
            .fill(bg_color)
            .inner_margin(self.style.bubble_margin)
            .corner_radius(rounding)
            .stroke(self.style.stroke)
            .show(ui, |ui| self.render_bubble_content(ui, event, is_first))
            .response
    }

    fn render_bubble_content(&self, ui: &mut Ui, event: &Event, is_first: bool) {
        ui.vertical(|ui| {
            if !self.group.from_self && is_first {
                self.render_username(ui);
            }

            match &event.kind {
                EventKind::Message(content) => {
                    ui.label(egui::RichText::new(content).color(self.style.text_color));
                }
            }

            self.render_timestamp(ui, event.timestamp);
        });
    }

    fn render_username(&self, ui: &mut Ui) {
        ui.label(
            egui::RichText::new(&self.group.display_name)
                .color(self.style.name_color)
                .size(12.0),
        );
    }

    fn render_timestamp(&self, ui: &mut Ui, timestamp: u64) {
        ui.horizontal(|ui| {
            ui.add_space(ui.available_width() - 40.0);
            ui.label(
                egui::RichText::new(format_time(timestamp))
                    .color(self.style.time_color)
                    .size(10.0),
            );
        });
    }

    fn render_avatar(&self, ui: &mut Ui) {
        if let Some(avatar) = &self.group.avatar {
            let size = self.style.avatar_size;
            egui::Image::from_bytes(
                format!("user-avatar-{}", self.group.user_id),
                avatar.clone(),
            )
            .fit_to_exact_size(Vec2::splat(size))
            .corner_radius(size / 2.0)
            .ui(ui);
        } else {
            ui.add_space(self.style.avatar_size + 8.0);
        }
    }

    fn calculate_bubble_rounding(&self, is_first: bool, is_last: bool) -> CornerRadiusF32 {
        let is_only = is_first && is_last;
        let radius = self.style.corner_radius;
        let small_radius = 2.0;

        let (nw, ne, sw, se) = (
            if is_first || is_only {
                radius
            } else {
                small_radius
            },
            radius,
            if is_last || is_only {
                radius
            } else {
                small_radius
            },
            radius,
        );

        if self.group.from_self && self.style.self_message_side == MessageSide::Right {
            CornerRadiusF32 {
                nw: ne,
                ne: nw,
                sw: se,
                se: sw,
            }
        } else {
            CornerRadiusF32 { nw, ne, sw, se }
        }
    }
}

/// Format timestamp as HH:MM
fn format_time(timestamp: u64) -> String {
    Utc.timestamp_opt(timestamp as i64, 0)
        .unwrap()
        .format("%H:%M")
        .to_string()
}
