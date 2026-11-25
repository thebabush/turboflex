//! Target for c21: trait whose *default* method body calls another module.

pub trait Report {
    fn name(&self) -> String;

    fn render(&self) -> String {
        crate::render::frame(&self.name())
    }
}
