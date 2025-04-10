use std::sync::Arc;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use egui::ahash::HashSet;
use grammers_client::session::Session as GrammersSession;
use grammers_client::{
    Client as GrammersClient, Config, SignInError, Update,
    types::{Chat as GrammersChat, Message as GrammersMessage},
};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::runtime::Runtime;
use tokio::sync::Mutex as AsyncMutex;

use super::{Chat, Client, Event, EventGroup, EventKind, LoginForm};

/// Stores Telegram client session information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientSession {
    api_id: i32,
    api_hash: String,
    #[cfg(not(target_arch = "wasm32"))]
    session_path: PathBuf,
}

/// Complete session information including client config
#[derive(Debug, Serialize, Deserialize)]
pub struct FullSession {
    client_session: ClientSession,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_data: Option<Vec<u8>>,
}

/// Custom session that can be serialized/deserialized
struct SerializableSession {
    inner: GrammersSession,
}

impl Serialize for SerializableSession {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let bytes = self.inner.save();
        serializer.serialize_bytes(&bytes)
    }
}

impl<'de> Deserialize<'de> for SerializableSession {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let bytes = Vec::<u8>::deserialize(deserializer)?;
        let session = GrammersSession::load(&bytes).map_err(serde::de::Error::custom)?;
        Ok(SerializableSession { inner: session })
    }
}

/// Login form for Telegram
#[derive(Default)]
pub struct Login {
    phone: String,
    api_id: String,
    api_hash: String,
    error_message: Option<String>,
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
            ui.heading("Login to Telegram");
            ui.add_space(10.0);

            ui.label("Phone Number:");
            ui.text_edit_singleline(&mut self.phone);

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
                // Get API ID and API Hash from environment variables
                let api_id = match std::env::var("API_ID") {
                    Ok(id_str) => match id_str.parse::<i32>() {
                        Ok(id) => id,
                        Err(_) => {
                            self.error_message = Some(
                                "API_ID environment variable must be a valid number".to_string(),
                            );
                            return Ok(());
                        }
                    },
                    Err(_) => {
                        self.error_message =
                            Some("API_ID environment variable not set".to_string());
                        return Ok(());
                    }
                };

                let api_hash = match std::env::var("API_HASH") {
                    Ok(hash) => hash,
                    Err(_) => {
                        self.error_message =
                            Some("API_HASH environment variable not set".to_string());
                        return Ok(());
                    }
                };

                match rt.block_on(TelegramClient::login(
                    storage,
                    &self.phone,
                    api_id,
                    &api_hash,
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

/// Telegram client implementation for the chat application
pub struct TelegramClient {
    client: Arc<AsyncMutex<GrammersClient>>,
    client_session: ClientSession,
    event_groups: Arc<Mutex<Vec<EventGroup>>>,
    selected_chat: AsyncMutex<Option<String>>,
    processed_events: AsyncMutex<HashSet<String>>,
    user_id: Mutex<Arc<String>>,
}

impl TelegramClient {
    /// Create a new Telegram client and log in with the provided credentials
    pub async fn login(
        storage: &mut dyn eframe::Storage,
        phone: &str,
        api_id: i32,
        api_hash: &str,
    ) -> Result<Arc<Self>> {
        // Set up session path for desktop platforms
        #[cfg(not(target_arch = "wasm32"))]
        let session_path = {
            let data_dir = dirs::data_dir().unwrap().join("echat");
            std::fs::create_dir_all(&data_dir)?;
            data_dir.join("telegram.session")
        };

        // Create client session object
        let client_session = ClientSession {
            api_id,
            api_hash: api_hash.to_owned(),
            #[cfg(not(target_arch = "wasm32"))]
            session_path: session_path.clone(),
        };

        // Connect to Telegram
        let client = GrammersClient::connect(Config {
            #[cfg(not(target_arch = "wasm32"))]
            session: GrammersSession::load_file_or_create(&session_path)?,
            #[cfg(target_arch = "wasm32")]
            session: GrammersSession::new(),
            api_id,
            api_hash: api_hash.to_owned(),
            params: Default::default(),
        })
        .await?;

        // If not authorized, perform login
        if !client.is_authorized().await? {
            println!("Signing in...");
            let token = client.request_login_code(phone).await?;
            // Note: In a real application, you would prompt for the code here
            // For this example, we'll have to assume the code is passed in somehow
            let code = "12345"; // This should be obtained from user input
            let signed_in = client.sign_in(&token, code).await;

            match signed_in {
                Err(SignInError::PasswordRequired(password_token)) => {
                    // In a real app, prompt for password here
                    let password = "your_password"; // Should be obtained from user input
                    client.check_password(password_token, password).await?;
                }
                Ok(_) => (),
                Err(e) => return Err(anyhow!("Failed to sign in: {}", e)),
            };
        }

        // Get user ID
        let user_id = client.get_me().await?.id().to_string();

        // Store session
        #[cfg(not(target_arch = "wasm32"))]
        client.session().save_to_file(&session_path)?;

        let session_data = client.session().save();

        // Create full session for storage
        let full_session = FullSession {
            client_session: client_session.clone(),
            session_data: Some(session_data),
        };

        // Store session information
        storage.set_string("telegram_session", serde_json::to_string(&full_session)?);
        storage.flush();

        // Create and return the client
        Ok(Arc::new(Self {
            client: Arc::new(AsyncMutex::new(client)),
            client_session,
            event_groups: Arc::default(),
            selected_chat: AsyncMutex::default(),
            processed_events: AsyncMutex::default(),
            user_id: Mutex::new(Arc::new(user_id)),
        }))
    }

    /// Load an existing client session from storage
    pub fn load_from_storage(storage: &dyn eframe::Storage, key: &str) -> Result<Arc<Self>> {
        let serialized = storage
            .get_string(key)
            .ok_or_else(|| anyhow!("No saved session found"))?;

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
            let session_data = full_session
                .session_data
                .ok_or_else(|| anyhow!("No session data"))?;

            // Load session from saved data
            let session = GrammersSession::load(&session_data)?;

            // Create client
            let client = GrammersClient::connect(Config {
                session,
                api_id: full_session.client_session.api_id,
                api_hash: full_session.client_session.api_hash.clone(),
                params: Default::default(),
            })
            .await?;

            // Get user ID
            let user_id = client.get_me().await?.id().to_string();

            // Create and return the client
            Ok(Arc::new(Self {
                client: Arc::new(AsyncMutex::new(client)),
                client_session: full_session.client_session,
                event_groups: Arc::default(),
                selected_chat: AsyncMutex::default(),
                processed_events: AsyncMutex::default(),
                user_id: Mutex::new(Arc::new(user_id)),
            }))
        })
    }

    /// Process a message into event groups
    async fn process_message(&self, message: &GrammersMessage) -> Result<()> {
        let mut processed_events = self.processed_events.lock().await;
        let event_id = message.id().to_string();

        // Skip already processed events
        if processed_events.contains(&event_id) {
            return Ok(());
        }
        processed_events.insert(event_id.clone());

        if !message.outgoing() {
            // Handle incoming message
            let sender = message.sender().unwrap();
            let sender_id = sender.id().to_string();
            let display_name = sender.name().to_string();

            // Get avatar (not directly supported in grammers, would need additional implementation)
            let avatar = None; // Placeholder for avatar implementation

            // Create event
            let event = Event {
                id: event_id,
                timestamp: message.date().timestamp() as u64,
                kind: EventKind::Message(message.text().to_owned()),
            };

            // Update event groups
            let mut event_groups = self.event_groups.lock();

            // Try to find an existing group for this sender
            let mut found = false;
            for group in event_groups.iter_mut() {
                if group.user_id == sender_id {
                    group.events.push(event.clone());
                    found = true;
                    break;
                }
            }

            // Create a new group if needed
            if !found {
                event_groups.push(EventGroup {
                    user_id: sender_id,
                    display_name,
                    avatar,
                    events: vec![event.clone()],
                    from_self: false,
                });
            }
        } else {
            // Handle outgoing messages
            let self_id = self.user_id.lock().clone();

            // Create event
            let event = Event {
                id: event_id,
                timestamp: message.date().timestamp() as u64,
                kind: EventKind::Message(message.text().to_owned()),
            };

            // Update event groups
            let mut event_groups = self.event_groups.lock();

            // Try to find an existing group for self
            let mut found = false;
            for group in event_groups.iter_mut() {
                if group.from_self {
                    group.events.push(event.clone());
                    found = true;
                    break;
                }
            }

            // Create a new group if needed
            if !found {
                event_groups.push(EventGroup {
                    user_id: self_id.to_string(),
                    display_name: "You".to_owned(),
                    avatar: None,
                    events: vec![event.clone()],
                    from_self: true,
                });
            }
        }

        Ok(())
    }

    /// Process Telegram updates into event groups for display
    async fn process_update(&self, update: Update) -> Result<()> {
        match update {
            Update::NewMessage(message) => {
                self.process_message(&message).await?;
            }
            _ => (), // Ignore other update types
        }

        Ok(())
    }

    /// Load chat history for a given chat
    async fn load_chat_history(&self, chat: &GrammersChat, limit: i32) -> Result<()> {
        let client = self.client.lock().await;

        // Get message history - convert limit from i32 to usize
        let limit_usize = limit.try_into().unwrap();

        // Fetch messages first
        let mut messages_vec = Vec::new();
        let mut iter = client.iter_messages(chat).limit(limit_usize);

        while let Some(message) = iter.next().await? {
            messages_vec.push(message);
        }

        // We can now drop the client lock
        drop(client);

        // Process messages without holding the lock
        for message in messages_vec {
            self.process_message(&message).await?;
        }

        Ok(())
    }

    /// Find chat by ID
    async fn find_chat(&self, chat_id: &str) -> Result<GrammersChat> {
        let client = self.client.lock().await;
        let mut dialogs = client.iter_dialogs();

        while let Some(dialog) = dialogs.next().await? {
            let chat = dialog.chat();
            if chat.id().to_string() == chat_id {
                return Ok(chat.clone());
            }
        }

        Err(anyhow!("Chat not found: {}", chat_id))
    }
}

#[async_trait]
impl Client for TelegramClient {
    fn client_name(&self) -> &str {
        "telegram"
    }

    /// Synchronize with Telegram to get latest messages
    async fn sync(&self) -> Result<()> {
        let client = self.client.lock().await;

        // Get updates - assuming we need to use a different method since iter_updates doesn't exist
        // Placeholder: Replace with the correct method for getting updates
        let mut messages_vec = Vec::new();

        // This would need to be replaced with the actual method to get updates
        // For now, we'll just handle any new messages in chats we're monitoring
        if let Some(chat_id) = self.selected_chat.lock().await.clone() {
            if let Ok(chat) = self.find_chat(&chat_id).await {
                let mut iter = client.iter_messages(&chat).limit(10_usize);
                while let Some(message) = iter.next().await? {
                    messages_vec.push(message);
                }
            }
        }

        // Drop the lock before processing
        drop(client);

        // Process each message without holding the lock
        for message in messages_vec {
            self.process_update(Update::NewMessage(message)).await?;
        }

        Ok(())
    }

    /// Save current session state to storage
    fn save(&self, storage: &mut dyn eframe::Storage, key: &str) -> Result<()> {
        // Create session data
        let session_data = {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let client_session = &self.client_session;
                let session_path = &client_session.session_path;
                if session_path.exists() {
                    Some(std::fs::read(session_path)?)
                } else {
                    None
                }
            }
            #[cfg(target_arch = "wasm32")]
            None
        };

        let full_session = FullSession {
            client_session: self.client_session.clone(),
            session_data,
        };

        storage.set_string(key, serde_json::to_string(&full_session)?);
        Ok(())
    }

    /// Get list of available chats
    async fn chats(&self) -> Result<Vec<Chat>> {
        let client = self.client.lock().await;
        let mut dialogs = client.iter_dialogs();
        let mut chats = Vec::new();

        while let Some(dialog) = dialogs.next().await? {
            let chat_entity = dialog.chat();

            chats.push(Chat {
                id: chat_entity.id().to_string(),
                name: Some(chat_entity.name().to_owned()),
                avatar: None, // Placeholder for avatar implementation
            });
        }

        Ok(chats)
    }

    /// Select a chat and load its messages
    async fn select_chat(&self, chat_id: &str) -> Result<()> {
        // Find the requested chat
        let chat = self.find_chat(chat_id).await?;

        // Reset state
        self.event_groups.lock().clear();
        self.processed_events.lock().await.clear();

        // Set selected chat
        *self.selected_chat.lock().await = Some(chat_id.to_owned());

        // Load initial messages (20 by default)
        self.load_chat_history(&chat, 20).await?;

        Ok(())
    }

    /// Load more historical events
    async fn load_more_events(&self) -> Result<()> {
        if let Some(chat_id) = self.selected_chat.lock().await.clone() {
            let chat = self.find_chat(&chat_id).await?;

            // Get count of existing events to know how many to skip
            let event_count = self
                .event_groups
                .lock()
                .iter()
                .map(|group| group.events.len())
                .sum::<usize>();

            // Get client and fetch messages
            let client = self.client.lock().await;

            // Create message iterator
            let mut messages_vec = Vec::new();
            let mut iter = client
                .iter_messages(&chat)
                .offset_id(event_count as i32)
                .limit(20_usize);

            // Fetch messages while holding the lock
            while let Some(message) = iter.next().await? {
                messages_vec.push(message);
            }

            // Release the lock
            drop(client);

            // Process the messages without holding the lock
            for message in messages_vec {
                self.process_message(&message).await?;
            }
        }

        Ok(())
    }

    /// Delete an event
    async fn delete_event(&self, message_id: &str) -> Result<()> {
        if let Some(chat_id) = self.selected_chat.lock().await.clone() {
            // Find the chat
            let chat = self.find_chat(&chat_id).await?;

            // Convert message_id to integer
            let msg_id: i32 = message_id.parse()?;

            // Delete the message - delete_messages takes the chat and messages as arguments
            let client = self.client.lock().await;
            client.delete_messages(&chat, &[msg_id]).await?;
        }

        Ok(())
    }

    /// Get current event groups
    fn event_groups(&self) -> Result<Arc<Mutex<Vec<EventGroup>>>> {
        Ok(self.event_groups.clone())
    }

    /// Get current user ID
    fn self_id(&self) -> Arc<String> {
        self.user_id.lock().clone()
    }
}
