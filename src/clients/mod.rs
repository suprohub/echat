use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

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
    async fn select_chat(&self, chat_id: &str) -> Result<()>;
    async fn load_more_messages(&self) -> Result<()>;
    async fn delete_message(&self, message_id: &str) -> Result<()>;
    fn get_event_groups(&self) -> Vec<EventGroup>;
    fn get_chats(&self) -> Vec<Chat>;
    fn get_user_id(&self) -> &str;
}

#[derive(Debug, Clone)]
pub struct Chat {
    pub id: String,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct EventGroup {
    pub user_id: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub events: Vec<Event>,
}

#[derive(Debug, Clone)]
pub struct Event {
    pub id: String,
    pub timestamp: u64,
    pub kind: EventKind,
}

#[derive(Debug, Clone)]
pub enum EventKind {
    Message(String),
    // Другие типы событий можно добавить здесь
}