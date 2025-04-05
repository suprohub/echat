use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub mod matrix;

pub enum LoginOption {
    Auth { username: String, password: String },
    LoggedIn(Arc<dyn Client>),
}

impl Default for LoginOption {
    fn default() -> Self {
        Self::Auth {
            username: String::new(),
            password: String::new(),
        }
    }
}

#[async_trait]
pub trait Client: Send + Sync {
    async fn sync(&self) -> Result<()>;
    fn save(&self, storage: &mut dyn eframe::Storage) -> Result<()>;

    async fn chats(&self) -> Result<Vec<Chat>>;
    async fn select_chat(&self, chat_id: &str) -> Result<()>;
    async fn load_more_events(&self) -> Result<()>;
    async fn event_groups(&self) -> Result<Vec<EventGroup>>;

    async fn delete_event(&self, message_id: &str) -> Result<()>;

    fn self_id(&self) -> &str;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chat {
    pub id: String,
    pub name: Option<String>,
    pub avatar: Option<Arc<[u8]>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventGroup {
    pub user_id: String,
    pub display_name: String,
    pub avatar: Option<Arc<[u8]>>,
    pub events: Vec<Event>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub timestamp: u64,
    pub kind: EventKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EventKind {
    Message(String),
    // Другие типы событий можно добавить здесь
}
