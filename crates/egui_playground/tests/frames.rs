//! Drives an app through the exported entry points, the same way the host does.

use egui_playground::{egui, pg_alloc, pg_frame, run};
use pg_abi::{decode_output, encode_input, FrameOutput, Input};

fn run_frame(input: &Input) -> FrameOutput {
    let mut bytes = Vec::new();
    encode_input(input, &mut bytes);

    let ptr = pg_alloc(bytes.len());
    // SAFETY: `pg_alloc` gave us `bytes.len()` writable bytes.
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
    }

    // SAFETY: the buffer comes from `pg_alloc` and holds encoded input.
    let result = unsafe { pg_frame(ptr, bytes.len()) };

    // SAFETY: `pg_frame` returns a byte count followed by that many bytes.
    unsafe {
        let len = u32::from_le_bytes(std::slice::from_raw_parts(result, 4).try_into().unwrap());
        let payload = std::slice::from_raw_parts(result.add(4), len as usize);
        decode_output(payload).expect("decode output")
    }
}

fn click_at(pos: egui::Pos2) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::default(),
        },
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::default(),
        },
    ]
}

/// Sixty frames of a button app all paint, the font atlas arrives on the first frame, and a
/// synthetic click reaches the widget.
#[test]
fn sixty_frames_paint() {
    let mut clicks = 0;
    let counter = std::rc::Rc::new(std::cell::Cell::new(0));
    let seen = counter.clone();

    run(move |ui: &mut egui::Ui| {
        egui::CentralPanel::default().show(ui, |ui| {
            if ui.button("click me").clicked() {
                clicks += 1;
                counter.set(clicks);
            }
            ui.label(format!("{clicks} clicks"));
        });
    });

    let mut font_atlas_size = None;
    let mut painted = 0;

    for frame in 0..60 {
        let input = Input {
            screen_rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640.0, 480.0)),
            pixels_per_point: 2.0,
            time: frame as f64 / 60.0,
            events: match frame {
                10 => click_at(egui::pos2(30.0, 20.0)),
                _ => Vec::new(),
            },
            ..Default::default()
        };

        let mut output = run_frame(&input);

        let vertices: usize = output.primitives.iter().map(|(_, m)| m.vertices.len()).sum();
        if vertices > 0 {
            painted += 1;
        }

        if frame == 0 {
            font_atlas_size = output
                .textures_delta
                .set
                .get(&egui::TextureId::Managed(0))
                .and_then(|deltas| deltas.first())
                .map(|delta| delta.image.size());
        }

        // `TexturesDelta` asserts on drop that every delta was applied.
        output.textures_delta.clear();
    }

    assert_eq!(painted, 60, "every frame should paint something");
    assert!(
        font_atlas_size.is_some_and(|[w, h]| w > 0 && h > 0),
        "the first frame should upload the font atlas"
    );
    assert_eq!(seen.get(), 1, "the click should reach the button");
}
