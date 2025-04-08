use std::{path::PathBuf, sync::Arc};

use anyhow::{Result, anyhow};
use egui::ahash::HashSet;
use matrix_sdk::{
    authentication::matrix::MatrixSession,
    media::MediaFormat,
    room::{MessagesOptions, Room},
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

use super::{Chat, Client, Event, EventGroup, LoginOption};

type AsyncMutex<T> = tokio::sync::Mutex<T>;

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
    event_groups: Arc<Mutex<Vec<EventGroup>>>,
    selected_room: AsyncMutex<Option<Room>>,
    pagination_token: Mutex<Option<String>>,
    processed_events: AsyncMutex<HashSet<String>>,
}

impl MatrixClient {
    pub async fn login(
        storage: &mut dyn eframe::Storage,
        username: &str,
        password: &str,
        homeserver: &str,
    ) -> Result<Arc<Self>> {
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

        let passphrase: String = rand::rng()
            .sample_iter(Alphanumeric)
            .take(32)
            .map(char::from)
            .collect();

        let client = {
            #[cfg(target_arch = "wasm32")]
            {
                matrix_sdk::Client::builder()
                    .homeserver_url(homeserver)
                    .indexeddb_store("matrix_client_db", Some(&passphrase))
                    .build()
                    .await?
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                matrix_sdk::Client::builder()
                    .homeserver_url(homeserver)
                    .sqlite_store(&db_path, Some(&passphrase))
                    .build()
                    .await?
            }
        };

        client
            .matrix_auth()
            .login_username(username, password)
            .initial_device_display_name("matrix-client")
            .await?;

        let user_session = client
            .matrix_auth()
            .session()
            .ok_or_else(|| anyhow!("Ошибка входа"))?
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

        storage.set_string("matrix_session", serde_json::to_string(&full_session)?);
        storage.flush();

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

    pub fn load_from_storage(storage: &dyn eframe::Storage) -> LoginOption {
        let serialized = match storage.get_string("matrix_session") {
            Some(s) => s,
            None => return LoginOption::default(),
        };

        #[cfg(not(target_arch = "wasm32"))]
        let rt = Runtime::new().unwrap();
        #[cfg(target_arch = "wasm32")]
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();

        match rt.block_on(async {
            let full_session: FullSession = serde_json::from_str(&serialized)?;

            let client = {
                #[cfg(target_arch = "wasm32")]
                {
                    matrix_sdk::Client::builder()
                        .homeserver_url(&full_session.client_session.homeserver)
                        .indexeddb_store(
                            "matrix_client_db",
                            Some(&full_session.client_session.passphrase),
                        )
                        .build()
                        .await?
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    matrix_sdk::Client::builder()
                        .homeserver_url(&full_session.client_session.homeserver)
                        .sqlite_store(
                            &full_session.client_session.db_path,
                            Some(&full_session.client_session.passphrase),
                        )
                        .build()
                        .await?
                }
            };

            client.restore_session(full_session.user_session).await?;

            Ok::<_, anyhow::Error>(Arc::new(Self {
                client,
                sync_token: Mutex::new(full_session.sync_token),
                client_session: full_session.client_session,
                event_groups: Arc::default(),
                selected_room: AsyncMutex::default(),
                pagination_token: Mutex::default(),
                processed_events: AsyncMutex::default(),
            }))
        }) {
            Ok(client) => LoginOption::LoggedIn(client),
            Err(_) => LoginOption::default(),
        }
    }

    async fn process_timeline_events(
        &self,
        timeline: &matrix_sdk::room::Messages,
        room: &Room,
        prepend: bool,
    ) -> Result<()> {
        let mut new_groups = Vec::new();
        let mut current_group: Option<EventGroup> = None;
        let mut processed_events = self.processed_events.lock().await;
        let self_user_id = self.client.user_id().unwrap();

        for event in timeline.chunk.iter().rev() {
            if let AnySyncTimelineEvent::MessageLike(msg) = event.raw().deserialize()? {
                let event_id = msg.event_id().to_string();

                if processed_events.contains(&event_id) {
                    continue;
                }
                processed_events.insert(event_id.clone());

                let timestamp = msg.origin_server_ts().0.into();
                let sender = msg.sender();

                let member = room.get_member(sender).await?;
                let display_name = member
                    .as_ref()
                    .and_then(|m| m.display_name().map(|s| s.to_owned()))
                    .unwrap_or_else(|| sender.to_string());
                let avatar = if let Some(m) = &member {
                    m.avatar(MediaFormat::File).await?.map(Arc::<[u8]>::from)
                } else {
                    None
                };

                let event_type = match msg.original_content() {
                    Some(AnyMessageLikeEventContent::Message(message)) => {
                        let text = message.text.iter().map(|t| t.body.clone()).collect();
                        super::EventKind::Message(text)
                    }
                    Some(AnyMessageLikeEventContent::RoomMessage(message)) => {
                        super::EventKind::Message(message.body().to_owned())
                    }
                    _ => continue,
                };

                let event = Event {
                    id: event_id,
                    timestamp,
                    kind: event_type,
                };

                match &mut current_group {
                    Some(group) if group.user_id == *sender => {
                        group.events.push(event);
                    }
                    _ => {
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

        if let Some(group) = current_group {
            new_groups.push(group);
        }

        let mut event_groups = self.event_groups.lock();
        if prepend {
            new_groups.reverse();
            for group in new_groups {
                event_groups.insert(0, group);
            }
        } else {
            event_groups.extend(new_groups);
        }

        Ok(())
    }
}

#[async_trait::async_trait]
impl Client for MatrixClient {
    async fn sync(&self) -> Result<()> {
        let filter = FilterDefinition::with_lazy_loading();
        let mut sync_settings = matrix_sdk::config::SyncSettings::default().filter(filter.into());

        if let Some(token) = &*self.sync_token.lock() {
            sync_settings = sync_settings.token(token);
        }

        let response = self.client.sync_once(sync_settings).await?;
        *self.sync_token.lock() = Some(response.next_batch);

        for (room_id, room_info) in response.rooms.join {
            let room = self
                .client
                .get_room(&room_id)
                .ok_or_else(|| anyhow!("Комната не найдена"))?;

            let timeline = room_info.timeline;
            let messages = matrix_sdk::room::Messages {
                chunk: timeline.events,
                start: timeline.prev_batch.clone().unwrap_or_default(),
                end: timeline.prev_batch,
                state: Vec::new(),
            };

            self.process_timeline_events(&messages, &room, false)
                .await?;
        }

        Ok(())
    }

    fn save(&self, storage: &mut dyn eframe::Storage) -> Result<()> {
        let user_session = self
            .client
            .matrix_auth()
            .session()
            .ok_or_else(|| anyhow!("Сессия истекла"))?
            .clone();

        let full_session = FullSession {
            client_session: self.client_session.clone(),
            user_session,
            sync_token: self.sync_token.lock().clone(),
        };

        storage.set_string("matrix_session", serde_json::to_string(&full_session)?);
        Ok(())
    }

    async fn select_chat(&self, chat_id: &str) -> Result<()> {
        let room_id = RoomId::parse(chat_id)?;
        let room = self
            .client
            .get_room(&room_id)
            .ok_or_else(|| anyhow!("Комната не найдена"))?;

        // Сброс состояния
        self.event_groups.lock().clear();
        *self.pagination_token.lock() = None;
        self.processed_events.lock().await.clear();

        // Загрузка сообщений
        let mut options = MessagesOptions::backward();
        options.limit = UInt::new(20).unwrap();

        let timeline = room.messages(options).await?;
        self.process_timeline_events(&timeline, &room, true).await?;
        *self.pagination_token.lock() = timeline.end.clone();
        *self.selected_room.lock().await = Some(room);

        Ok(())
    }

    async fn load_more_events(&self) -> Result<()> {
        let room = {
            let lock = self.selected_room.lock().await;
            lock.clone().ok_or_else(|| anyhow!("Комната не выбрана"))?
        };

        let mut options = MessagesOptions::backward();
        options.limit = UInt::new(20).unwrap();

        options.from = self.pagination_token.lock().clone();

        let timeline = room.messages(options).await?;
        self.process_timeline_events(&timeline, &room, true).await?;
        *self.pagination_token.lock() = timeline.end.clone();

        Ok(())
    }

    async fn delete_event(&self, _message_id: &str) -> Result<()> {
        // Заглушка для будущей реализации
        Ok(())
    }

    fn event_groups(&self) -> Result<Arc<Mutex<Vec<EventGroup>>>> {
        Ok(self.event_groups.clone())
    }

    async fn chats(&self) -> Result<Vec<Chat>> {
        let rooms = self.client.rooms();
        let mut chats = Vec::with_capacity(rooms.len());

        for room in rooms {
            let avatar = room.avatar(MediaFormat::File).await?.map(Arc::<[u8]>::from);

            chats.push(Chat {
                id: room.room_id().to_string(),
                name: room.name(),
                avatar,
            });
        }

        Ok(chats)
    }

    fn self_id(&self) -> &str {
        self.client.user_id().unwrap().as_str()
    }
}
