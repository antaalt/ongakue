//! No shader editor natively (yet): every method does nothing.

use winit::event_loop::EventLoopProxy;

use crate::UserEvent;

pub struct Editor;

impl Editor {
    pub fn new(_proxy: EventLoopProxy<UserEvent>) -> Result<Self, String> {
        Ok(Self)
    }

    pub fn show(&self, _visual: usize, _name: &str, _source: &str) {}

    pub fn set_error(&self, _error: Option<&str>) {}

    pub fn saved_source(&self, _name: &str) -> Option<String> {
        None
    }

    pub fn save_source(&self, _name: &str, _source: &str) {}

    pub fn forget_source(&self, _name: &str) {}
}
