//! Small streaming JSON grammar. Tokens and nesting have explicit limits. The
//! decoder consumes complete HTTP bodies transactionally and ignores unknown
//! fields without constructing a JSON tree or a complete response buffer.
use super::ParseError;
use heapless::{String, Vec};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Role {
    Root,
    Data,
    Results,
    Series,
    Labels,
    Values,
    Pair,
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Key {
    Status,
    Data,
    ResultType,
    Result,
    Metric,
    Value,
    Values,
    Other,
}
impl Key {
    fn parse(s: &str) -> Self {
        match s {
            "status" => Self::Status,
            "data" => Self::Data,
            "resultType" => Self::ResultType,
            "result" => Self::Result,
            "metric" => Self::Metric,
            "value" => Self::Value,
            "values" => Self::Values,
            _ => Self::Other,
        }
    }
}
#[derive(Clone, Copy)]
struct Frame {
    role: Role,
    object: bool,
    state: u8,
    key: Key,
    had_item: bool,
}
pub(super) trait Events {
    fn begin(&mut self, role: Role) -> Result<(), ParseError>;
    fn key(&mut self, role: Role, key: &str) -> Result<(), ParseError>;
    fn primitive(
        &mut self,
        role: Role,
        key: Key,
        string: bool,
        value: &str,
    ) -> Result<(), ParseError>;
    fn end(&mut self, role: Role) -> Result<(), ParseError>;
}
pub(super) struct Decoder {
    stack: Vec<Frame, 16>,
    token: Vec<u8, 1024>,
    in_string: bool,
    escape: bool,
    root: bool,
    done: bool,
}
impl Decoder {
    pub const fn new() -> Self {
        Self {
            stack: Vec::new(),
            token: Vec::new(),
            in_string: false,
            escape: false,
            root: false,
            done: false,
        }
    }
    pub fn reset(&mut self) {
        self.stack.clear();
        self.token.clear();
        self.in_string = false;
        self.escape = false;
        self.root = false;
        self.done = false;
    }
    pub fn byte(&mut self, b: u8, out: &mut impl Events) -> Result<(), ParseError> {
        if self.in_string {
            self.token.push(b).map_err(|_| ParseError)?;
            if self.escape {
                self.escape = false;
            } else if b == b'\\' {
                self.escape = true;
            } else if b == b'"' {
                self.in_string = false;
                let decoded: String<512> =
                    serde_json::from_slice(&self.token).map_err(|_| ParseError)?;
                self.token.clear();
                self.string(&decoded, out)?;
            }
            return Ok(());
        }
        if !self.token.is_empty() {
            if b.is_ascii_whitespace() || matches!(b, b',' | b']' | b'}') {
                let (role, key) = self.value()?;
                let value = core::str::from_utf8(&self.token).map_err(|_| ParseError)?;
                // serde_json checks JSON number/literal grammar, including leading
                // zeros and truncated exponents. Non-finite values use strings.
                if !matches!(value, "true" | "false" | "null") {
                    let _: f64 = serde_json::from_str(value).map_err(|_| ParseError)?;
                }
                out.primitive(role, key, false, value)?;
                self.token.clear();
            } else {
                self.token.push(b).map_err(|_| ParseError)?;
                return Ok(());
            }
        }
        if b.is_ascii_whitespace() {
            return Ok(());
        }
        if self.done {
            return Err(ParseError);
        }
        match b {
            b'"' => {
                self.in_string = true;
                self.token.push(b).map_err(|_| ParseError)?;
            }
            b'{' | b'[' => {
                let object = b == b'{';
                let role = if !self.root {
                    if !object {
                        return Err(ParseError);
                    }
                    self.root = true;
                    Role::Root
                } else {
                    let (role, key) = self.value()?;
                    match (role, key, object) {
                        (Role::Root, Key::Data, true) => Role::Data,
                        (Role::Data, Key::Result, false) => Role::Results,
                        (Role::Results, _, true) => Role::Series,
                        (Role::Results, _, false) => return Err(ParseError),
                        (Role::Series, Key::Metric, true) => Role::Labels,
                        (Role::Series, Key::Value, false) => Role::Pair,
                        (Role::Series, Key::Values, false) => Role::Values,
                        (Role::Values, _, false) => Role::Pair,
                        // A recognized field with the wrong type is never silently
                        // treated as an empty successful result.
                        (Role::Root, Key::Status, _)
                        | (Role::Data, Key::ResultType, _)
                        | (Role::Root, Key::Data, _)
                        | (Role::Data, Key::Result, _)
                        | (Role::Series, Key::Metric | Key::Value | Key::Values, _)
                        | (Role::Values, _, _) => return Err(ParseError),
                        _ => Role::Unknown,
                    }
                };
                out.begin(role)?;
                self.stack
                    .push(Frame {
                        role,
                        object,
                        state: 0,
                        key: Key::Other,
                        had_item: false,
                    })
                    .map_err(|_| ParseError)?;
            }
            b'}' | b']' => {
                let frame = self.stack.pop().ok_or(ParseError)?;
                if frame.object != (b == b'}')
                    || (frame.object && frame.state != 0 && frame.state != 3)
                    || (!frame.object && frame.state != 0 && frame.state != 1)
                    || (frame.state == 0 && frame.had_item)
                {
                    return Err(ParseError);
                }
                out.end(frame.role)?;
                if self.stack.is_empty() {
                    self.done = true;
                }
            }
            b':' => {
                let frame = self.stack.last_mut().ok_or(ParseError)?;
                if !frame.object || frame.state != 1 {
                    return Err(ParseError);
                }
                frame.state = 2;
            }
            b',' => {
                let frame = self.stack.last_mut().ok_or(ParseError)?;
                if frame.state != if frame.object { 3 } else { 1 } {
                    return Err(ParseError);
                }
                frame.state = 0;
                frame.had_item = true;
            }
            b'-' | b'0'..=b'9' | b't' | b'f' | b'n' => {
                self.token.push(b).map_err(|_| ParseError)?;
            }
            _ => return Err(ParseError),
        }
        Ok(())
    }
    fn string(&mut self, s: &str, out: &mut impl Events) -> Result<(), ParseError> {
        if let Some(frame) = self.stack.last_mut()
            && frame.object
            && frame.state == 0
        {
            frame.key = Key::parse(s);
            frame.state = 1;
            return out.key(frame.role, s);
        }
        let (role, key) = self.value()?;
        out.primitive(role, key, true, s)
    }
    fn value(&mut self) -> Result<(Role, Key), ParseError> {
        let frame = self.stack.last_mut().ok_or(ParseError)?;
        if frame.object {
            if frame.state != 2 {
                return Err(ParseError);
            }
            frame.state = 3;
        } else {
            if frame.state != 0 {
                return Err(ParseError);
            }
            frame.state = 1;
        }
        frame.had_item = false;
        Ok((frame.role, frame.key))
    }
    pub fn finish(&self) -> Result<(), ParseError> {
        if self.done && self.stack.is_empty() && self.token.is_empty() && !self.in_string {
            Ok(())
        } else {
            Err(ParseError)
        }
    }
}
