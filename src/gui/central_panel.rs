use crate::gui::MyApp;
impl MyApp {
    pub fn cenral_panel(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .id_source("table scroll")
                .min_scrolled_height(0.0)
                .show(ui, |ui| {
                    ui.set_enabled(!self.import_opened);
                    self.draw_table(ui, ctx);
                });
        });
    }
}