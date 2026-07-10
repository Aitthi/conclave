mod unrelated;

pub struct User;

// Second definition with same name to trigger Low confidence (ambiguity).
mod duplicate {
    pub struct User;
}
