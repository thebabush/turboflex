//! Target for c09: type with a user `Drop` impl.

pub struct Guard {
    pub id: u32,
}

impl Guard {
    pub fn new() -> Self {
        Guard { id: 0 }
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.id = 0;
    }
}
