use egui_playground::{egui, run, App};

#[derive(Default)]
struct Hello {
    clicks: u32,
}

impl App for Hello {
    fn ui(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("Hello from the playground");
            if ui.button("click me").clicked() {
                self.clicks += 1;
            }
            ui.label(format!("{} clicks", self.clicks));
        });
    }
}

fn main() {
    run(Hello::default());
}
