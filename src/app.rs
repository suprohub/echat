use egui::{Color32, Style};
use tokio::runtime::Runtime;

use crate::{clients::{matrix::MatrixClient, EventKind, LoginOption}, message::{MessageStyle, MessageWidget}};

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct EChat {
    #[serde(skip)]
    rt: Runtime,
    #[serde(skip)]
    client: LoginOption,
}

impl Default for EChat {
    fn default() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        let rt = Runtime::new().unwrap();
        #[cfg(target_arch = "wasm32")]
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();

        Self {
            client: Default::default(),
            rt,
        }
    }
}

impl EChat {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {

        egui_extras::install_image_loaders(&cc.egui_ctx);

        if let Some(storage) = cc.storage {
            let mut echat: EChat = eframe::get_value(storage, eframe::APP_KEY).unwrap_or_default();

            echat.client = MatrixClient::load_from_storage(storage);

            if let LoginOption::LoggedIn(client) = &echat.client {
                let client = client.clone();
                echat.rt.spawn(async move { client.sync().await.unwrap() });
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
            client.save(storage).unwrap();
        }
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        match &mut self.client {
            LoginOption::Auth { username, password } => {
                let mut try_login = false;

                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.label("Login");
                    ui.text_edit_singleline(username);
                    ui.label("Password");
                    ui.text_edit_singleline(password);

                    if ui.button("Confirm").clicked() {
                        try_login = true;
                    }
                });

                if try_login {
                    self.client = LoginOption::LoggedIn(
                        self.rt
                            .block_on(MatrixClient::login(
                                frame.storage_mut().unwrap(),
                                username,
                                password,
                                "https://matrix.envs.net/",
                            ))
                            .unwrap(),
                    );
                }
            }
            LoginOption::LoggedIn(client) => {
                egui::SidePanel::left("left_panel").show(ctx, |ui| {
                    ui.label("Chats");
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for chat in client.get_chats() {
                            let btn = ui.add(egui::Button::new(chat.name.as_deref().unwrap_or("Unnamed")));
                            if btn.clicked() {
                                let client = client.clone();
                                let chat_id = chat.id.clone();
                                self.rt.spawn(async move {
                                    log::info!("select start");
                                    client.select_chat(&chat_id).await.unwrap();
                                    log::info!("select stop");
                                });
                            }
                            if let Some(avatar) = &chat.avatar_url {
                                ui.image(avatar);
                            }
                        }
                    });
                });
                egui::CentralPanel::default().show(ctx, |ui| {
                    let current_user_id = client.get_user_id().to_string();
                    let groups = client.get_event_groups();
                    
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for group in groups {
                            let widget = MessageWidget::new(
                                MessageStyle::default(),
                                group.clone(),
                                current_user_id.clone()
                            );
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
