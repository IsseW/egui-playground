use egui_playground::{egui, run, App};

struct Painter {
    frequency: f32,
    amplitude: f32,
    phase: f32,
}

impl Default for Painter {
    fn default() -> Self {
        Self {
            frequency: 3.0,
            amplitude: 0.8,
            phase: 0.0,
        }
    }
}

impl App for Painter {
    fn ui(&mut self, ui: &mut egui::Ui) {
        self.phase += ui.ctx().input(|i| i.stable_dt);
        ui.ctx().request_repaint();

        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::Slider::new(&mut self.frequency, 0.5..=20.0).text("frequency"));
                ui.add(egui::Slider::new(&mut self.amplitude, 0.0..=1.0).text("amplitude"));
            });

            let (response, painter) =
                ui.allocate_painter(ui.available_size(), egui::Sense::hover());
            let rect = response.rect;
            painter.rect_filled(rect, 4.0, egui::Color32::from_gray(20));

            let points: Vec<egui::Pos2> = (0..=200)
                .map(|step| {
                    let t = step as f32 / 200.0;
                    let y = (t * self.frequency * std::f32::consts::TAU + self.phase).sin()
                        * self.amplitude;
                    egui::pos2(
                        rect.left() + t * rect.width(),
                        rect.center().y - y * rect.height() * 0.5,
                    )
                })
                .collect();

            painter.add(egui::Shape::line(
                points,
                egui::Stroke::new(2.0, egui::Color32::from_rgb(120, 200, 255)),
            ));
        });
    }
}

fn main() {
    run(Painter::default());
}
