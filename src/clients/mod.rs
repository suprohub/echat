use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::runtime::Runtime;

pub mod matrix;
pub mod telegram;

pub trait LoginForm: Send + Sync {
    fn show(
        &mut self,
        clients: &Arc<Mutex<Vec<Arc<dyn Client>>>>,
        chats: &Arc<Mutex<Vec<Chat>>>,
        rt: &mut Runtime,
        frame: &mut eframe::Frame,
        ui: &mut egui::Ui,
    ) -> Result<()>;
}

#[async_trait]
pub trait Client: Send + Sync {
    fn client_name(&self) -> &str;

    async fn sync(&self) -> Result<()>;
    fn save(&self, storage: &mut dyn eframe::Storage, key: &str) -> Result<()>;

    async fn chats(&self) -> Result<Vec<Chat>>;
    async fn select_chat(&self, chat_id: &str) -> Result<()>;
    async fn load_more_events(&self) -> Result<()>;
    fn event_groups(&self) -> Result<Arc<Mutex<Vec<EventGroup>>>>;

    async fn delete_event(&self, message_id: &str) -> Result<()>;

    fn self_id(&self) -> Arc<String>;
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
    pub from_self: bool,
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
