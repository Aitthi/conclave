use crate::User;

pub fn authenticate(u: &User) -> bool {
    !u.name.is_empty()
}

pub fn revoke(u: &User) -> bool {
    authenticate(u)
}
