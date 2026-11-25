//! Target for c04: "module C" — holds the impls of a trait from module B
//! (and from crate `core_api`) for a type from module A.

impl crate::traits_b::Describe for crate::model_a::Widget {
    fn describe(&self) -> String {
        format!("widget {}", self.id)
    }
}

impl core_api::Greeter for crate::model_a::Widget {
    fn greet(&self) -> String {
        String::from("hello from widget")
    }
}
