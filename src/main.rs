#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod engine;
mod gui;
use engine::logger;
use crate::gui::start_gui;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    logger::Logger::init()?;
    start_gui().unwrap();
    Ok(())
}