use std::collections::{HashMap, HashSet};

use gilrs::{Axis, Button, Gamepad, GamepadId, Gilrs};
use serde::Serialize;

const BUTTONS: [Button; 17] = [
    Button::South,
    Button::East,
    Button::West,
    Button::North,
    Button::LeftTrigger,
    Button::RightTrigger,
    Button::LeftTrigger2,
    Button::RightTrigger2,
    Button::Select,
    Button::Start,
    Button::LeftThumb,
    Button::RightThumb,
    Button::DPadUp,
    Button::DPadDown,
    Button::DPadLeft,
    Button::DPadRight,
    Button::Mode,
];

const AXES: [Axis; 4] = [
    Axis::LeftStickX,
    Axis::LeftStickY,
    Axis::RightStickX,
    Axis::RightStickY,
];

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GamepadButton {
    pub pressed: bool,
    pub touched: bool,
    pub value: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GamepadSnapshot {
    pub id: String,
    pub index: usize,
    pub connected: bool,
    pub mapping: String,
    pub timestamp: f64,
    pub axes: Vec<f32>,
    pub buttons: Vec<GamepadButton>,
}

pub enum GamepadEvent {
    Connected(GamepadSnapshot),
    Disconnected { index: usize, id: String },
}

struct Pad {
    index: usize,
    id: String,
}

pub struct Gamepads {
    gilrs: Option<Gilrs>,
    pads: HashMap<GamepadId, Pad>,
    json: String,
    revision: u64,
    last: Vec<GamepadSnapshot>,
}

impl Gamepads {
    pub fn new() -> Self {
        let gilrs = match Gilrs::new() {
            Ok(gilrs) => Some(gilrs),
            Err(gilrs::Error::NotImplemented(gilrs)) => {
                log::warn!("gamepad input is not available on this platform");
                Some(gilrs)
            }
            Err(error) => {
                log::warn!("gamepad input unavailable: {error}");
                None
            }
        };
        Self {
            gilrs,
            pads: HashMap::new(),
            json: "[]".to_string(),
            revision: 0,
            last: Vec::new(),
        }
    }

    pub fn pump(&mut self, timestamp: f64) -> Vec<GamepadEvent> {
        let Some(gilrs) = self.gilrs.as_mut() else {
            if !self.last.is_empty() {
                self.last.clear();
                self.json = "[]".to_string();
                self.revision = self.revision.wrapping_add(1);
            }
            return Vec::new();
        };
        while gilrs.next_event().is_some() {}
        gilrs.inc();

        let mut events = Vec::new();
        let mut seen = HashSet::new();
        let mut current = Vec::new();
        for (id, gamepad) in gilrs.gamepads() {
            seen.insert(id);
            let mut snapshot = snapshot(&gamepad, timestamp);
            let mut connected = false;
            let index = match self.pads.get(&id).map(|pad| pad.index) {
                Some(index) => index,
                None => {
                    let mut index = 0;
                    while self.pads.values().any(|pad| pad.index == index) {
                        index += 1;
                    }
                    self.pads.insert(
                        id,
                        Pad {
                            index,
                            id: snapshot.id.clone(),
                        },
                    );
                    connected = true;
                    index
                }
            };
            snapshot.index = index;
            if connected {
                events.push(GamepadEvent::Connected(snapshot.clone()));
            }
            current.push(snapshot);
        }

        let stale: Vec<GamepadId> = self
            .pads
            .keys()
            .copied()
            .filter(|id| !seen.contains(id))
            .collect();
        for id in stale {
            if let Some(pad) = self.pads.remove(&id) {
                events.push(GamepadEvent::Disconnected {
                    index: pad.index,
                    id: pad.id,
                });
            }
        }

        current.sort_by_key(|snapshot| snapshot.index);
        for (snapshot, previous) in current.iter_mut().zip(self.last.iter()) {
            if same_state(snapshot, previous) {
                snapshot.timestamp = previous.timestamp;
            }
        }
        if current != self.last {
            self.json = serde_json::to_string(&current).unwrap_or_else(|_| "[]".to_string());
            self.revision = self.revision.wrapping_add(1);
            self.last = current;
        }
        events
    }

    pub fn json(&self) -> String {
        self.json.clone()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }
}

fn same_state(left: &GamepadSnapshot, right: &GamepadSnapshot) -> bool {
    left.id == right.id
        && left.index == right.index
        && left.connected == right.connected
        && left.mapping == right.mapping
        && left.axes == right.axes
        && left.buttons == right.buttons
}

fn snapshot(gamepad: &Gamepad<'_>, timestamp: f64) -> GamepadSnapshot {
    GamepadSnapshot {
        id: gamepad_id(gamepad),
        index: 0,
        connected: true,
        mapping: "standard".to_string(),
        timestamp,
        axes: AXES
            .iter()
            .map(|axis| browser_axis(gamepad.value(*axis), *axis))
            .collect(),
        buttons: BUTTONS
            .iter()
            .map(|button| {
                let data = gamepad.button_data(*button);
                let value = data.map_or(0.0, |data| data.value().clamp(0.0, 1.0));
                let pressed = data.is_some_and(|data| data.is_pressed());
                GamepadButton {
                    pressed,
                    touched: pressed || value > 0.0,
                    value,
                }
            })
            .collect(),
    }
}

fn browser_axis(value: f32, axis: Axis) -> f32 {
    if matches!(axis, Axis::LeftStickY | Axis::RightStickY) {
        -value
    } else {
        value
    }
}

fn gamepad_id(gamepad: &Gamepad<'_>) -> String {
    standard_id(gamepad.name(), gamepad.vendor_id(), gamepad.product_id())
}

fn standard_id(name: &str, vendor: Option<u16>, product: Option<u16>) -> String {
    if name.contains("STANDARD GAMEPAD") {
        return name.to_string();
    }
    match (vendor, product) {
        (Some(vendor), Some(product)) => {
            format!("{name} (STANDARD GAMEPAD Vendor: {vendor:04x} Product: {product:04x})")
        }
        _ => format!("{name} (STANDARD GAMEPAD)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_serialize_the_browser_shape() {
        let snapshot = GamepadSnapshot {
            id: "Test (STANDARD GAMEPAD)".to_string(),
            index: 2,
            connected: true,
            mapping: "standard".to_string(),
            timestamp: 12.5,
            axes: vec![0.0, -1.0, 0.5, 1.0],
            buttons: vec![GamepadButton {
                pressed: true,
                touched: true,
                value: 1.0,
            }],
        };
        let json = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(json["index"], 2);
        assert_eq!(json["mapping"], "standard");
        assert_eq!(json["buttons"][0]["pressed"], true);
        assert_eq!(json["axes"][1], -1.0);
    }

    #[test]
    fn state_checks_ignore_timestamps() {
        let first = GamepadSnapshot {
            id: "Test (STANDARD GAMEPAD)".to_string(),
            index: 0,
            connected: true,
            mapping: "standard".to_string(),
            timestamp: 1.0,
            axes: vec![0.0, 0.0, 0.0, 0.0],
            buttons: vec![GamepadButton {
                pressed: false,
                touched: false,
                value: 0.0,
            }],
        };
        let mut second = first.clone();
        second.timestamp = 2.0;
        assert!(same_state(&first, &second));
        second.buttons[0].value = 0.5;
        assert!(!same_state(&first, &second));
    }

    #[test]
    fn stick_y_axes_follow_the_browser_direction() {
        assert_eq!(browser_axis(1.0, Axis::LeftStickX), 1.0);
        assert_eq!(browser_axis(1.0, Axis::LeftStickY), -1.0);
        assert_eq!(browser_axis(-1.0, Axis::RightStickY), 1.0);
    }

    #[test]
    fn gamepad_ids_use_the_browser_suffix() {
        assert_eq!(
            standard_id("DualSense Wireless Controller", Some(0x054c), Some(0x0ce6)),
            "DualSense Wireless Controller (STANDARD GAMEPAD Vendor: 054c Product: 0ce6)"
        );
        assert_eq!(
            standard_id("Test Pad", None, None),
            "Test Pad (STANDARD GAMEPAD)"
        );
        assert_eq!(
            standard_id(
                "Test (STANDARD GAMEPAD Vendor: 0001 Product: 0002)",
                None,
                None
            ),
            "Test (STANDARD GAMEPAD Vendor: 0001 Product: 0002)"
        );
    }
}
