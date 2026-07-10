struct User {
    id: u64,
}

fn make() -> User {
    User { id: 0 }
}

fn count(u: &User) -> u64 {
    u.id
}
