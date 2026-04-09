use crate::{bindings, shifted_char};
use iced::{keyboard, mouse};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct AppKeyEvent {
    pub keycode: u32,
    pub mods: i32,
    pub text: Option<String>,
    pub modified_text: Option<String>,
    pub named_key: Option<bindings::NamedKey>,
    pub repeat: bool,
    pub input_seq: Option<u64>,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub enum AppMouseButton {
    Left,
    Right,
    Middle,
    Other,
}

impl AppMouseButton {
    pub fn to_iced(self) -> mouse::Button {
        match self {
            Self::Left => mouse::Button::Left,
            Self::Right => mouse::Button::Right,
            Self::Middle => mouse::Button::Middle,
            Self::Other => mouse::Button::Other(0),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub enum AppMouseEvent {
    CursorMoved {
        x: f64,
        y: f64,
        mods: i32,
    },
    ButtonPressed {
        button: AppMouseButton,
        x: f64,
        y: f64,
        mods: i32,
    },
    ButtonReleased {
        button: AppMouseButton,
        x: f64,
        y: f64,
        mods: i32,
    },
    WheelScrolledLines {
        x: f64,
        y: f64,
        mods: i32,
    },
    WheelScrolledPixels {
        x: f64,
        y: f64,
        mods: i32,
    },
}

impl AppKeyEvent {
    pub fn key_char(&self) -> Option<char> {
        shifted_char(self.keycode, self.mods)
            .or_else(|| self.text.as_deref().and_then(|s| s.chars().next()))
            .or_else(|| self.modified_text.as_deref().and_then(|s| s.chars().next()))
    }

    pub fn keyboard_key(&self) -> keyboard::Key {
        use keyboard::key::Named;

        if let Some(named) = self.named_key {
            let named = match named {
                bindings::NamedKey::ArrowUp => Named::ArrowUp,
                bindings::NamedKey::ArrowDown => Named::ArrowDown,
                bindings::NamedKey::ArrowLeft => Named::ArrowLeft,
                bindings::NamedKey::ArrowRight => Named::ArrowRight,
                bindings::NamedKey::PageUp => Named::PageUp,
                bindings::NamedKey::PageDown => Named::PageDown,
                bindings::NamedKey::Home => Named::Home,
                bindings::NamedKey::End => Named::End,
                bindings::NamedKey::Escape => Named::Escape,
            };
            return keyboard::Key::Named(named);
        }

        let text = self
            .modified_text
            .as_deref()
            .or(self.text.as_deref())
            .map(ToOwned::to_owned)
            .or_else(|| self.key_char().map(|ch| ch.to_string()))
            .unwrap_or_default();
        keyboard::Key::Character(text.into())
    }
}
