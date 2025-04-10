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

        Self {
            rt,
            logins: vec![
                Box::new(matrix::Login::default()),
                Box::new(telegram::Login::default()),
            ],
            clients: Default::default(),
            client_keys: Vec::new(),
            chats: Arc::new(Mutex::new(Vec::new())),
            active_client_index: None,
        }
    }
}

impl EChat {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Load vars
        dotenv::dotenv().unwrap();

        // Enable image loading for avatars
        egui_extras::install_image_loaders(&cc.egui_ctx);

        if let Some(storage) = cc.storage {
            // Load app state from storage if available
            let mut echat: EChat = eframe::get_value(storage, eframe::APP_KEY).unwrap_or_default();

            // Load clients from storage using saved keys
            for key in &echat.client_keys {
                let client_result = match key {
                    k if k.starts_with("matrix-") => {
                        MatrixClient::load_from_storage(storage, k).map(|c| c as Arc<dyn Client>)
                    }
                    k if k.starts_with("telegram-") => {
                        TelegramClient::load_from_storage(storage, k).map(|c| c as Arc<dyn Client>)
                    }
                    _ => {
                        log::error!("Unknown client type in key: {}", key);
                        continue;
                    }
                };

                if let Ok(client) = client_result {
                    echat.clients.lock().push(client.clone());

                    // Set first loaded client as active
                    if echat.active_client_index.is_none() {
                        echat.active_client_index = Some(0);
                        echat.sync_client_chats(&client);
                    }
                } else {
                    log::error!("Failed to load client with key: {}", key);
                }
            }

            // Add login forms if no clients loaded
            if echat.clients.lock().is_empty() {
                echat.logins = vec![
                    Box::new(matrix::Login::default()),
                    Box::new(telegram::Login::default()),
                ];
            }

            echat
        } else {
            Default::default()
        }
    }

    fn sync_client_chats(&self, client: &Arc<dyn Client>) {
        let client_clone = client.clone();
        let chats = self.chats.clone();

        self.rt.spawn(async move {
            if let Err(e) = client_clone.sync().await {
                log::error!("Failed to sync with server: {}", e);
                return;
            }

            match client_clone.chats().await {
                Ok(client_chats) => *chats.lock() = client_chats,
                Err(e) => log::error!("Failed to fetch chats: {}", e),
            }
        });
    }
}

impl eframe::App for EChat {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.client_keys.clear();

        for client in self.clients.lock().iter() {
            let key = format!("{}-{}", client.client_name(), client.self_id());
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

        if !self.clients.lock().is_empty() {
            self.show_chat_interface(ctx, frame);
        } else {
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

                    // Check if login was successful
                    let clients = self.clients.lock();
                    if !clients.is_empty() {
                        drop(clients); // Drop the lock before calling save

                        if let Some(storage) = frame.storage_mut() {
                            let client = &self.clients.lock()[0];
                            let key = format!("{}-{}", client.client_name(), client.self_id());

                            if let Err(e) = client.save(storage, &key) {
                                log::error!("Failed to save client state: {}", e);
                            } else {
                                self.client_keys.push(key);
                                eframe::set_value(storage, eframe::APP_KEY, self);
                            }

                            self.active_client_index = Some(0);
                            self.logins.clear();
                            break;
                        }
                    }
                }
            });
        });
    }

    fn show_chat_interface(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let active_client_index = match self.active_client_index {
            Some(idx) if idx < self.clients.lock().len() => idx,
            _ => return,
        };

        let client = self.clients.lock()[active_client_index].clone();

        // Side panel for chat list
        self.show_chat_list(ctx, &client);

        // Main panel for chat messages
        self.show_message_panel(ctx, &client);
    }

    fn show_chat_list(&mut self, ctx: &egui::Context, client: &Arc<dyn Client>) {
        egui::SidePanel::left("left_panel")
            .resizable(true)
            .default_width(250.0)
            .width_range(200.0..=350.0)
            .show(ctx, |ui| {
                ui.heading("Chats");

                egui::ScrollArea::vertical().show(ui, |ui| {
                    for chat in self.chats.lock().iter() {
                        ui.add_space(4.0);
                        self.render_chat_item(ui, ctx, client, chat);
                        ui.add_space(4.0);
                        ui.separator();
                    }
                });
            });
    }

    fn render_chat_item(
        &self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        client: &Arc<dyn Client>,
        chat: &Chat,
    ) {
        let container = egui::Frame::new()
            .fill(egui::Color32::TRANSPARENT)
            .inner_margin(2)
            .corner_radius(4.0);

        container.show(ui, |ui| {
            // Make the entire row interactive
            let (rect, response) = ui
                .allocate_exact_size(egui::vec2(ui.available_width(), 50.0), egui::Sense::click());

            if response.clicked() {
                let client_clone = client.clone();
                let chat_id = chat.id.clone();
                let ctx_clone = ctx.clone();

                self.rt.spawn(async move {
                    if let Err(e) = client_clone.select_chat(&chat_id).await {
                        log::error!("Failed to select chat: {}", e);
                    }
                    ctx_clone.request_repaint();
                });
            }

            if response.hovered() {
                ui.painter().rect_filled(
                    rect,
                    4.0,
                    egui::Color32::from_rgba_premultiplied(100, 100, 100, 15),
                );
            }

            ui.allocate_new_ui(UiBuilder::default().max_rect(rect), |ui| {
                ui.horizontal(|ui| {
                    let avatar_size = egui::Vec2::new(40.0, 40.0);

                    if let Some(avatar) = &chat.avatar {
                        ui.add(
                            egui::Image::new((
                                Cow::Owned("chat-avatar-".to_owned() + chat.id.as_str()),
                                avatar.clone(),
                            ))
                            .fit_to_exact_size(avatar_size)
                            .corner_radius(5.0),
                        );
                    } else {
                        ui.add(egui::Label::new("📝").selectable(false));
                    }

                    ui.add_space(8.0);

                    ui.vertical(|ui| {
                        let chat_name = chat.name.as_deref().unwrap_or("Unnamed Chat");
                        ui.add(
                            egui::Label::new(egui::RichText::new(chat_name).strong().size(16.0))
                                .selectable(false),
                        );

                        ui.add(
                            egui::Label::new(
                                egui::RichText::new("Tap to view messages")
                                    .weak()
                                    .size(14.0),
                            )
                            .selectable(false),
                        );
                    });
                });
            });
        });
    }

    fn show_message_panel(&self, ctx: &egui::Context, client: &Arc<dyn Client>) {
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    if let Ok(event_groups) = client.event_groups() {
                        for group in event_groups.lock().iter() {
                            MessageWidget::new(MessageStyle::default(), group.clone()).show(ui);
                        }
                    }
                });
        });
    }
}
