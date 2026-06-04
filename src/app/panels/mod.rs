//! Sidebar / details panel render code, split out of `app.rs` for
//! readability. Each panel module owns the rendering of one logical
//! region of the sidebar plus the helper methods it calls.

pub mod command_palette;
pub mod details;
pub mod roots_menu;
pub mod rules_section;
pub mod sections;
pub mod treemap_view;
