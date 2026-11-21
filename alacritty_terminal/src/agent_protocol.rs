use crate::event::{Event, EventListener};

const MAX_PAYLOAD: usize = 256;

#[derive(Clone, Copy, Default)]
enum Frame {
    #[default]
    Ground,
    Escape,
    Osc,
    OscEscape,
    String,
    StringEscape,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Status {
    state: &'static str,
    kind: Option<&'static str>,
}

pub(crate) struct AgentProtocol {
    frame: Frame,
    buffer: [u8; MAX_PAYLOAD],
    len: usize,
    invalid: bool,
    active: bool,
    pending: Option<Status>,
    last: Option<Status>,
}

impl Default for AgentProtocol {
    fn default() -> Self {
        Self {
            frame: Frame::Ground,
            buffer: [0; MAX_PAYLOAD],
            len: 0,
            invalid: false,
            active: false,
            pending: None,
            last: None,
        }
    }
}

impl AgentProtocol {
    pub(crate) fn advance<T: EventListener>(&mut self, listener: &T, bytes: &[u8]) {
        self.pending = None;
        for byte in bytes.iter().copied() {
            match self.frame {
                Frame::Ground if byte == 0x1b => self.frame = Frame::Escape,
                Frame::Escape => self.escape(byte),
                Frame::Osc => match byte {
                    0x07 => self.finish(),
                    0x1b => self.frame = Frame::OscEscape,
                    0x18 | 0x1a => {
                        self.clear_buffer();
                        self.frame = Frame::Ground;
                    },
                    0x20..=0x7e if self.len < MAX_PAYLOAD && !self.invalid => {
                        if self.len < 5 && byte != b"7501;"[self.len] {
                            self.invalid = true;
                        } else {
                            self.buffer[self.len] = byte;
                            self.len += 1;
                        }
                    },
                    _ => self.invalid = true,
                },
                Frame::OscEscape if byte == b'\\' => self.finish(),
                Frame::OscEscape => {
                    self.clear_buffer();
                    self.escape(byte);
                },
                Frame::String if byte == 0x1b => self.frame = Frame::StringEscape,
                Frame::String if byte == 0x18 || byte == 0x1a => self.frame = Frame::Ground,
                Frame::StringEscape if byte == b'\\' => self.frame = Frame::Ground,
                Frame::StringEscape if byte != 0x1b => self.frame = Frame::String,
                _ => (),
            }
        }
        if let Some(status) = self.pending {
            if self.last != Some(status) {
                self.last = Some(status);
                listener.send_event(Event::AgentState {
                    app: "codex".into(),
                    state: status.state.into(),
                    kind: status.kind.map(str::to_owned),
                });
            }
        }
    }

    fn escape(&mut self, byte: u8) {
        self.frame = match byte {
            b']' => {
                self.clear_buffer();
                Frame::Osc
            },
            b'P' | b'X' | b'^' | b'_' => Frame::String,
            0x1b => Frame::Escape,
            _ => Frame::Ground,
        };
    }

    fn finish(&mut self) {
        self.frame = Frame::Ground;
        let status = if self.invalid { None } else { self.parse() };
        self.clear_buffer();
        if let Some(status) = status {
            self.active = status.state != "clear";
            self.pending = Some(status);
        }
    }

    fn clear_buffer(&mut self) {
        self.buffer.fill(0);
        self.len = 0;
        self.invalid = false;
    }

    fn parse(&self) -> Option<Status> {
        let text = std::str::from_utf8(&self.buffer[..self.len]).ok()?;
        let text = text.strip_prefix("7501;")?;
        let mut app = None;
        let mut state = None;
        let mut kind = None;
        for field in text.split(':') {
            let (name, value) = field.split_once('=')?;
            match name {
                "app" if app.is_none() => app = Some(value),
                "state" if state.is_none() => state = Some(value),
                "kind" if kind.is_none() => kind = Some(value),
                _ => return None,
            }
        }
        if state == Some("clear") && app.is_none() && kind.is_none() && self.active {
            return Some(Status { state: "clear", kind: None });
        }
        if app != Some("codex") {
            return None;
        }
        let state = match state? {
            "idle" => "idle",
            "working" => "working",
            "done" => "done",
            "error" => "error",
            "clear" => "clear",
            "blocked" => "blocked",
            _ => return None,
        };
        let kind = match kind {
            Some("permission") if state == "blocked" => Some("permission"),
            Some("question") if state == "blocked" => Some("question"),
            Some("input") if state == "blocked" => Some("input"),
            None if state != "blocked" => None,
            _ => return None,
        };
        Some(Status { state, kind })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct Listener(Mutex<Vec<(String, String, Option<String>)>>);

    impl EventListener for Listener {
        fn send_event(&self, event: Event) {
            if let Event::AgentState { app, state, kind } = event {
                self.0.lock().unwrap().push((app, state, kind));
            }
        }
    }

    fn states(listener: &Listener) -> Vec<(String, Option<String>)> {
        listener
            .0
            .lock()
            .unwrap()
            .iter()
            .map(|(app, state, kind)| {
                assert_eq!(app, "codex");
                (state.clone(), kind.clone())
            })
            .collect()
    }

    #[test]
    fn actual_codex_states_survive_every_split_and_both_terminators() {
        for payload in [
            "state=idle:app=codex",
            "state=working:app=codex",
            "state=done:app=codex",
            "state=error:app=codex",
            "state=blocked:app=codex:kind=permission",
            "state=blocked:app=codex:kind=question",
        ] {
            for terminator in ["\x07", "\x1b\\"] {
                let input = format!("\x1b]7501;{payload}{terminator}");
                for split in 0..=input.len() {
                    let listener = Listener::default();
                    let mut parser = AgentProtocol::default();
                    parser.advance(&listener, &input.as_bytes()[..split]);
                    parser.advance(&listener, &input.as_bytes()[split..]);
                    assert_eq!(listener.0.lock().unwrap().len(), 1, "{payload} split={split}");
                }
            }
        }
    }

    #[test]
    fn fragmented_escapes_and_clear_require_a_known_agent() {
        let listener = Listener::default();
        let mut parser = AgentProtocol::default();
        parser.advance(&listener, b"\x1b]7501;state=clear\x1b\\");
        assert!(states(&listener).is_empty());
        for byte in b"\x1b]7501;state=working:app=codex\x1b\\\x1b]7501;state=clear\x1b\\" {
            parser.advance(&listener, &[*byte]);
        }
        assert_eq!(states(&listener), vec![("working".into(), None), ("clear".into(), None)]);
        parser.advance(&listener, b"\x1b]7501;state=clear\x1b\\");
        assert_eq!(states(&listener).len(), 2);
    }

    #[test]
    fn malformed_unknown_and_control_bearing_records_are_ignored() {
        let listener = Listener::default();
        let mut parser = AgentProtocol::default();
        for input in [
            "7501;state=working:app=codex",
            "\x1b]7501;state=done:app=other\x07",
            "\x1b]7501;state=done:app=codex:message=private\x07",
            "\x1b]7501;state=done:app=codex:app=codex\x07",
            "\x1b]7501;state=working:app=codex:kind=question\x07",
            "\x1b]7501;state=blocked:app=codex\x07",
            "\x1b]7501;state=blocked:app=codex:kind=unsupported\x07",
            "\x1b]7501;state=unknown:app=codex\x07",
            "\x1b]7501;state=working:app=co\ndex\x07",
            "\x1b]7501;state=done:app=codex\x18",
            "\x1b]7501;state=done:app=codex\x1b[0m",
        ] {
            parser.advance(&listener, input.as_bytes());
        }
        assert!(states(&listener).is_empty());
        parser.advance(&listener, b"\x1b]7501;state=done:app=codex\x07");
        assert_eq!(states(&listener), vec![("done".into(), None)]);
    }

    #[test]
    fn oversized_and_nested_string_sequences_do_not_emit_or_retain_text() {
        let listener = Listener::default();
        let mut parser = AgentProtocol::default();
        let mut input = b"\x1b]7501;state=done:app=codex".to_vec();
        input.extend(std::iter::repeat_n(b'x', 100_000));
        input.extend_from_slice(b"\x07");
        parser.advance(&listener, &input);
        assert!(parser.len <= MAX_PAYLOAD);
        assert!(parser.buffer.iter().all(|byte| *byte == 0));
        assert!(states(&listener).is_empty());
        for prefix in [b'P', b'X', b'^', b'_'] {
            let mut input = vec![0x1b, prefix];
            input.extend_from_slice(b"ignored\x1b]7501;state=done:app=codex\x07\x1b\\");
            parser.advance(&listener, &input);
        }
        assert!(states(&listener).is_empty());
        parser.advance(&listener, b"\x1b]broken\x1b]7501;state=working:app=codex\x1b\\");
        assert_eq!(states(&listener), vec![("working".into(), None)]);
    }

    #[test]
    fn repeated_frames_are_coalesced_without_changing_original_pty_bytes() {
        let listener = Listener::default();
        let mut parser = AgentProtocol::default();
        let input =
            b"text\x1b]7501;state=working:app=codex\x07\x1b]7501;state=done:app=codex\x07tail";
        let original = input.to_vec();
        parser.advance(&listener, input);
        parser.advance(&listener, b"\x1b]7501;state=done:app=codex\x07");
        assert_eq!(states(&listener), vec![("done".into(), None)]);
        assert_eq!(input.as_slice(), original);
    }
}
