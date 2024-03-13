use crate::gui::MyApp;
impl MyApp {
    pub fn bottom_panel(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("bottom_panel")
        .resizable(true)
        .show(ctx, |ui| {
            ui.set_enabled(!self.import_opened);
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .id_source("bottom panel scroll")
                .show(ui, |ui| {
                    ui.label("world!");
                    ui.label("Hello");
                });
        });
    }
}