//! c23: generic container (`app::containers::Stack<T>`) instantiated with
//! another module's type (`app::shapes::Circle`).

use crate::containers::Stack;
use crate::shapes::Circle;

pub fn run() -> Stack<Circle> {
    let mut s = Stack::new();
    s.push(Circle::new(2.0));
    s
}
