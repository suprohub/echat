use egui::UiBuilder;
use parking_lot::Mutex;
use std::{borrow::Cow, sync::Arc};
use tokio::runtime::Runtime;

use crate::{
    clients::{
        Chat, Client, LoginForm,
        matrix::{self, MatrixClient},
        telegram::{self, TelegramClient},
    },
    message::{MessageStyle, MessageWidget},
};

/// Main application state for the EChat app
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct EChat {
    #[serde(skip)]
    rt: Runtime,
    #[serde(skip)]
    logins: Vec<Box<dyn LoginForm>>,
    #[serde(skip)]
    clients: Arc<Mutex<Vec<Arc<dyn Client>>>>,
    // Store client keys for persistence
    client_keys: Vec<String>,
    chats: Arc<Mutex<Vec<Chat>>>,
    active_client_index: Option<usize>,
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

        // Create default login forms
        let mut logins: Vec<Box<dyn LoginForm>> = Vec::new();
        logins.push(Box::new(matrix::Login::default()));
        logins.push(Box::new(telegram::Login::default()));

        Self {
            rt,
            logins,
            clients: Default::default(),
            client_keys: Vec::new(),
            chats: Arc::new(Mutex::new(Vec::new())),
            active_client_index: None,
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

            // Load clients from storage using saved keys
            for key in &echat.client_keys {
                if key.starts_with("matrix-") {
                    if let Ok(client) = MatrixClient::load_from_storage(storage, key) {
                        let client_clone = client.clone();
                        echat.clients.lock().push(client.clone());

                        // Set the first loaded client as active
                        if echat.active_client_index.is_none() {
                            echat.active_client_index = Some(0);

                            // Sync with server and get chats
                            let chats = echat.chats.clone();
                            echat.rt.spawn(async move {
                                if let Err(e) = client_clone.sync().await {
                                    log::error!("Failed to sync with server: {}", e);
                                    return;
                                }

                                match client_clone.chats().await {
                                    Ok(client_chats) => {
                                        *chats.lock() = client_chats;
                                    }
                                    Err(e) => {
                                        log::error!("Failed to fetch chats: {}", e);
                                    }
                                }
                            });
                        }
                    } else {
                        log::error!("Failed to load Matrix client with key: {}", key);
                    }
                } else if key.starts_with("telegram-") {
                    if let Ok(client) = TelegramClient::load_from_storage(storage, key) {
                        echat.clients.lock().push(client);

                        // Set as active if no other client is active
                        if echat.active_client_index.is_none() {
                            echat.active_client_index = Some(0);
                        }
                    } else {
                        log::error!("Failed to load Telegram client with key: {}", key);
                    }
                } else {
                    log::error!("Unknown client type in key: {}", key);
                }
            }

            // Add login forms if no clients loaded
            if echat.clients.lock().is_empty() {
                echat.logins.push(Box::new(matrix::Login::default()));
                echat.logins.push(Box::new(telegram::Login::default()));
            }

            echat
        } else {
            Default::default()
        }
    }
}

impl eframe::App for EChat {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        // Update client keys before saving
        self.client_keys.clear();

        for client in self.clients.lock().iter() {
            let client_name = client.client_name();
            let self_id = client.self_id();
            let key = format!("{}-{}", client_name, self_id);

            self.client_keys.push(key.clone());

            if let Err(e) = client.save(storage, &key) {
                log::error!("Failed to save client state: {}", e);
            }
        }

        eframe::set_value(storage, eframe::APP_KEY, self);
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Handle app close
        if ctx.input(|i| i.viewport().close_requested()) {
            if let Some(storage) = frame.storage_mut() {
                self.save(storage);
            }
            std::process::exit(0); // TODO: Implement cleaner shutdown
        }

        // If we have active clients, show the chat interface
        if !self.clients.lock().is_empty() {
            self.show_chat_interface(ctx, frame);
        } else {
            // Otherwise show login forms
            self.show_login_forms(ctx, frame);
        }
    }
}

impl EChat {
    fn show_login_forms(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.heading("Choose Login Method");
                ui.add_space(10.0);

                for login_form in &mut self.logins {
                    if let Err(e) =
                        login_form.show(&self.clients, &self.chats, &mut self.rt, frame, ui)
                    {
                        log::error!("Error displaying login form: {}", e);
                    }

                    // Check if login was successful and a new client was added
                    if !self.clients.lock().is_empty() {
                        if let Some(storage) = frame.storage_mut() {
                            // Get the latest client and create a key
                            let client = &self.clients.lock()[0];
                            let client_name = client.client_name();
                            let self_id = client.self_id();
                            let key = format!("{}-{}", client_name, self_id);

                            // Save the client with its unique key
                            if let Err(e) = client.save(storage, &key) {
                                log::error!("Failed to save client state: {}", e);
                            } else {
                                // Add key to our list on successful save
                                self.client_keys.push(key);
                                // Save updated state
                                eframe::set_value(storage, eframe::APP_KEY, self);
                            }

                            self.active_client_index = Some(0);
                            self.logins.clear(); // Clear login forms after successful login
                            break;
                        }
                    }
                }
            });
        });
    }

    fn show_chat_interface(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Get the active client
        let active_client_index = self.active_client_index.unwrap_or(0);
        if active_client_index >= self.clients.lock().len() {
            return;
        }

        let client = &self.clients.lock()[active_client_index];

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

                        container.show(ui, |ui| {
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
                                self.rt.spawn(async move {
                                    log::info!("select start");
                                    if let Err(e) = client_clone.select_chat(&chat_id).await {
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
                                    egui::Color32::from_rgba_premultiplied(100, 100, 100, 15), // Much lighter highlight
                                );
                            }

                            // Now add the content inside the interactive area
                            ui.allocate_new_ui(UiBuilder::default().max_rect(rect), |ui| {
                                ui.horizontal(|ui| {
                                    let avatar_size = egui::Vec2::new(40.0, 40.0);

                                    // Display chat avatar if available
                                    if let Some(avatar) = &chat.avatar {
                                        let avatar_image = egui::Image::new((
                                            Cow::Owned(
                                                "chat-avatar-".to_owned() + chat.id.as_str(),
                                            ),
                                            avatar.clone(),
                                        ))
                                        .fit_to_exact_size(avatar_size)
                                        .corner_radius(5.0);

                                        ui.add(avatar_image);
                                    } else {
                                        // Display placeholder if no avatar
                                        ui.add(egui::Label::new("📝").selectable(false));
                                    }

                                    ui.add_space(8.0);

                                    // Chat info column (name and last message placeholder)
                                    ui.vertical(|ui| {
                                        let chat_name =
                                            chat.name.as_deref().unwrap_or("Unnamed Chat");
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(chat_name).strong().size(16.0),
                                            )
                                            .selectable(false),
                                        ); // Disable text selection cursor

                                        // Show a placeholder for last message
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new("Tap to view messages")
                                                    .weak()
                                                    .size(14.0),
                                            )
                                            .selectable(false),
                                        ); // Disable text selection cursor
                                    });
                                });
                            });

                            response
                        });

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
                    if let Ok(event_groups) = client.event_groups() {
                        for group in event_groups.lock().iter() {
                            let widget = MessageWidget::new(MessageStyle::default(), group.clone());
                            widget.show(ui);
                        }
                    }
                });
        });
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
