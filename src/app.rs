use egui::UiBuilder;
use parking_lot::Mutex;
use std::{borrow::Cow, sync::Arc};
use tokio::runtime::Runtime;

use crate::{
    clients::{Chat, LoginOption, matrix::MatrixClient},
    message::{MessageStyle, MessageWidget},
};

/// Main application state for the EChat app
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct EChat {
    #[serde(skip)]
    rt: Runtime,
    #[serde(skip)]
    client: LoginOption,
    chats: Arc<Mutex<Vec<Chat>>>,
}

impl Default for EChat {
    fn default() -> Self {
        // Create appropriate runtime based on target architecture
        #[cfg(not(target_arch = "wasm32"))]
        let rt = Runtime::new().expect("Failed to create Tokio runtime");

        #[cfg(target_arch = "wasm32")]
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("Failed to create wasm Tokio runtime");

        Self {
            client: Default::default(),
            chats: Default::default(),
            rt,
        }
    }
}

impl EChat {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Enable image loading for avatars
        egui_extras::install_image_loaders(&cc.egui_ctx);

        if let Some(storage) = cc.storage {
            // Load app state from storage if available
            let mut echat: EChat = eframe::get_value(storage, eframe::APP_KEY).unwrap_or_default();

            // Load matrix client from storage
            echat.client = MatrixClient::load_from_storage(storage);

            // If user is logged in, sync with server and get chats
            if let LoginOption::LoggedIn(client) = &echat.client {
                let client = client.clone();
                let chats = echat.chats.clone();

                echat.rt.spawn(async move {
                    if let Err(e) = client.sync().await {
                        log::error!("Failed to sync with server: {}", e);
                        return;
                    }

                    match client.chats().await {
                        Ok(client_chats) => {
                            *chats.lock() = client_chats;
                        }
                        Err(e) => {
                            log::error!("Failed to fetch chats: {}", e);
                        }
                    }
                });
            }

            echat
        } else {
            Default::default()
        }
    }
}

impl eframe::App for EChat {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, eframe::APP_KEY, self);

        if let LoginOption::LoggedIn(client) = &self.client {
            if let Err(e) = client.save(storage) {
                log::error!("Failed to save client state: {}", e);
            }
        }
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Handle app close
        if ctx.input(|i| i.viewport().close_requested()) {
            if let Some(storage) = frame.storage_mut() {
                self.save(storage);
            }
            std::process::exit(0); // TODO: Implement cleaner shutdown
        }

        match &mut self.client {
            LoginOption::Auth { username, password } => {
                let mut try_login = false;

                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.heading("Login to Matrix");
                        ui.add_space(10.0);

                        ui.label("Username:");
                        ui.text_edit_singleline(username);

                        ui.add_space(5.0);

                        ui.label("Password:");
                        let password_edit = egui::TextEdit::singleline(password)
                            .password(true)
                            .hint_text("Enter your password");
                        ui.add(password_edit);

                        ui.add_space(10.0);

                        if ui.button("Login").clicked() {
                            try_login = true;
                        }
                    });
                });

                if try_login {
                    if let Some(storage) = frame.storage_mut() {
                        match self.rt.block_on(MatrixClient::login(
                            storage,
                            username,
                            password,
                            "https://matrix.envs.net/",
                        )) {
                            Ok(client) => {
                                self.client = LoginOption::LoggedIn(client);
                            }
                            Err(e) => {
                                log::error!("Login failed: {}", e);
                                // TODO: Show error to user
                            }
                        }
                    }
                }
            }
            LoginOption::LoggedIn(client) => {
                // Configure side panel for chat list
                egui::SidePanel::left("left_panel")
                    .resizable(true)
                    .default_width(250.0)
                    .width_range(200.0..=350.0)
                    .show(ctx, |ui| {
                        ui.heading("Chats");

                        egui::ScrollArea::vertical().show(ui, |ui| {
                            for chat in self.chats.lock().iter() {
                                ui.add_space(4.0);

                                // Create a container for the whole chat entry
                                let container = egui::Frame::new()
                                    .fill(egui::Color32::TRANSPARENT)
                                    .inner_margin(2)
                                    .corner_radius(4.0);

                                container
                                    .show(ui, |ui| {
                                        // Make the entire row interactive
                                        let (rect, response) = ui.allocate_exact_size(
                                            egui::vec2(ui.available_width(), 50.0),
                                            egui::Sense::click(),
                                        );

                                        // Handle click
                                        if response.clicked() {
                                            // Clone needed data for async operation
                                            let client_clone = client.clone();
                                            let chat_id = chat.id.clone();
                                            let ctx_clone = ctx.clone();

                                            // Execute in the runtime with proper wakeup
                                            self.rt.block_on(async {
                                                log::info!("select start");
                                                if let Err(e) =
                                                    client_clone.select_chat(&chat_id).await
                                                {
                                                    log::error!("Failed to select chat: {}", e);
                                                }
                                                log::info!("select stop");
                                                ctx_clone.request_repaint();
                                            });
                                        }
                                        // Draw visual feedback for hover with proper sizing
                                        if response.hovered() {
                                            ui.painter().rect_filled(
                                                rect,
                                                4.0,
                                                egui::Color32::from_rgba_premultiplied(
                                                    100, 100, 100, 15,
                                                ), // Much lighter highlight
                                            );
                                        }

                                        // Now add the content inside the interactive area
                                        ui.allocate_new_ui(
                                            UiBuilder::default().max_rect(rect),
                                            |ui| {
                                                ui.horizontal(|ui| {
                                                    let avatar_size = egui::Vec2::new(40.0, 40.0);

                                                    // Display chat avatar if available
                                                    if let Some(avatar) = &chat.avatar {
                                                        let avatar_image = egui::Image::new((
                                                            Cow::Owned(
                                                                "chat-avatar-".to_owned()
                                                                    + chat.id.as_str(),
                                                            ),
                                                            avatar.clone(),
                                                        ))
                                                        .fit_to_exact_size(avatar_size)
                                                        .corner_radius(5.0);

                                                        ui.add(avatar_image);
                                                    } else {
                                                        // Display placeholder if no avatar
                                                        ui.add(
                                                            egui::Label::new("📝")
                                                                .selectable(false),
                                                        );
                                                    }

                                                    ui.add_space(8.0);

                                                    // Chat info column (name and last message placeholder)
                                                    ui.vertical(|ui| {
                                                        let chat_name = chat
                                                            .name
                                                            .as_deref()
                                                            .unwrap_or("Unnamed Chat");
                                                        ui.add(
                                                            egui::Label::new(
                                                                egui::RichText::new(chat_name)
                                                                    .strong()
                                                                    .size(16.0),
                                                            )
                                                            .selectable(false),
                                                        ); // Disable text selection cursor

                                                        // Show a placeholder for last message
                                                        ui.add(
                                                            egui::Label::new(
                                                                egui::RichText::new(
                                                                    "Tap to view messages",
                                                                )
                                                                .weak()
                                                                .size(14.0),
                                                            )
                                                            .selectable(false),
                                                        ); // Disable text selection cursor
                                                    });
                                                });
                                            },
                                        );

                                        response
                                    })
                                    .inner;

                                ui.add_space(4.0);
                                ui.separator();
                            }
                        });
                    });

                // Main panel for chat messages
                egui::CentralPanel::default().show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            for group in client.event_groups().unwrap().lock().iter() {
                                let widget =
                                    MessageWidget::new(MessageStyle::default(), group.clone());
                                widget.show(ui);
                            }
                        });
                });
            }
        }
    }
}

fn powered_by_egui_and_eframe(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.label("Powered by ");
        ui.hyperlink_to("egui", "https://github.com/emilk/egui");
        ui.label(" and ");
        ui.hyperlink_to(
            "eframe",
            "https://github.com/emilk/egui/tree/master/crates/eframe",
        );
        ui.label(".");
    });
}
