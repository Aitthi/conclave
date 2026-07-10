pub struct Greeter {
    pub name: String,
}

impl Greeter {
    pub fn new(name: &str) -> Self {
        Self { name: name.into() }
    }

    pub fn greet(&self) -> String {
        format!("hello, {}", self.name)
    }
}

pub enum Mood {
    Happy,
    Sad,
}

pub trait Speak {
    fn speak(&self) -> String;
}

pub type Result<T> = std::result::Result<T, String>;

pub const VERSION: &str = "0.1.0";
