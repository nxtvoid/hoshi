//! hoshi's native components and theme. Like `@hoshi/ui` on the web, nothing here knows the
//! app: no engine, no `hoshi-core` types, no stores. Text and state come in from the caller.

pub mod assets;
mod components;
pub mod theme;

pub use components::*;

pub mod prelude {
    pub use crate::assets::IconName;
    pub use crate::components::*;
    pub use crate::theme::{ActiveTheme, Colors};
    pub use gpui_kit::prelude::FluentBuilder as _;
}

type ClickHandler = Box<dyn Fn(&gpui_kit::ClickEvent, &mut gpui_kit::Window, &mut gpui_kit::App)>;
