pub mod auth;
pub mod handlers;

pub struct User {
    pub id: u32,
    pub name: String,
}

pub fn new_user(id: u32, name: &str) -> User {
    User { id, name: name.to_string() }
}

pub fn unused_helper() -> u32 {
    42
}
