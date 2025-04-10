use std::{path::PathBuf, sync::Arc};

use anyhow::{Result, anyhow};
use egui::ahash::HashSet;
use matrix_sdk::{
    authentication::matrix::MatrixSession,
    config::SyncSettings,
    media::MediaFormat,
    room::{Messages, MessagesOptions, Room},
    ruma::{
        RoomId, UInt,
        api::client::filter::FilterDefinition,
        events::{AnyMessageLikeEventContent, AnySyncTimelineEvent},
    },
};
use parking_lot::Mutex;
use rand::{Rng, distr::Alphanumeric};
use serde::{Deserialize, Serialize};
use tokio::runtime::Runtime;

use super::{Chat, Client, Event, EventGroup, EventKind, LoginForm};

/// Tokio mutex type alias for better readability
type AsyncMutex<T> = tokio::sync::Mutex<T>;

/// Stores Matrix client session information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientSession {
    homeserver: String,
    passphrase: String,
    #[cfg(not(target_arch = "wasm32"))]
    db_path: PathBuf,
}

/// Complete session information including client config and authentication
#[derive(Debug, Serialize, Deserialize)]
pub struct FullSession {
    client_session: ClientSession,
    user_session: MatrixSession,
    #[serde(skip_serializing_if = "Option::is_none")]
    sync_token: Option<String>,
}

/// Matrix client implementation for the chat application
pub struct MatrixClient {
    client: matrix_sdk::Client,
    sync_token: Mutex<Option<String>>,
    client_session: ClientSession,
    event_groups: Arc<Mutex<Vec<EventGroup>>>,
    selected_room: AsyncMutex<Option<Room>>,
    pagination_token: Mutex<Option<String>>,
    processed_events: AsyncMutex<HashSet<String>>,
}

impl MatrixClient {
    /// Create a new Matrix client and log in with the provided credentials
    pub async fn login(
        storage: &mut dyn eframe::Storage,
        username: &str,
        password: &str,
        homeserver: &str,
    ) -> Result<Arc<Self>> {
        // Generate random storage path for desktop platforms
        #[cfg(not(target_arch = "wasm32"))]
        let db_path = {
            let data_dir = dirs::data_dir().unwrap().join("echat");
            let db_subfolder: String = rand::rng()
                .sample_iter(Alphanumeric)
                .take(7)
                .map(char::from)
                .collect();
            data_dir.join(db_subfolder)
        };

        // Generate secure random passphrase for database encryption
        let passphrase: String = rand::rng()
            .sample_iter(Alphanumeric)
            .take(32)
            .map(char::from)
            .collect();

        // Create platform-specific client
        let client = Self::build_client(
            homeserver,
            &passphrase,
            #[cfg(not(target_arch = "wasm32"))]
            &db_path,
        )
        .await?;

        // Perform login
        client
            .matrix_auth()
            .login_username(username, password)
            .initial_device_display_name("matrix-client")
            .await?;

        // Get session information
        let user_session = client
            .matrix_auth()
            .session()
            .ok_or_else(|| anyhow!("Не удалось получить сессию после входа"))?
            .clone();

        // Create client session object
        let client_session = ClientSession {
            homeserver: homeserver.to_owned(),
            passphrase,
            #[cfg(not(target_arch = "wasm32"))]
            db_path,
        };

        // Store session information
        let full_session = FullSession {
            client_session: client_session.clone(),
            user_session,
            sync_token: None,
        };

        storage.set_string("matrix_session", serde_json::to_string(&full_session)?);

        log::info!("Matrix client session created");

        // Create and return the client
        Ok(Arc::new(Self {
            client,
            sync_token: Mutex::new(None),
            client_session,
            event_groups: Arc::default(),
            selected_room: AsyncMutex::default(),
            pagination_token: Mutex::default(),
            processed_events: AsyncMutex::default(),
        }))
    }

    /// Build platform-specific Matrix client
    #[cfg(target_arch = "wasm32")]
    async fn build_client(homeserver: &str, passphrase: &str) -> Result<matrix_sdk::Client> {
        Ok(matrix_sdk::Client::builder()
            .homeserver_url(homeserver)
            .indexeddb_store("matrix_client_db", Some(passphrase))
            .build()
            .await?)
    }

    /// Build platform-specific Matrix client
    #[cfg(not(target_arch = "wasm32"))]
    async fn build_client(
        homeserver: &str,
        passphrase: &str,
        db_path: &PathBuf,
    ) -> Result<matrix_sdk::Client> {
        Ok(matrix_sdk::Client::builder()
            .homeserver_url(homeserver)
            .sqlite_store(db_path, Some(passphrase))
            .build()
            .await?)
    }

    /// Load an existing client session from storage
    pub fn load_from_storage(storage: &dyn eframe::Storage, key: &str) -> Result<Arc<Self>> {
        let serialized = match storage.get_string(key) {
            Some(s) => s,
            None => return Err(anyhow!("Сессия не найдена в хранилище")),
        };

        // Create appropriate runtime for platform
        #[cfg(not(target_arch = "wasm32"))]
        let rt = Runtime::new().unwrap();
        #[cfg(target_arch = "wasm32")]
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();

        rt.block_on(async {
            // Parse stored session data
            let full_session: FullSession = serde_json::from_str(&serialized)?;

            // Create platform-specific client
            let client = {
                #[cfg(target_arch = "wasm32")]
                {
                    Self::build_client(
                        &full_session.client_session.homeserver,
                        &full_session.client_session.passphrase,
                    )
                    .await?
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    Self::build_client(
                        &full_session.client_session.homeserver,
                        &full_session.client_session.passphrase,
                        &full_session.client_session.db_path,
                    )
                    .await?
                }
            };

            // Restore session
            client.restore_session(full_session.user_session).await?;

            // Create and return the client
            Ok(Arc::new(Self {
                client,
                sync_token: Mutex::new(full_session.sync_token),
                client_session: full_session.client_session,
                event_groups: Arc::default(),
                selected_room: AsyncMutex::default(),
                pagination_token: Mutex::default(),
                processed_events: AsyncMutex::default(),
            }))
        })
    }

    /// Process timeline events into event groups for display
    async fn process_timeline_events(
        &self,
        timeline: &Messages,
        room: &Room,
        prepend: bool,
    ) -> Result<()> {
        let mut new_groups = Vec::new();
        let mut current_group: Option<EventGroup> = None;
        let mut processed_events = self.processed_events.lock().await;
        let self_user_id = self
            .client
            .user_id()
            .ok_or_else(|| anyhow!("Не авторизован"))?;

        // Process events in reverse chronological order
        for event in timeline.chunk.iter().rev() {
            // Try to deserialize as a message-like event
            if let AnySyncTimelineEvent::MessageLike(msg) = event.raw().deserialize()? {
                let event_id = msg.event_id().to_string();

                // Skip already processed events
                if processed_events.contains(&event_id) {
                    continue;
                }
                processed_events.insert(event_id.clone());

                let timestamp = msg.origin_server_ts().0.into();
                let sender = msg.sender();

                // Get sender profile information
                let member = room.get_member(sender).await?;
                let display_name = member
                    .as_ref()
                    .and_then(|m| m.display_name().map(ToString::to_string))
                    .unwrap_or_else(|| sender.to_string());

                // Get avatar if available
                let avatar = if let Some(m) = &member {
                    m.avatar(MediaFormat::File).await?.map(Arc::<[u8]>::from)
                } else {
                    None
                };

                // Extract message content
                let event_kind = match msg.original_content() {
                    Some(AnyMessageLikeEventContent::Message(message)) => {
                        let text = message.text.iter().map(|t| t.body.clone()).collect();
                        EventKind::Message(text)
                    }
                    Some(AnyMessageLikeEventContent::RoomMessage(message)) => {
                        EventKind::Message(message.body().to_owned())
                    }
                    _ => continue, // Skip non-message events
                };

                // Create event object
                let event = Event {
                    id: event_id,
                    timestamp,
                    kind: event_kind,
                };

                // Either add to existing group or create a new one
                match &mut current_group {
                    Some(group) if group.user_id == *sender => {
                        // Add to existing group if sender matches
                        group.events.push(event);
                    }
                    _ => {
                        // Otherwise store current group and create a new one
                        if let Some(group) = current_group.take() {
                            new_groups.push(group);
                        }

                        current_group = Some(EventGroup {
                            user_id: sender.to_string(),
                            display_name,
                            avatar,
                            from_self: sender == self_user_id,
                            events: vec![event],
                        });
                    }
                }
            }
        }

        // Add the final group
        if let Some(group) = current_group {
            new_groups.push(group);
        }

        // Update event groups in proper order
        let mut event_groups = self.event_groups.lock();
        if prepend {
            // Add at beginning for historical messages (reversed for correct order)
            new_groups.reverse();
            for group in new_groups {
                event_groups.insert(0, group);
            }
        } else {
            // Add at end for new messages
            event_groups.extend(new_groups);
        }

        Ok(())
    }
}

#[async_trait::async_trait]
impl Client for MatrixClient {
    fn client_name(&self) -> &str {
        "matrix"
    }

    /// Synchronize with the Matrix server to get latest messages
    async fn sync(&self) -> Result<()> {
        // Set up lazy loading filter for optimization
        let filter = FilterDefinition::with_lazy_loading();
        let mut sync_settings = SyncSettings::default().filter(filter.into());

        // Use existing sync token if available
        if let Some(token) = &*self.sync_token.lock() {
            sync_settings = sync_settings.token(token);
        }

        // Perform sync
        let response = self.client.sync_once(sync_settings).await?;
        *self.sync_token.lock() = Some(response.next_batch);

        // Process new messages for each joined room
        for (room_id, room_info) in response.rooms.join {
            let room = self
                .client
                .get_room(&room_id)
                .ok_or_else(|| anyhow!("Комната не найдена: {}", room_id))?;

            // Create Messages object from timeline info
            let timeline = room_info.timeline;
            let messages = Messages {
                chunk: timeline.events,
                start: timeline.prev_batch.clone().unwrap_or_default(),
                end: timeline.prev_batch,
                state: Vec::new(),
            };

            // Process events
            self.process_timeline_events(&messages, &room, false)
                .await?;
        }

        Ok(())
    }

    /// Save current session state to storage
    fn save(&self, storage: &mut dyn eframe::Storage, key: &str) -> Result<()> {
        let user_session = self
            .client
            .matrix_auth()
            .session()
            .ok_or_else(|| anyhow!("Сессия истекла или недоступна"))?
            .clone();

        let full_session = FullSession {
            client_session: self.client_session.clone(),
            user_session,
            sync_token: self.sync_token.lock().clone(),
        };

        storage.set_string(key, serde_json::to_string(&full_session)?);
        Ok(())
    }

    /// Select a chat room and load its messages
    async fn select_chat(&self, chat_id: &str) -> Result<()> {
        // Parse room ID and get room
        let room_id = RoomId::parse(chat_id)?;
        let room = self
            .client
            .get_room(&room_id)
            .ok_or_else(|| anyhow!("Комната не найдена: {}", chat_id))?;

        // Reset state
        self.event_groups.lock().clear();
        *self.pagination_token.lock() = None;
        self.processed_events.lock().await.clear();

        // Set up message loading options
        let mut options = MessagesOptions::backward();
        options.limit = UInt::new(20).unwrap_or_default();

        // Load initial messages
        let timeline = room.messages(options).await?;
        self.process_timeline_events(&timeline, &room, true).await?;

        // Update state
        *self.pagination_token.lock() = timeline.end.clone();
        *self.selected_room.lock().await = Some(room);

        Ok(())
    }

    /// Load more historical events for the selected chat
    async fn load_more_events(&self) -> Result<()> {
        // Get selected room
        let room = {
            let lock = self.selected_room.lock().await;
            lock.clone().ok_or_else(|| anyhow!("Комната не выбрана"))?
        };

        // Set up options with pagination token
        let mut options = MessagesOptions::backward();
        options.limit = UInt::new(20).unwrap_or_default();
        options.from = self.pagination_token.lock().clone();

        // Load and process messages
        let timeline = room.messages(options).await?;
        self.process_timeline_events(&timeline, &room, true).await?;
        *self.pagination_token.lock() = timeline.end.clone();

        Ok(())
    }

    /// Delete an event (placeholder implementation)
    async fn delete_event(&self, _message_id: &str) -> Result<()> {
        // TODO: Implement event deletion
        Ok(())
    }

    /// Get current event groups
    fn event_groups(&self) -> Result<Arc<Mutex<Vec<EventGroup>>>> {
        Ok(self.event_groups.clone())
    }

    /// Get list of available chats
    async fn chats(&self) -> Result<Vec<Chat>> {
        let rooms = self.client.rooms();
        let mut chats = Vec::with_capacity(rooms.len());

        for room in rooms {
            // Get room avatar if available
            let avatar = room.avatar(MediaFormat::File).await?.map(Arc::<[u8]>::from);

            chats.push(Chat {
                id: room.room_id().to_string(),
                name: room.name(),
                avatar,
            });
        }

        Ok(chats)
    }

    /// Get current user ID
    fn self_id(&self) -> Arc<String> {
        Arc::new(
            self.client
                .user_id()
                .map_or("", |id| id.as_str())
                .to_string(),
        )
    }
}

pub struct Login {
    username: String,
    password: String,
    server_url: String,
    error_message: Option<String>,
}

impl Default for Login {
    fn default() -> Self {
        Self {
            username: String::new(),
            password: String::new(),
            server_url: "https://matrix.org/".to_string(),
            error_message: None,
        }
    }
}

impl LoginForm for Login {
    fn show(
        &mut self,
        clients: &Arc<Mutex<Vec<Arc<dyn Client>>>>,
        chats: &Arc<Mutex<Vec<Chat>>>,
        rt: &mut Runtime,
        frame: &mut eframe::Frame,
        ui: &mut egui::Ui,
    ) -> Result<()> {
        let mut try_login = false;

        ui.vertical_centered(|ui| {
            ui.heading("Login to Matrix");
            ui.add_space(10.0);

            ui.label("Username:");
            ui.text_edit_singleline(&mut self.username);

            ui.add_space(5.0);

            ui.label("Password:");
            let password_edit = egui::TextEdit::singleline(&mut self.password)
                .password(true)
                .hint_text("Enter your password");
            ui.add(password_edit);

            ui.add_space(5.0);

            ui.label("Server URL:");
            let server_edit =
                egui::TextEdit::singleline(&mut self.server_url).hint_text("https://matrix.org/");
            ui.add(server_edit);

            ui.add_space(10.0);

            if ui.button("Login").clicked() {
                try_login = true;
            }

            // Show error message if needed
            if let Some(error) = &self.error_message {
                ui.add_space(10.0);
                ui.colored_label(egui::Color32::RED, error);
            }
        });

        if try_login {
            if let Some(storage) = frame.storage_mut() {
                // Make sure the server URL has the correct format
                let server_url = if !self.server_url.starts_with("http") {
                    format!("https://{}", self.server_url)
                } else {
                    self.server_url.clone()
                };

                // Perform the login (block on this part)
                match rt.block_on(MatrixClient::login(
                    storage,
                    &self.username,
                    &self.password,
                    &server_url,
                )) {
                    Ok(client) => {
                        // Login successful, clear error message
                        self.error_message = None;

                        // Add client to the clients list
                        clients.lock().push(client.clone());

                        // Clone for async tasks
                        let client_clone = client.clone();
                        let chats_clone = chats.clone();

                        // Spawn an async task to handle sync and chat loading
                        rt.spawn(async move {
                            if let Err(e) = client_clone.sync().await {
                                log::error!("Failed to sync with server: {}", e);
                                return;
                            }

                            match client_clone.chats().await {
                                Ok(client_chats) => {
                                    *chats_clone.lock() = client_chats;
                                }
                                Err(e) => {
                                    log::error!("Failed to fetch chats: {}", e);
                                }
                            }
                        });

                        return Ok(());
                    }
                    Err(e) => {
                        log::error!("Login failed: {}", e);
                        self.error_message = Some(format!("Login failed: {}", e));
                    }
                }
            }
        }

        Ok(())
    }
}
