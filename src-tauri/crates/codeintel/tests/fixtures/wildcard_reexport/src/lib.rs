mod inner;

// `User` crosses this re-export boundary; renaming it would silently
// change the wildcard's export surface.
pub use crate::inner::*;
