use egui::panel::Side;
use tokio::runtime::Runtime;

use crate::clients::{LoginOption, matrix::MatrixClient};

/// We derive Deserialize/Serialize so we can persist app state on shutdown.
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(default)] // if we add new fields, give them default values when deserializing old state
pub struct EChat {
    #[serde(skip)]
    rt: Runtime,
    #[serde(skip)]
    matrix_client: LoginOption<MatrixClient>,
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
            matrix_client: Default::default(),
            rt,
        }
    }
}

impl EChat {
    /// Called once before the first frame.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // This is also where you can customize the look and feel of egui using
        // `cc.egui_ctx.set_visuals` and `cc.egui_ctx.set_fonts`.

        // Load previous app state (if any).
        // Note that you must enable the `persistence` feature for this to work.
        if let Some(storage) = cc.storage {
            let mut echat: EChat = eframe::get_value(storage, eframe::APP_KEY).unwrap_or_default();

            echat.matrix_client = MatrixClient::load_from_storage(storage);

            echat
        } else {
            Default::default()
        }
    }
}

impl eframe::App for EChat {
    /// Called by the frame work to save state before shutdown.
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, eframe::APP_KEY, self);
        if let LoginOption::LoggedIn(client) = &mut self.matrix_client {
            self.rt.block_on(client.sync(storage)).unwrap();
        }
    }

    /// Called each time the UI needs repainting, which may be many times per second.
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        match &mut self.matrix_client {
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
                    self.matrix_client = LoginOption::LoggedIn(
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
                egui::CentralPanel::default().show(
                    ctx,
                    |ui| {
                        if ui.button("Display name").clicked() {}
                    },
                );

                egui::SidePanel::new(Side::Left, "left_panel").show(ctx, |ui| {
                    ui.label("Chats");

                    for room in client.client().rooms() {
                        ui.label(room.name().unwrap_or("Unknown name".into()));
                    }
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
