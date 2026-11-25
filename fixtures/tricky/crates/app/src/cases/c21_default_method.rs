//! c21: call to a trait method that the impl (`app::report_impl`) does not
//! override, so it runs the *default* body in `app::defaults`, which calls
//! `app::render`.

use crate::defaults::Report;

pub fn run() -> String {
    crate::report_impl::Doc.render()
}
