//! What the host sends into the guest each frame.

use egui::{
    Event, Key, Modifiers, MouseWheelUnit, PointerButton, Pos2, RawInput, Rect, TouchDeviceId,
    TouchId, TouchPhase, ViewportId, ViewportInfo,
};

use crate::{read_header, write_header, DecodeError, Reader, Writer};

/// The part of an [`egui::RawInput`] that crosses into the guest, plus the scale to run at.
///
/// [`Event::AccessKitActionRequest`] and [`Event::Screenshot`] are dropped by [`encode_input`]:
/// they are produced by an integration, not by a user clicking in the guest view.
#[derive(Clone, Debug, PartialEq)]
pub struct Input {
    /// Guest-local coordinates, so `min` is zero.
    pub screen_rect: Rect,

    pub pixels_per_point: f32,

    /// Seconds since the host started.
    pub time: f64,

    pub predicted_dt: f32,

    /// Whether the guest view has keyboard focus in the host.
    pub focused: bool,

    pub max_texture_side: usize,

    pub events: Vec<Event>,
}

impl Default for Input {
    fn default() -> Self {
        Self {
            screen_rect: Rect::from_min_size(Pos2::ZERO, egui::vec2(320.0, 240.0)),
            pixels_per_point: 1.0,
            time: 0.0,
            predicted_dt: 1.0 / 60.0,
            focused: true,
            max_texture_side: 8192,
            events: Vec::new(),
        }
    }
}

impl Input {
    /// Builds the [`RawInput`] to hand to [`egui::Context::run`].
    pub fn to_raw_input(&self) -> RawInput {
        let mut viewport = ViewportInfo::default();
        viewport.native_pixels_per_point = Some(self.pixels_per_point);
        viewport.focused = Some(self.focused);

        RawInput {
            viewport_id: ViewportId::ROOT,
            viewports: std::iter::once((ViewportId::ROOT, viewport)).collect(),
            screen_rect: Some(self.screen_rect),
            max_texture_side: Some(self.max_texture_side),
            time: Some(self.time),
            predicted_dt: self.predicted_dt,
            events: self.events.clone(),
            focused: self.focused,
            ..Default::default()
        }
    }
}

pub fn encode_input(input: &Input, out: &mut Vec<u8>) {
    let w = &mut Writer::new(out);
    write_header(w);
    w.rect(input.screen_rect);
    w.f32(input.pixels_per_point);
    w.f64(input.time);
    w.f32(input.predicted_dt);
    w.bool(input.focused);
    w.usize(input.max_texture_side);

    let events: Vec<&Event> = input.events.iter().filter(|e| is_encodable(e)).collect();
    w.usize(events.len());
    for event in events {
        write_event(w, event);
    }
}

pub fn decode_input(bytes: &[u8]) -> Result<Input, DecodeError> {
    let r = &mut Reader::new(bytes);
    read_header(r)?;
    let screen_rect = r.rect()?;
    let pixels_per_point = r.f32()?;
    let time = r.f64()?;
    let predicted_dt = r.f32()?;
    let focused = r.bool()?;
    let max_texture_side = r.usize()?;

    let count = r.usize()?;
    let mut events = Vec::with_capacity(count.min(4096));
    for _ in 0..count {
        events.push(read_event(r)?);
    }

    Ok(Input {
        screen_rect,
        pixels_per_point,
        time,
        predicted_dt,
        focused,
        max_texture_side,
        events,
    })
}

fn is_encodable(event: &Event) -> bool {
    !matches!(
        event,
        Event::AccessKitActionRequest(_) | Event::Screenshot { .. }
    )
}

fn write_modifiers(w: &mut Writer<'_>, m: Modifiers) {
    let bits = (m.alt as u8)
        | (m.ctrl as u8) << 1
        | (m.shift as u8) << 2
        | (m.mac_cmd as u8) << 3
        | (m.command as u8) << 4;
    w.u8(bits);
}

fn read_modifiers(r: &mut Reader<'_>) -> Result<Modifiers, DecodeError> {
    let bits = r.u8()?;
    Ok(Modifiers {
        alt: bits & 1 != 0,
        ctrl: bits & 2 != 0,
        shift: bits & 4 != 0,
        mac_cmd: bits & 8 != 0,
        command: bits & 16 != 0,
    })
}

fn write_key(w: &mut Writer<'_>, key: Key) {
    w.str(key.name());
}

fn read_key(r: &mut Reader<'_>) -> Result<Key, DecodeError> {
    Key::from_name(r.str()?).ok_or(DecodeError::UnknownKey)
}

fn write_button(w: &mut Writer<'_>, button: PointerButton) {
    w.u8(match button {
        PointerButton::Primary => 0,
        PointerButton::Secondary => 1,
        PointerButton::Middle => 2,
        PointerButton::Extra1 => 3,
        PointerButton::Extra2 => 4,
    });
}

fn read_button(r: &mut Reader<'_>) -> Result<PointerButton, DecodeError> {
    Ok(match r.u8()? {
        0 => PointerButton::Primary,
        1 => PointerButton::Secondary,
        2 => PointerButton::Middle,
        3 => PointerButton::Extra1,
        4 => PointerButton::Extra2,
        tag => {
            return Err(DecodeError::BadTag {
                what: "PointerButton",
                tag: tag as u32,
            })
        }
    })
}

fn write_touch_phase(w: &mut Writer<'_>, phase: TouchPhase) {
    w.u8(match phase {
        TouchPhase::Start => 0,
        TouchPhase::Move => 1,
        TouchPhase::End => 2,
        TouchPhase::Cancel => 3,
    });
}

fn read_touch_phase(r: &mut Reader<'_>) -> Result<TouchPhase, DecodeError> {
    Ok(match r.u8()? {
        0 => TouchPhase::Start,
        1 => TouchPhase::Move,
        2 => TouchPhase::End,
        3 => TouchPhase::Cancel,
        tag => {
            return Err(DecodeError::BadTag {
                what: "TouchPhase",
                tag: tag as u32,
            })
        }
    })
}

fn write_wheel_unit(w: &mut Writer<'_>, unit: MouseWheelUnit) {
    w.u8(match unit {
        MouseWheelUnit::Point => 0,
        MouseWheelUnit::Line => 1,
        MouseWheelUnit::Page => 2,
    });
}

fn read_wheel_unit(r: &mut Reader<'_>) -> Result<MouseWheelUnit, DecodeError> {
    Ok(match r.u8()? {
        0 => MouseWheelUnit::Point,
        1 => MouseWheelUnit::Line,
        2 => MouseWheelUnit::Page,
        tag => {
            return Err(DecodeError::BadTag {
                what: "MouseWheelUnit",
                tag: tag as u32,
            })
        }
    })
}

fn write_event(w: &mut Writer<'_>, event: &Event) {
    match event {
        Event::Copy => w.u8(0),
        Event::Cut => w.u8(1),
        Event::Paste(text) => {
            w.u8(2);
            w.str(text);
        }
        Event::Text(text) => {
            w.u8(3);
            w.str(text);
        }
        Event::Key {
            key,
            physical_key,
            pressed,
            repeat,
            modifiers,
        } => {
            w.u8(4);
            write_key(w, *key);
            w.option(*physical_key, write_key);
            w.bool(*pressed);
            w.bool(*repeat);
            write_modifiers(w, *modifiers);
        }
        Event::ModifiersChanged(modifiers) => {
            w.u8(5);
            write_modifiers(w, *modifiers);
        }
        Event::PointerMoved(pos) => {
            w.u8(6);
            w.pos2(*pos);
        }
        Event::MouseMoved(delta) => {
            w.u8(7);
            w.vec2(*delta);
        }
        Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers,
        } => {
            w.u8(8);
            w.pos2(*pos);
            write_button(w, *button);
            w.bool(*pressed);
            write_modifiers(w, *modifiers);
        }
        Event::PointerGone => w.u8(9),
        Event::Zoom(factor) => {
            w.u8(10);
            w.f32(*factor);
        }
        Event::Rotate(radians) => {
            w.u8(11);
            w.f32(*radians);
        }
        Event::Ime(ime) => {
            w.u8(12);
            write_ime(w, ime);
        }
        Event::Touch {
            device_id,
            id,
            phase,
            pos,
            force,
        } => {
            w.u8(13);
            w.u64(device_id.0);
            w.u64(id.0);
            write_touch_phase(w, *phase);
            w.pos2(*pos);
            w.option(*force, |w, force| w.f32(force));
        }
        Event::MouseWheel {
            unit,
            delta,
            phase,
            modifiers,
        } => {
            w.u8(14);
            write_wheel_unit(w, *unit);
            w.vec2(*delta);
            write_touch_phase(w, *phase);
            write_modifiers(w, *modifiers);
        }
        Event::WindowFocused(focused) => {
            w.u8(15);
            w.bool(*focused);
        }
        Event::AccessKitActionRequest(_) | Event::Screenshot { .. } => {
            unreachable!("filtered out by is_encodable")
        }
    }
}

fn read_event(r: &mut Reader<'_>) -> Result<Event, DecodeError> {
    Ok(match r.u8()? {
        0 => Event::Copy,
        1 => Event::Cut,
        2 => Event::Paste(r.string()?),
        3 => Event::Text(r.string()?),
        4 => Event::Key {
            key: read_key(r)?,
            physical_key: r.option(read_key)?,
            pressed: r.bool()?,
            repeat: r.bool()?,
            modifiers: read_modifiers(r)?,
        },
        5 => Event::ModifiersChanged(read_modifiers(r)?),
        6 => Event::PointerMoved(r.pos2()?),
        7 => Event::MouseMoved(r.vec2()?),
        8 => Event::PointerButton {
            pos: r.pos2()?,
            button: read_button(r)?,
            pressed: r.bool()?,
            modifiers: read_modifiers(r)?,
        },
        9 => Event::PointerGone,
        10 => Event::Zoom(r.f32()?),
        11 => Event::Rotate(r.f32()?),
        12 => Event::Ime(read_ime(r)?),
        13 => Event::Touch {
            device_id: TouchDeviceId(r.u64()?),
            id: TouchId(r.u64()?),
            phase: read_touch_phase(r)?,
            pos: r.pos2()?,
            force: r.option(|r| r.f32())?,
        },
        14 => Event::MouseWheel {
            unit: read_wheel_unit(r)?,
            delta: r.vec2()?,
            phase: read_touch_phase(r)?,
            modifiers: read_modifiers(r)?,
        },
        15 => Event::WindowFocused(r.bool()?),
        tag => {
            return Err(DecodeError::BadTag {
                what: "Event",
                tag: tag as u32,
            })
        }
    })
}

/// The deprecated `Enabled` and `Disabled` variants are written as `Commit("")`.
fn write_ime(w: &mut Writer<'_>, ime: &egui::ImeEvent) {
    match ime {
        egui::ImeEvent::Preedit {
            text,
            active_range_chars,
        } => {
            w.u8(0);
            w.str(text);
            w.option(active_range_chars.clone(), |w, range| {
                w.usize(range.start);
                w.usize(range.end);
            });
        }
        egui::ImeEvent::Commit(text) => {
            w.u8(1);
            w.str(text);
        }
        egui::ImeEvent::DeleteSurrounding {
            before_chars,
            after_chars,
        } => {
            w.u8(2);
            w.usize(*before_chars);
            w.usize(*after_chars);
        }
        _ => {
            w.u8(1);
            w.str("");
        }
    }
}

fn read_ime(r: &mut Reader<'_>) -> Result<egui::ImeEvent, DecodeError> {
    Ok(match r.u8()? {
        0 => egui::ImeEvent::Preedit {
            text: r.string()?,
            active_range_chars: r.option(|r| Ok(r.usize()?..r.usize()?))?,
        },
        1 => egui::ImeEvent::Commit(r.string()?),
        2 => egui::ImeEvent::DeleteSurrounding {
            before_chars: r.usize()?,
            after_chars: r.usize()?,
        },
        tag => {
            return Err(DecodeError::BadTag {
                what: "ImeEvent",
                tag: tag as u32,
            })
        }
    })
}
