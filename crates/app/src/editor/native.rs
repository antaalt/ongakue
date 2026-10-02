//! No shader editor natively (yet): every method does nothing.

use winit::event_loop::EventLoopProxy;

use crate::UserEvent;
use render::ShaderError;

pub struct Editor;

impl Editor {
    pub fn new(_proxy: EventLoopProxy<UserEvent>) -> Result<Self, String> {
        Ok(Self)
    }

    pub fn set_visuals(&self, _names: &[&str], _builtin_count: usize) {}

    pub fn show(&self, _visual: usize, _source: &str) {}

    pub fn set_error(&self, _error: Option<&ShaderError>) {}

    pub fn saved_source(&self, _name: &str) -> Option<String> {
        None
    }

    pub fn save_source(&self, _name: &str, _source: &str) {}

    pub fn forget_source(&self, _name: &str) {}

    pub fn saved_visuals(&self) -> Vec<String> {
        Vec::new()
    }

    pub fn save_visuals(&self, _names: &[&str]) {}
}
