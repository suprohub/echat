use std::sync::Arc;

use anyhow::Result;
use matrix_sdk::async_trait;

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
#[async_trait]
pub trait Client: Send + Sync {
    async fn sync(&self) -> Result<()>;
    fn save(&self, storage: &mut dyn eframe::Storage) -> Result<()>;
    fn chats(&self) -> Vec<Chat>;
}

pub struct Chat {
    pub name: Option<String>,
}

impl Chat {
    pub fn new(name: Option<String>) -> Self {
        Self { name }
    }
}
