use std::{path::PathBuf, sync::Arc};

use anyhow::{Result, anyhow};
use egui::mutex::Mutex;
use matrix_sdk::{
    async_trait, authentication::matrix::MatrixSession, config::SyncSettings,
    ruma::api::client::filter::FilterDefinition,
};
use rand::{Rng, distr::Alphanumeric};
use serde::{Deserialize, Serialize};
use tokio::runtime::Runtime;

use super::{Chat, Client, LoginOption};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientSession {
    homeserver: String,
    passphrase: String,
    #[cfg(not(target_arch = "wasm32"))]
    db_path: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FullSession {
    client_session: ClientSession,
    user_session: MatrixSession,
    #[serde(skip_serializing_if = "Option::is_none")]
    sync_token: Option<String>,
}

pub struct MatrixClient {
    client: matrix_sdk::Client,
    sync_token: Mutex<Option<String>>,
    client_session: ClientSession,
}

impl MatrixClient {
    pub async fn login(
        storage: &mut dyn eframe::Storage,
        username: &str,
        password: &str,
        homeserver: &str,
    ) -> Result<Arc<Self>> {
        #[cfg(not(target_arch = "wasm32"))]
        let data_dir = dirs::data_dir().unwrap().join("echat");

        #[cfg(not(target_arch = "wasm32"))]
        let db_subfolder: String = rand::rng()
            .sample_iter(Alphanumeric)
            .take(7)
            .map(char::from)
            .collect();

        #[cfg(not(target_arch = "wasm32"))]
        let db_path = data_dir.join(db_subfolder);

        let passphrase: String = rand::rng()
            .sample_iter(Alphanumeric)
            .take(32)
            .map(char::from)
            .collect();

        #[cfg(target_arch = "wasm32")]
        let client = matrix_sdk::Client::builder()
            .homeserver_url(homeserver)
            .indexeddb_store("matrix_client_db", Some(&passphrase))
            .build()
            .await?;

        #[cfg(not(target_arch = "wasm32"))]
        let client = matrix_sdk::Client::builder()
            .homeserver_url(homeserver)
            .sqlite_store(&db_path, Some(&passphrase))
            .build()
            .await?;

        client
            .matrix_auth()
            .login_username(username, password)
            .initial_device_display_name("matrix-client")
            .await?;

        let user_session = client
            .matrix_auth()
            .session()
            .ok_or_else(|| anyhow!("Login failed"))?
            .clone();

        let client_session = ClientSession {
            homeserver: homeserver.to_owned(),
            passphrase,
            #[cfg(not(target_arch = "wasm32"))]
            db_path,
        };

        let full_session = FullSession {
            client_session: client_session.clone(),
            user_session,
            sync_token: None,
        };

        let serialized = serde_json::to_string(&full_session)?;
        storage.set_string("matrix_session", serialized);
        storage.flush();

        Ok(Arc::new(Self {
            client,
            sync_token: Mutex::new(None),
            client_session,
        }))
    }

    pub fn load_from_storage(storage: &dyn eframe::Storage) -> LoginOption {
        if let Some(serialized) = storage.get_string("matrix_session") {
            #[cfg(not(target_arch = "wasm32"))]
            let rt = Runtime::new().unwrap();
            #[cfg(target_arch = "wasm32")]
            let rt = tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap();

            rt.block_on(async {
                let full_session: FullSession = serde_json::from_str(&serialized)
                    .map_err(|e| anyhow!("Deserialization error: {}", e))?;

                #[cfg(target_arch = "wasm32")]
                let client = matrix_sdk::Client::builder()
                    .homeserver_url(&full_session.client_session.homeserver)
                    .indexeddb_store(
                        "matrix_client_db",
                        Some(&full_session.client_session.passphrase),
                    )
                    .build()
                    .await?;

                #[cfg(not(target_arch = "wasm32"))]
                let client = matrix_sdk::Client::builder()
                    .homeserver_url(&full_session.client_session.homeserver)
                    .sqlite_store(
                        &full_session.client_session.db_path,
                        Some(&full_session.client_session.passphrase),
                    )
                    .build()
                    .await?;

                client.restore_session(full_session.user_session).await?;

                Ok(Arc::new(Self {
                    client,
                    sync_token: Mutex::new(full_session.sync_token),
                    client_session: full_session.client_session,
                }))
            })
            .map(|c| LoginOption::LoggedIn(c))
            .unwrap_or_else(|_: anyhow::Error| LoginOption::default())
        } else {
            LoginOption::default()
        }
    }
}

#[async_trait]
impl Client for MatrixClient {
    async fn sync(&self) -> Result<()> {
        let filter = FilterDefinition::with_lazy_loading();
        let mut sync_settings = SyncSettings::default().filter(filter.into());

        if let Some(token) = &*self.sync_token.lock() {
            sync_settings = sync_settings.token(token);
        }

        let response = self.client.sync_once(sync_settings).await?;
        *self.sync_token.lock() = Some(response.next_batch.clone());
        Ok(())
    }

    fn save(&self, storage: &mut dyn eframe::Storage) -> Result<()> {
        let user_session = self
            .client
            .matrix_auth()
            .session()
            .ok_or_else(|| anyhow!("Session expired"))?
            .clone();

        storage.set_string(
            "matrix_session",
            serde_json::to_string(&FullSession {
                client_session: self.client_session.clone(),
                user_session,
                sync_token: self.sync_token.lock().clone(),
            })?,
        );

        Ok(())
    }

    fn chats(&self) -> Vec<Chat> {
        self.client
            .rooms()
            .into_iter()
            .map(|r| Chat::new(r.name()))
            .collect()
    }
}
