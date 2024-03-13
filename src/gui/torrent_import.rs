use crate::gui::MyApp;
use crate::gui::TorrentInfo;
use egui::{ViewportBuilder, ViewportId};
use std::path::Path;
use egui::TextEdit;

use super::files_tree::draw_tree;
use crate::gui::get_readable_size;

impl MyApp {
    pub fn import_window(&mut self, ctx: &egui::Context) {
        ctx.show_viewport_immediate(
            ViewportId::from_hash_of("Import torrent window"),
            ViewportBuilder::default()
                .with_title("Import torrent")
                .with_inner_size([400.0, 300.0]),
            |ctx, _| {
                if ctx.input(|i| i.viewport().close_requested()) {
                    self.import_opened = false;
                }
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Save to:");
                        ui.spacing_mut().interact_size.y = 4.0;
                        ui.add(TextEdit::singleline(&mut self.import_dest_dir).desired_width(ui.available_width() / 1.5));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Max), |ui| {
                            if ui.button("Pick").clicked() {
                                let file = rfd::FileDialog::new().pick_folder();
                                if let Some(path) = file {
                                    self.import_dest_dir = path.to_str().unwrap().to_owned();
                                }
                            };
                        });
                    });

                    let dest_path = Path::new(&self.import_dest_dir);

                    // ui.add_space(5.0);
                    ui.separator();
                    // ui.add_space(5.0);
                    ui.label("Included files: ");
                    ui.spacing_mut().slider_width = ui.available_width() * 0.95;
                    egui::ScrollArea::vertical()
                        .max_height(ui.available_height() / 2.0)
                        .auto_shrink(false)
                        .show(ui, |ui| {
                            let torrent = self.import_torrent.as_ref().unwrap();
                            if let Some(files) = &torrent.info.files {
                                draw_tree(
                                    &files.iter().map(|x| x.path.as_str()).collect(),
                                    torrent.info.name.clone(),
                                    ui,
                                )
                            } else {
                                ui.label(&torrent.info.name);
                            }
                        });

                    ui.separator();
                    // ui.columns(2, |columns| {
                    //     columns[0].label("First column");
                    //     columns[1].with_layout(egui::Layout::left_to_right(egui::Align::Max), |ui| {
                    //         ui.label("Second column")
                    //     });
                    // });
                    let mut start_download = false;

                    // ui.with_layout(egui::Layout::left_to_right(egui::Align::BOTTOM), |ui| {
                        ui.horizontal(|ui| {
                            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                                ui.vertical(|ui| {
                                    ui.label(format!(
                                        "Required space: {}",
                                        get_readable_size(
                                            self.import_torrent.as_ref().unwrap().info.length as usize,
                                            2
                                        )
                                    ));
                                    let mut available_size = String::from("0");
                                    if dest_path.exists() {
                                        if let Ok(space) = fs2::available_space(dest_path) {
                                            available_size = get_readable_size(space as usize, 2);
                                        }
                                    }
                                    ui.label(format!("Available space: {}", available_size));
                                });
                            });
    
                            
                        
                            ui.with_layout(egui::Layout::bottom_up(egui::Align::RIGHT), |ui| {
                                let button_enabled = if !self.import_dest_dir.is_empty()
                                    && dest_path.exists()
                                    && dest_path.is_dir()
                                {
                                    true
                                } else {
                                    false
                                };
                                ui.set_enabled(button_enabled);
                                if ui.button("Start").clicked() {
                                    self.import_opened = false;
    
                                    start_download = true;
                                }
                            });
                        });
                    // });
                    if start_download {
                        self.start_download(
                            TorrentInfo::Torrent(
                                self.import_torrent.as_ref().unwrap().clone(),
                            ),
                            ctx,
                        );
                    }
                });
            },
        );
    }
}
