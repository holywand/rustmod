use std::fmt;

const MAX_DEPTH: usize = 512;

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsonError {
    pub offset: usize,
    pub message: String,
}

impl fmt::Display for JsonError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} at byte {}", self.message, self.offset)
    }
}

impl std::error::Error for JsonError {}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(entries) => entries
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Number(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Array(values) => Some(values),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&[(String, Json)]> {
        match self {
            Json::Object(entries) => Some(entries),
            _ => None,
        }
    }
}

pub fn parse(input: &str) -> Result<Json, JsonError> {
    let mut parser = Parser {
        input,
        bytes: input.as_bytes(),
        position: 0,
    };
    parser.skip_whitespace();
    let value = parser.value(0)?;
    parser.skip_whitespace();
    if parser.position != parser.bytes.len() {
        return Err(parser.error("trailing characters after document"));
    }
    Ok(value)
}

struct Parser<'a> {
    input: &'a str,
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Parser<'a> {
    fn error(&self, message: &str) -> JsonError {
        JsonError {
            offset: self.position,
            message: message.to_string(),
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.position).copied()
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.position += 1;
        }
    }

    fn expect(&mut self, expected: u8) -> Result<(), JsonError> {
        if self.peek() == Some(expected) {
            self.position += 1;
            Ok(())
        } else {
            Err(self.error(&format!("expected '{}'", expected as char)))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, JsonError> {
        if depth > MAX_DEPTH {
            return Err(self.error("nesting too deep"));
        }
        match self.peek() {
            None => Err(self.error("unexpected end of input")),
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => Ok(Json::String(self.string()?)),
            Some(b't') => self.literal("true", Json::Bool(true)),
            Some(b'f') => self.literal("false", Json::Bool(false)),
            Some(b'n') => self.literal("null", Json::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.error("unexpected character")),
        }
    }

    fn literal(&mut self, word: &str, value: Json) -> Result<Json, JsonError> {
        if self.input[self.position..].starts_with(word) {
            self.position += word.len();
            Ok(value)
        } else {
            Err(self.error("invalid literal"))
        }
    }

    fn object(&mut self, depth: usize) -> Result<Json, JsonError> {
        self.expect(b'{')?;
        let mut entries: Vec<(String, Json)> = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.position += 1;
            return Ok(Json::Object(entries));
        }
        loop {
            self.skip_whitespace();
            if self.peek() != Some(b'"') {
                return Err(self.error("expected object key"));
            }
            let key_offset = self.position;
            let key = self.string()?;
            if entries.iter().any(|(existing, _)| *existing == key) {
                return Err(JsonError {
                    offset: key_offset,
                    message: format!("duplicate key \"{key}\""),
                });
            }
            self.skip_whitespace();
            self.expect(b':')?;
            self.skip_whitespace();
            let value = self.value(depth + 1)?;
            entries.push((key, value));
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.position += 1,
                Some(b'}') => {
                    self.position += 1;
                    return Ok(Json::Object(entries));
                }
                _ => return Err(self.error("expected ',' or '}'")),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Json, JsonError> {
        self.expect(b'[')?;
        let mut values = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.position += 1;
            return Ok(Json::Array(values));
        }
        loop {
            self.skip_whitespace();
            values.push(self.value(depth + 1)?);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.position += 1,
                Some(b']') => {
                    self.position += 1;
                    return Ok(Json::Array(values));
                }
                _ => return Err(self.error("expected ',' or ']'")),
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, JsonError> {
        if self.position + 4 > self.bytes.len() {
            return Err(self.error("truncated unicode escape"));
        }
        let mut value = 0u32;
        for _ in 0..4 {
            let digit = match self.bytes[self.position] {
                byte @ b'0'..=b'9' => u32::from(byte - b'0'),
                byte @ b'a'..=b'f' => u32::from(byte - b'a') + 10,
                byte @ b'A'..=b'F' => u32::from(byte - b'A') + 10,
                _ => return Err(self.error("invalid unicode escape")),
            };
            value = value * 16 + digit;
            self.position += 1;
        }
        Ok(value)
    }

    fn string(&mut self) -> Result<String, JsonError> {
        self.expect(b'"')?;
        let mut output = String::new();
        loop {
            let start = self.position;
            while let Some(byte) = self.peek() {
                if byte == b'"' || byte == b'\\' || byte < 0x20 {
                    break;
                }
                self.position += 1;
            }
            output.push_str(&self.input[start..self.position]);
            match self.peek() {
                None => return Err(self.error("unterminated string")),
                Some(b'"') => {
                    self.position += 1;
                    return Ok(output);
                }
                Some(b'\\') => {
                    self.position += 1;
                    let escape = self
                        .peek()
                        .ok_or_else(|| self.error("unterminated escape"))?;
                    self.position += 1;
                    match escape {
                        b'"' => output.push('"'),
                        b'\\' => output.push('\\'),
                        b'/' => output.push('/'),
                        b'b' => output.push('\u{0008}'),
                        b'f' => output.push('\u{000C}'),
                        b'n' => output.push('\n'),
                        b'r' => output.push('\r'),
                        b't' => output.push('\t'),
                        b'u' => {
                            let first = self.hex4()?;
                            let scalar = if (0xD800..0xDC00).contains(&first) {
                                if !self.input[self.position..].starts_with("\\u") {
                                    return Err(self.error("unpaired high surrogate"));
                                }
                                self.position += 2;
                                let second = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&second) {
                                    return Err(self.error("invalid low surrogate"));
                                }
                                0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00)
                            } else if (0xDC00..0xE000).contains(&first) {
                                return Err(self.error("unpaired low surrogate"));
                            } else {
                                first
                            };
                            match char::from_u32(scalar) {
                                Some(character) => output.push(character),
                                None => return Err(self.error("invalid unicode scalar")),
                            }
                        }
                        _ => return Err(self.error("invalid escape")),
                    }
                }
                Some(_) => return Err(self.error("control character in string")),
            }
        }
    }

    fn digits(&mut self) -> Result<(), JsonError> {
        let start = self.position;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.position += 1;
        }
        if self.position == start {
            Err(self.error("expected digit"))
        } else {
            Ok(())
        }
    }

    fn number(&mut self) -> Result<Json, JsonError> {
        let start = self.position;
        if self.peek() == Some(b'-') {
            self.position += 1;
        }
        match self.peek() {
            Some(b'0') => self.position += 1,
            Some(b'1'..=b'9') => self.digits()?,
            _ => return Err(self.error("invalid number")),
        }
        if self.peek() == Some(b'.') {
            self.position += 1;
            self.digits()?;
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.position += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.position += 1;
            }
            self.digits()?;
        }
        let text = &self.input[start..self.position];
        match text.parse::<f64>() {
            Ok(value) if value.is_finite() => Ok(Json::Number(value)),
            _ => Err(JsonError {
                offset: start,
                message: "number out of range".to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scalars() {
        assert_eq!(parse("null").unwrap(), Json::Null);
        assert_eq!(parse(" true ").unwrap(), Json::Bool(true));
        assert_eq!(parse("false").unwrap(), Json::Bool(false));
        assert_eq!(parse("-12.5e2").unwrap(), Json::Number(-1250.0));
        assert_eq!(parse("0").unwrap(), Json::Number(0.0));
        assert_eq!(
            parse("0.16666666666666666").unwrap().as_f64().unwrap(),
            0.16666666666666666
        );
    }

    #[test]
    fn parses_nested_documents() {
        let document = parse(r#"{"a":[1,2,{"b":null}],"c":"x","d":{}}"#).unwrap();
        assert_eq!(document.get("c").and_then(Json::as_str), Some("x"));
        let list = document.get("a").and_then(Json::as_array).unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list[2].get("b"), Some(&Json::Null));
        assert_eq!(
            document.get("d").and_then(Json::as_object).map(<[_]>::len),
            Some(0)
        );
        assert!(document.get("missing").is_none());
    }

    #[test]
    fn decodes_escapes_and_surrogates() {
        let value = parse(r#""a\n\t\"\\\/\u0041\ud83d\ude00é""#).unwrap();
        assert_eq!(value.as_str(), Some("a\n\t\"\\/A\u{1F600}é"));
    }

    #[test]
    fn rejects_malformed_input() {
        for bad in [
            "",
            "{",
            "[1,]",
            "{\"a\":1,}",
            "01",
            "1.",
            ".5",
            "+1",
            "-",
            "1e",
            "nul",
            "\"abc",
            "\"\\x\"",
            "\"\\ud83d\"",
            "\"\\ude00\"",
            "[1 2]",
            "{\"a\" 1}",
            "{1:2}",
            "tru",
            "1 2",
            "\"\u{0001}\"",
            "1e999",
        ] {
            assert!(parse(bad).is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn rejects_duplicate_keys() {
        let error = parse(r#"{"a":1,"a":2}"#).unwrap_err();
        assert!(error.message.contains("duplicate"));
    }

    #[test]
    fn enforces_depth_limit() {
        let deep = "[".repeat(600) + &"]".repeat(600);
        assert!(parse(&deep).is_err());
        let fine = "[".repeat(100) + &"]".repeat(100);
        assert!(parse(&fine).is_ok());
    }

    #[test]
    fn error_reports_offset() {
        let error = parse("[1, ?]").unwrap_err();
        assert_eq!(error.offset, 4);
    }
}
