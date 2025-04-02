pub mod matrix;

pub enum LoginOption<T> {
    Auth { username: String, password: String },
    LoggedIn(T),
}

impl<T> Default for LoginOption<T> {
    fn default() -> Self {
        Self::Auth {
            username: String::new(),
            password: String::new(),
        }
    }
}
