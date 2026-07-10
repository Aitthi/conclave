mod inner;

use crate::inner::User;

fn make(name: String) -> User {
    User { name }
}
