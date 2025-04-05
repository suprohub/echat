use std::{borrow::Cow, sync::Arc};

use crate::clients::{Event, EventGroup, EventKind};
use chrono::{TimeZone, Utc};
use egui::{Align, Color32, Layout, Stroke, Ui, Vec2, Widget, epaint::CornerRadiusF32};

#[derive(Clone, PartialEq, Default)]
pub enum MessageSide {
    #[default]
    Left,
    Right,
}

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

        let (self_bg, other_bg) = if visuals.dark_mode {
            let self_bg = button_bg.linear_multiply(0.8);
            let other_bg = button_bg.linear_multiply(0.6);
            (self_bg, other_bg)
        } else {
            let self_bg = button_bg.linear_multiply(1.2);
            let other_bg = button_bg.linear_multiply(0.9);
            (self_bg, other_bg)
        };

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

pub struct MessageWidget {
    style: MessageStyle,
    group: EventGroup,
    current_user_id: String,
}

impl MessageWidget {
    pub fn new(style: MessageStyle, group: EventGroup, current_user_id: String) -> Self {
        Self {
            style,
            group,
            current_user_id,
        }
    }

    pub fn show(&self, ui: &mut Ui) {
        let is_self = self.group.user_id == self.current_user_id;
        let has_avatar = self.group.avatar.is_some();
        let avatar_size = self.style.avatar_size;

        ui.vertical(|ui| {
            for (idx, event) in self.group.events.iter().enumerate() {
                let is_first = idx == 0;
                let is_last = idx == self.group.events.len() - 1;
                let is_only = is_first && is_last;

                ui.horizontal(|ui| {
                    let message_side = if is_self {
                        self.style.self_message_side.clone()
                    } else {
                        MessageSide::Left
                    };

                    match message_side {
                        MessageSide::Right => {
                            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                                self.message_bubble(ui, event, is_first, is_last, is_only, is_self);
                                if !is_self && (is_last || has_avatar) {
                                    ui.add_space(8.0);
                                }
                            });
                        }
                        MessageSide::Left => {
                            if !is_self {
                                if is_last && has_avatar {
                                    self.avatar(ui, self.group.avatar.clone().unwrap());
                                } else {
                                    ui.add_space(avatar_size + 8.0);
                                }
                            }
                            self.message_bubble(ui, event, is_first, is_last, is_only, is_self);
                        }
                    }
                });
            }
        });

        ui.add_space(self.style.group_spacing);
    }

    fn message_bubble(
        &self,
        ui: &mut Ui,
        event: &Event,
        is_first: bool,
        is_last: bool,
        is_only: bool,
        is_self: bool,
    ) -> egui::Response {
        let style = &self.style;
        let bg_color = if is_self {
            style.self_bg
        } else {
            style.other_bg
        };

        let mut rounding = CornerRadiusF32 {
            nw: if is_first || is_only {
                style.corner_radius
            } else {
                2.0
            },
            ne: style.corner_radius,
            sw: if is_last || is_only {
                style.corner_radius
            } else {
                2.0
            },
            se: style.corner_radius,
        };

        if is_self && style.self_message_side == MessageSide::Right {
            rounding = CornerRadiusF32 {
                nw: rounding.ne,
                ne: rounding.nw,
                sw: rounding.se,
                se: rounding.sw,
            };
        }

        let frame = egui::Frame::new()
            .fill(bg_color)
            .inner_margin(style.bubble_margin)
            .corner_radius(rounding)
            .stroke(style.stroke);

        frame
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    if is_first && !is_self {
                        ui.label(
                            egui::RichText::new(&self.group.display_name)
                                .color(style.name_color)
                                .size(12.0),
                        );
                    }

                    match &event.kind {
                        EventKind::Message(content) => {
                            ui.label(egui::RichText::new(content).color(style.text_color));
                        }
                    }

                    ui.horizontal(|ui| {
                        ui.add_space(ui.available_width() - 40.0);
                        ui.label(
                            egui::RichText::new(format_time(event.timestamp))
                                .color(style.time_color)
                                .size(10.0),
                        );
                    });
                })
            })
            .response
    }

    fn avatar(&self, ui: &mut Ui, avatar: Arc<[u8]>) {
        let size = self.style.avatar_size;
        egui::Image::new((Cow::default(), avatar))
            .corner_radius(CornerRadiusF32::same(size / 2.0))
            .fit_to_exact_size(Vec2::splat(size))
            .ui(ui);
    }
}

fn format_time(timestamp: u64) -> String {
    let dt = Utc.timestamp_opt(timestamp as i64, 0).unwrap();
    dt.format("%H:%M").to_string()
}
