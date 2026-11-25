//! Target for c23: generic container instantiated with another module's type.

pub struct Stack<T> {
    items: Vec<T>,
}

impl<T> Stack<T> {
    pub fn new() -> Self {
        Stack { items: Vec::new() }
    }

    pub fn push(&mut self, t: T) {
        self.items.push(t);
    }
}
