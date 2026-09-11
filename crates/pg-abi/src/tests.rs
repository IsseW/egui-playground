use egui::{
    epaint::{
        image::ImageDelta,
        textures::{TextureFilter, TextureOptions, TextureWrapMode, TexturesDelta},
        ColorImage, Mesh, Vertex,
    },
    pos2, vec2, Color32, CursorIcon, Event, ImeEvent, Key, Modifiers, MouseWheelUnit,
    PointerButton, Rect, TextureId, TouchDeviceId, TouchId, TouchPhase,
};

use crate::{decode_input, decode_output, encode_input, encode_output, FrameOutput, Input};

fn round_trip_input(input: &Input) -> Input {
    let mut bytes = Vec::new();
    encode_input(input, &mut bytes);
    decode_input(&bytes).expect("decode")
}

fn round_trip_output(output: &FrameOutput) -> FrameOutput {
    let mut bytes = Vec::new();
    encode_output(output, &mut bytes);
    decode_output(&bytes).expect("decode")
}

fn every_event() -> Vec<Event> {
    let modifiers = Modifiers {
        alt: true,
        ctrl: false,
        shift: true,
        mac_cmd: true,
        command: true,
    };
    vec![
        Event::Copy,
        Event::Cut,
        Event::Paste("pasted".to_owned()),
        Event::Text("typed".to_owned()),
        Event::Key {
            key: Key::ArrowDown,
            physical_key: Some(Key::S),
            pressed: true,
            repeat: true,
            modifiers,
        },
        Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: Modifiers::default(),
        },
        Event::ModifiersChanged(modifiers),
        Event::PointerMoved(pos2(3.0, 4.0)),
        Event::MouseMoved(vec2(-1.0, 2.5)),
        Event::PointerButton {
            pos: pos2(10.0, 20.0),
            button: PointerButton::Secondary,
            pressed: true,
            modifiers,
        },
        Event::PointerGone,
        Event::Zoom(1.25),
        Event::Rotate(0.5),
        Event::Ime(ImeEvent::Preedit {
            text: "pre".to_owned(),
            active_range_chars: Some(1..2),
        }),
        Event::Ime(ImeEvent::Commit("done".to_owned())),
        Event::Ime(ImeEvent::DeleteSurrounding {
            before_chars: 2,
            after_chars: 3,
        }),
        Event::Touch {
            device_id: TouchDeviceId(7),
            id: TouchId(9),
            phase: TouchPhase::Move,
            pos: pos2(1.0, 2.0),
            force: Some(0.75),
        },
        Event::MouseWheel {
            unit: MouseWheelUnit::Line,
            delta: vec2(0.0, -3.0),
            phase: TouchPhase::End,
            modifiers,
        },
        Event::WindowFocused(false),
    ]
}

fn font_atlas_delta() -> ImageDelta {
    ImageDelta::full(
        ColorImage::new([2, 2], vec![Color32::WHITE, Color32::RED, Color32::BLUE, Color32::TRANSPARENT]),
        TextureOptions {
            magnification: TextureFilter::Linear,
            minification: TextureFilter::Nearest,
            wrap_mode: TextureWrapMode::MirroredRepeat,
            mipmap_mode: Some(TextureFilter::Linear),
        },
    )
}

/// Every event the host can forward survives the trip into the guest.
#[test]
fn input_events_round_trip() {
    let input = Input {
        screen_rect: Rect::from_min_size(pos2(0.0, 0.0), vec2(640.0, 480.0)),
        pixels_per_point: 2.0,
        time: 12.5,
        predicted_dt: 1.0 / 120.0,
        focused: false,
        max_texture_side: 4096,
        events: every_event(),
    };

    assert_eq!(round_trip_input(&input), input);
}

/// Events an integration produces for itself are dropped rather than sent on.
#[test]
fn integration_only_events_are_dropped() {
    let input = Input {
        events: vec![
            Event::Screenshot {
                viewport_id: egui::ViewportId::ROOT,
                user_data: Default::default(),
                image: std::sync::Arc::new(ColorImage::example()),
            },
            Event::PointerGone,
        ],
        ..Default::default()
    };

    assert_eq!(round_trip_input(&input).events, vec![Event::PointerGone]);
}

/// An empty frame carries no meshes and no texture work.
#[test]
fn empty_output_round_trips() {
    let output = FrameOutput {
        pixels_per_point: 1.5,
        ..Default::default()
    };

    assert_eq!(round_trip_output(&output), output);
}

/// Meshes, texture uploads, texture frees and platform output come back unchanged.
#[test]
fn full_output_round_trips() {
    let mut mesh = Mesh::with_texture(TextureId::User(3));
    mesh.indices = vec![0, 1, 2];
    mesh.vertices = vec![
        Vertex {
            pos: pos2(0.0, 0.0),
            uv: pos2(0.0, 1.0),
            color: Color32::from_rgba_premultiplied(1, 2, 3, 4),
        },
        Vertex {
            pos: pos2(10.0, 0.0),
            uv: pos2(1.0, 1.0),
            color: Color32::WHITE,
        },
        Vertex {
            pos: pos2(0.0, 10.0),
            uv: pos2(0.0, 0.0),
            color: Color32::TRANSPARENT,
        },
    ];

    let mut textures_delta = TexturesDelta::default();
    textures_delta
        .set
        .entry(TextureId::Managed(0))
        .or_default()
        .push(font_atlas_delta());
    textures_delta
        .set
        .entry(TextureId::Managed(0))
        .or_default()
        .push(ImageDelta::partial(
            [1, 1],
            ColorImage::new([1, 1], vec![Color32::GREEN]),
            TextureOptions::default(),
        ));
    textures_delta.free.insert(TextureId::Managed(4));
    textures_delta.free.insert(TextureId::User(5));

    let output = FrameOutput {
        pixels_per_point: 2.0,
        primitives: vec![
            (Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 100.0)), mesh),
            (Rect::EVERYTHING, Mesh::default()),
        ],
        textures_delta,
        cursor_icon: CursorIcon::ResizeHorizontal,
        copied_text: Some("clipboard".to_owned()),
        open_url: Some("https://example.com".to_owned()),
        repaint_after_ms: Some(16),
    };

    let mut decoded = round_trip_output(&output);
    assert_eq!(decoded, output);

    // `TexturesDelta` asserts on drop that every delta was applied.
    decoded.textures_delta.clear();
    let mut output = output;
    output.textures_delta.clear();
}

/// A buffer from a different bridge version is rejected instead of being misread.
#[test]
fn version_mismatch_is_rejected() {
    let mut bytes = Vec::new();
    encode_input(&Input::default(), &mut bytes);
    bytes[4] = bytes[4].wrapping_add(1);

    assert!(matches!(
        decode_input(&bytes),
        Err(crate::DecodeError::VersionMismatch { .. })
    ));
}

/// A truncated buffer is reported rather than panicking.
#[test]
fn truncated_buffer_is_rejected() {
    let mut bytes = Vec::new();
    encode_output(&FrameOutput::default(), &mut bytes);
    bytes.truncate(bytes.len() - 1);

    assert_eq!(
        decode_output(&bytes),
        Err(crate::DecodeError::UnexpectedEnd)
    );
}
