mod inner;

// pub use inner::Bar as User;  -- "User" here is an alias for inner::Bar.
// Renaming User → Account would change the public API surface of this crate.
pub use crate::inner::Bar as User;

fn make() -> User {
    User
}
