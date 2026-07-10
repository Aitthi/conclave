use crate::auth::authenticate;
use crate::{new_user, User};

pub fn login(name: &str) -> Option<User> {
    let u = new_user(1, name);
    if authenticate(&u) {
        Some(u)
    } else {
        None
    }
}

pub fn whoami(u: &User) -> String {
    u.name.clone()
}
