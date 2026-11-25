//! Target for c21: impl of `defaults::Report` that does NOT override
//! `render`, so calls to `Doc::render` resolve to the default body.

pub struct Doc;

impl crate::defaults::Report for Doc {
    fn name(&self) -> String {
        String::from("doc")
    }
}
