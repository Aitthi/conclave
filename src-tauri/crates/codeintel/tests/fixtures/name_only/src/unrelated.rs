// This file does NOT import User. The `User` reference in the type position
// has no resolving import — the resolver returns Confidence::Low (name-only).
fn make() -> User {
    todo!()
}
