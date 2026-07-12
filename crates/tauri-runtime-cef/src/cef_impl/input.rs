use cef::{ImplBrowserHost, KeyEvent, KeyEventType, MouseButtonType, MouseEvent};
use winit::event::{ButtonSource, ElementState, MouseScrollDelta, WindowEvent};
use winit::keyboard::{KeyCode, PhysicalKey};

#[allow(dead_code)]
const EVENTFLAG_CAPS_LOCK_ON: u32 = 1;
const EVENTFLAG_SHIFT_DOWN: u32 = 2;
const EVENTFLAG_CONTROL_DOWN: u32 = 4;
const EVENTFLAG_ALT_DOWN: u32 = 8;
const EVENTFLAG_LEFT_MOUSE_BUTTON: u32 = 16;
const EVENTFLAG_MIDDLE_MOUSE_BUTTON: u32 = 32;
const EVENTFLAG_RIGHT_MOUSE_BUTTON: u32 = 64;
const EVENTFLAG_COMMAND_DOWN: u32 = 128;
#[allow(dead_code)]
const EVENTFLAG_NUM_LOCK_ON: u32 = 256;
const EVENTFLAG_IS_KEY_PAD: u32 = 512;
#[allow(dead_code)]
const EVENTFLAG_IS_LEFT: u32 = 1024;
#[allow(dead_code)]
const EVENTFLAG_IS_RIGHT: u32 = 2048;
#[allow(dead_code)]
const EVENTFLAG_ALTGR_DOWN: u32 = 4096;
const EVENTFLAG_IS_REPEAT: u32 = 8192;

pub(crate) fn forward_to_host(
  host: &cef::BrowserHost,
  event: &WindowEvent,
  modifiers: winit::event::Modifiers,
  _scale_factor: f64,
) {
  match event {
    WindowEvent::PointerMoved { position, .. } => {
      let me = MouseEvent {
        x: position.x.round() as i32,
        y: position.y.round() as i32,
        modifiers: winit_modifiers_to_cef(modifiers, 0),
      };
      host.send_mouse_move_event(Some(&me), 0);
    }
    WindowEvent::PointerLeft { position, kind, .. } => {
      let pos = position.unwrap_or(winit::dpi::PhysicalPosition::new(0.0, 0.0));
      let me = MouseEvent {
        x: pos.x.round() as i32,
        y: pos.y.round() as i32,
        modifiers: winit_modifiers_to_cef(modifiers, 0),
      };
      let _ = kind;
      host.send_mouse_move_event(Some(&me), 1);
    }
    WindowEvent::PointerButton {
      state,
      position,
      button,
      ..
    } => {
      let (cef_button, button_flag) = match button {
        ButtonSource::Mouse(MouseButton::Left) | ButtonSource::Touch { .. } => {
          (MouseButtonType::LEFT, EVENTFLAG_LEFT_MOUSE_BUTTON)
        }
        ButtonSource::Mouse(MouseButton::Right) => {
          (MouseButtonType::RIGHT, EVENTFLAG_RIGHT_MOUSE_BUTTON)
        }
        ButtonSource::Mouse(MouseButton::Middle) => {
          (MouseButtonType::MIDDLE, EVENTFLAG_MIDDLE_MOUSE_BUTTON)
        }
        _ => return,
      };
      let me = MouseEvent {
        x: position.x.round() as i32,
        y: position.y.round() as i32,
        modifiers: winit_modifiers_to_cef(modifiers, button_flag),
      };
      let mouse_up = match state {
        ElementState::Released => 1,
        ElementState::Pressed => 0,
      };
      host.send_mouse_click_event(Some(&me), cef_button, mouse_up, 1);
    }
    WindowEvent::MouseWheel { delta, .. } => {
      let me = MouseEvent {
        x: 0,
        y: 0,
        modifiers: winit_modifiers_to_cef(modifiers, 0),
      };
      let (dx, dy) = match delta {
        MouseScrollDelta::LineDelta(x, y) => ((x * 100.0) as i32, (y * 100.0) as i32),
        MouseScrollDelta::PixelDelta(p) => (p.x as i32, p.y as i32),
      };
      host.send_mouse_wheel_event(Some(&me), dx, dy);
    }
    WindowEvent::KeyboardInput { event, .. } => {
      let ke = build_key_event(event, modifiers);
      host.send_key_event(Some(&ke));
    }
    WindowEvent::Focused(focused) => {
      host.set_focus(if *focused { 1 } else { 0 });
    }
    _ => {}
  }
}

fn build_key_event(event: &winit::event::KeyEvent, modifiers: winit::event::Modifiers) -> KeyEvent {
  let (windows_key_code, native_key_code, is_keypad) = match event.physical_key {
    PhysicalKey::Code(code) => code_to_native(code),
    _ => (0, 0, false),
  };

  let type_ = match event.state {
    ElementState::Pressed => KeyEventType::KEYDOWN,
    ElementState::Released => KeyEventType::KEYUP,
  };

  let character = event
    .text
    .as_ref()
    .and_then(|t| t.chars().next())
    .unwrap_or('\0');
  let unmodified_character = event
    .key_without_modifiers
    .to_text()
    .and_then(|s| s.chars().next())
    .unwrap_or('\0');

  let mut mods = winit_modifiers_to_cef(modifiers, 0);
  if is_keypad {
    mods |= EVENTFLAG_IS_KEY_PAD;
  }
  if event.repeat {
    mods |= EVENTFLAG_IS_REPEAT;
  }

  KeyEvent {
    type_,
    modifiers: mods,
    windows_key_code,
    native_key_code,
    is_system_key: 0,
    character: char16_from_char(character),
    unmodified_character: char16_from_char(unmodified_character),
    focus_on_editable_field: 0,
    ..Default::default()
  }
}

fn char16_from_char(c: char) -> u16 {
  let mut buf = [0u16; 2];
  let encoded = c.encode_utf16(&mut buf);
  encoded[0]
}

fn winit_modifiers_to_cef(m: winit::event::Modifiers, button_flag: u32) -> u32 {
  let state = m.state();
  let mut flags = button_flag;
  if state.shift_key() {
    flags |= EVENTFLAG_SHIFT_DOWN;
  }
  if state.control_key() {
    flags |= EVENTFLAG_CONTROL_DOWN;
  }
  if state.alt_key() {
    flags |= EVENTFLAG_ALT_DOWN;
  }
  if state.meta_key() {
    flags |= EVENTFLAG_COMMAND_DOWN;
  }
  flags
}

fn code_to_native(code: KeyCode) -> (i32, i32, bool) {
  use winit::keyboard::KeyCode::*;
  let native = match code {
    Escape => 1,
    Digit1 => 2,
    Digit2 => 3,
    Digit3 => 4,
    Digit4 => 5,
    Digit5 => 6,
    Digit6 => 7,
    Digit7 => 8,
    Digit8 => 9,
    Digit9 => 10,
    Digit0 => 11,
    Minus => 12,
    Equal => 13,
    Backspace => 14,
    Tab => 15,
    KeyQ => 16,
    KeyW => 17,
    KeyE => 18,
    KeyR => 19,
    KeyT => 20,
    KeyY => 21,
    KeyU => 22,
    KeyI => 23,
    KeyO => 24,
    KeyP => 25,
    BracketLeft => 26,
    BracketRight => 27,
    Enter => 28,
    ControlLeft => 29,
    KeyA => 30,
    KeyS => 31,
    KeyD => 32,
    KeyF => 33,
    KeyG => 34,
    KeyH => 35,
    KeyJ => 36,
    KeyK => 37,
    KeyL => 38,
    Semicolon => 39,
    Quote => 40,
    Backquote => 41,
    ShiftLeft => 42,
    Backslash => 43,
    KeyZ => 44,
    KeyX => 45,
    KeyC => 46,
    KeyV => 47,
    KeyB => 48,
    KeyN => 49,
    KeyM => 50,
    Comma => 51,
    Period => 52,
    Slash => 53,
    ShiftRight => 54,
    NumpadMultiply => 55,
    AltLeft => 56,
    Space => 57,
    CapsLock => 58,
    F1 => 59,
    F2 => 60,
    F3 => 61,
    F4 => 62,
    F5 => 63,
    F6 => 64,
    F7 => 65,
    F8 => 66,
    F9 => 67,
    F10 => 68,
    ScrollLock => 70,
    Numpad7 => 71,
    Numpad8 => 72,
    Numpad9 => 73,
    NumpadSubtract => 74,
    Numpad4 => 75,
    Numpad5 => 76,
    Numpad6 => 77,
    NumpadAdd => 78,
    Numpad1 => 79,
    Numpad2 => 80,
    Numpad3 => 81,
    Numpad0 => 82,
    NumpadDecimal => 83,
    PrintScreen => 84,
    IntlBackslash => 86,
    F11 => 87,
    F12 => 88,
    F13 => 183,
    F14 => 184,
    F15 => 185,
    F16 => 186,
    F17 => 187,
    F18 => 188,
    F19 => 189,
    F20 => 190,
    F21 => 191,
    F22 => 192,
    F23 => 193,
    F24 => 194,
    KanaMode => 90,
    Lang3 => 91,
    Lang4 => 92,
    Lang5 => 93,
    IntlRo => 97,
    Convert => 121,
    NonConvert => 123,
    NumpadEnter => 96,
    ControlRight => 97,
    NumpadDivide => 98,
    AltRight => 100,
    NumLock => 69,
    Pause => 119,
    Home => 102,
    ArrowUp => 103,
    PageUp => 104,
    ArrowLeft => 105,
    ArrowRight => 106,
    End => 107,
    ArrowDown => 108,
    PageDown => 109,
    Insert => 110,
    Delete => 111,
    MetaLeft => 125,
    MetaRight => 126,
    ContextMenu => 127,
    Power => 116,
    Sleep => 142,
    WakeUp => 143,
    MediaPlay => 200,
    MediaStop => 128,
    MediaTrackNext => 163,
    MediaTrackPrevious => 165,
    MediaSelect => 226,
    _ => 0,
  };
  let is_keypad = matches!(
    code,
    Numpad0
      | Numpad1
      | Numpad2
      | Numpad3
      | Numpad4
      | Numpad5
      | Numpad6
      | Numpad7
      | Numpad8
      | Numpad9
      | NumpadDecimal
      | NumpadEnter
      | NumpadSubtract
      | NumpadAdd
      | NumpadMultiply
      | NumpadDivide
      | NumLock
  );
  (native, native, is_keypad)
}

use winit::event::MouseButton;
