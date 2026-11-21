//! Positioned parsing of `fga.mod` manifests.

use alloc::{
    borrow::Cow,
    collections::BTreeMap,
    format,
    string::{String, ToString},
    vec::Vec,
};
use core::fmt;
use core::fmt::Write as _;
use saphyr_parser::{Event, Marker, Parser, ScalarStyle, Span, SpannedEventReceiver, Tag};

/// A manifest string with its zero-based source position.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModFileStringProperty {
    /// The decoded property value.
    pub value: String,
    /// The zero-based source line.
    pub line: usize,
    /// The zero-based source column.
    pub column: usize,
}

/// Manifest paths with their zero-based sequence position.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModFileArrayProperty {
    /// The normalized module paths.
    pub value: Vec<ModFileStringProperty>,
    /// The zero-based source line.
    pub line: usize,
    /// The zero-based source column.
    pub column: usize,
}

/// A validated `fga.mod` manifest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModFile {
    /// The supported schema version.
    pub schema: ModFileStringProperty,
    /// The module paths in declaration order.
    pub contents: ModFileArrayProperty,
}

/// A positioned manifest validation diagnostic.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("validation error at line={line}, column={column}: {message}")]
pub struct ModFileError {
    /// The validation message.
    pub message: String,
    /// The zero-based source line.
    pub line: usize,
    /// The zero-based source column.
    pub column: usize,
}

/// Manifest syntax, decoding, or validation failures.
#[derive(Debug, thiserror::Error)]
pub enum ModFileErrors {
    /// YAML syntax could not be parsed.
    #[error("YAML syntax error at line={}, column={}: {}", .0.line, .0.column, .0.message)]
    Syntax(ModFileError),
    /// YAML values could not be decoded into manifest fields.
    #[error("{message}")]
    Decode {
        /// The YAML decoding diagnostic.
        message: String,
        /// The zero-based source line.
        line: usize,
        /// The zero-based source column.
        column: usize,
    },
    /// Manifest fields failed validation in declaration-independent order.
    #[error("{}", ValidationDisplay(errors))]
    Validation {
        /// Schema diagnostics followed by contents diagnostics.
        errors: Vec<ModFileError>,
    },
}

struct ValidationDisplay<'errors>(&'errors [ModFileError]);

impl fmt::Display for ValidationDisplay<'_> {
    /// Formats the aggregate validation diagnostic.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            formatter,
            "{} error{} occurred:",
            self.0.len(),
            if self.0.len() == 1 { "" } else { "s" }
        )
        .and_then(|()| {
            self.0
                .iter()
                .try_for_each(|error| writeln!(formatter, "\t* {error}"))
        })
        .and_then(|()| writeln!(formatter))
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Kind {
    Scalar,
    Sequence,
    Mapping,
    Alias(usize),
}

struct Node<'input> {
    kind: Kind,
    tag: Cow<'input, str>,
    value: Cow<'input, str>,
    marker: Marker,
    children: Vec<usize>,
}

struct Document<'input> {
    source: &'input str,
    nodes: Vec<Node<'input>>,
    parents: Vec<usize>,
    anchors: BTreeMap<usize, usize>,
}

impl<'input> SpannedEventReceiver<'input> for Document<'input> {
    /// Records YAML nodes without expanding aliases.
    fn on_event(&mut self, event: Event<'input>, span: Span) {
        let (kind, tag, value, anchor) = match event {
            Event::Scalar(value, style, anchor, tag) => {
                let resolved = match tag.filter(|tag| !(tag.handle.is_empty() && tag.suffix == "!"))
                {
                    Some(tag) => explicit_tag(tag),
                    None if style != ScalarStyle::Plain => Cow::Borrowed("str"),
                    None => Cow::Borrowed(scalar_tag(&value)),
                };
                (Kind::Scalar, resolved, value, anchor)
            }
            Event::SequenceStart(anchor, tag) => (
                Kind::Sequence,
                tag.filter(|tag| !(tag.handle.is_empty() && tag.suffix == "!"))
                    .map_or(Cow::Borrowed("seq"), explicit_tag),
                Cow::Borrowed(""),
                anchor,
            ),
            Event::MappingStart(anchor, tag) => (
                Kind::Mapping,
                tag.filter(|tag| !(tag.handle.is_empty() && tag.suffix == "!"))
                    .map_or(Cow::Borrowed("map"), explicit_tag),
                Cow::Borrowed(""),
                anchor,
            ),
            Event::Alias(anchor) => {
                let name: String = self
                    .source
                    .chars()
                    .skip(span.start.index() + 1)
                    .take(span.end.index() - span.start.index() - 1)
                    .collect();
                (Kind::Alias(anchor), Cow::Borrowed(""), Cow::Owned(name), 0)
            }
            Event::SequenceEnd | Event::MappingEnd => {
                self.parents.pop();
                return;
            }
            Event::Nothing
            | Event::StreamStart
            | Event::StreamEnd
            | Event::DocumentStart(_)
            | Event::DocumentEnd => return,
        };
        let index = self.nodes.len();
        if let Some(&parent) = self.parents.last() {
            self.nodes[parent].children.push(index);
        }
        if anchor != 0 {
            self.anchors.insert(anchor, index);
        }
        self.nodes.push(Node {
            kind,
            tag,
            value,
            marker: span.start,
            children: Vec::new(),
        });
        if matches!(kind, Kind::Sequence | Kind::Mapping) {
            self.parents.push(index);
        }
    }
}

/// Resolves explicit core tags without accepting custom tag suffixes.
fn explicit_tag(tag: Cow<'_, Tag>) -> Cow<'_, str> {
    let tag = tag.into_owned();
    if tag.is_yaml_core_schema() {
        Cow::Owned(tag.suffix)
    } else {
        Cow::Owned(format!("{}{}", tag.handle, tag.suffix))
    }
}

/// Resolves plain scalars into their implicit YAML categories.
fn scalar_tag(value: &str) -> &'static str {
    match value {
        "" | "~" | "null" | "Null" | "NULL" => return "null",
        "true" | "True" | "TRUE" | "false" | "False" | "FALSE" => return "bool",
        ".nan" | ".NaN" | ".NAN" | ".inf" | ".Inf" | ".INF" | "+.inf" | "+.Inf" | "+.INF"
        | "-.inf" | "-.Inf" | "-.INF" => return "float",
        "<<" => return "merge",
        _ => {}
    }
    if is_timestamp(value) {
        return "timestamp";
    }
    let plain = if value.contains('_') {
        Cow::Owned(value.replace('_', ""))
    } else {
        Cow::Borrowed(value)
    };
    let unsigned = plain.strip_prefix(['+', '-']).unwrap_or(&plain);
    let negative = plain.starts_with('-');
    let (digits, radix) = if let Some(digits) = unsigned
        .strip_prefix("0x")
        .or_else(|| unsigned.strip_prefix("0X"))
    {
        (digits, 16)
    } else if let Some(digits) = unsigned
        .strip_prefix("0o")
        .or_else(|| unsigned.strip_prefix("0O"))
    {
        (digits, 8)
    } else if let Some(digits) = unsigned
        .strip_prefix("0b")
        .or_else(|| unsigned.strip_prefix("0B"))
    {
        (digits, 2)
    } else if unsigned.len() > 1 && unsigned.starts_with('0') {
        (unsigned, 8)
    } else {
        (unsigned, 10)
    };
    if digits.chars().all(|character| character.is_digit(radix))
        && u64::from_str_radix(digits, radix).is_ok_and(|number| !negative || number <= 1_u64 << 63)
    {
        return "int";
    }
    if plain.bytes().all(|character| {
        character.is_ascii_digit() || matches!(character, b'+' | b'-' | b'.' | b'e' | b'E')
    }) && plain.parse::<f64>().is_ok_and(f64::is_finite)
    {
        return "float";
    }
    "str"
}

/// Recognizes calendar dates and accepted timestamp formats.
fn is_timestamp(value: &str) -> bool {
    if !value.is_ascii() {
        return false;
    }
    let (date, time) = value.find(['T', 't', ' ']).map_or((value, None), |index| {
        (&value[..index], Some(&value[index..]))
    });
    let mut parts = date.split('-');
    let Some(year) = parts
        .next()
        .filter(|part| part.len() == 4)
        .and_then(|part| decimal_component(part, 4))
    else {
        return false;
    };
    let Some(month) = parts
        .next()
        .and_then(|part| decimal_component(part, 2))
        .filter(|month| (1..=12).contains(month))
    else {
        return false;
    };
    let Some(day) = parts.next().and_then(|part| decimal_component(part, 2)) else {
        return false;
    };
    let days = match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if parts.next().is_some() || !(1..=days).contains(&day) {
        return false;
    }
    let Some(time) = time else {
        return true;
    };
    let (clock, zone) = time[1..]
        .find(['Z', '+', '-'])
        .map_or((&time[1..], None), |index| {
            (&time[1..=index], Some(&time[index + 1..]))
        });
    if time.starts_with(' ') && zone.is_some() || !time.starts_with(' ') && zone.is_none() {
        return false;
    }
    let mut parts = clock.split(':');
    let hour = parts.next().and_then(|part| decimal_component(part, 2));
    let minute = parts.next().and_then(|part| decimal_component(part, 2));
    let second = parts.next().and_then(|part| {
        let (integer, fraction) = part
            .split_once(['.', ','])
            .map_or((part, None), |(integer, fraction)| {
                (integer, Some(fraction))
            });
        if fraction.is_some_and(|fraction| {
            fraction.is_empty() || !fraction.bytes().all(|character| character.is_ascii_digit())
        }) {
            return None;
        }
        decimal_component(integer, 2)
    });
    if hour.is_none_or(|hour| hour >= 24)
        || minute.is_none_or(|minute| minute >= 60)
        || second.is_none_or(|second| second >= 60)
        || parts.next().is_some()
    {
        return false;
    }
    match zone {
        None | Some("Z") => true,
        Some(zone) => {
            zone.len() == 6
                && zone.as_bytes()[3] == b':'
                && decimal_component(&zone[1..3], 2).is_some_and(|hour| hour <= 24)
                && decimal_component(&zone[4..], 2).is_some_and(|minute| minute <= 60)
        }
    }
}

/// Parses a bounded, unsigned decimal timestamp component.
fn decimal_component(value: &str, maximum_digits: usize) -> Option<u32> {
    if value.len() > maximum_digits || !value.bytes().all(|character| character.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

/// Parses and validates a module manifest with zero-based source positions.
///
/// # Errors
/// Returns YAML decoding errors or all schema and contents validation errors.
pub fn parse_mod_file(source: &str) -> Result<ModFile, ModFileErrors> {
    let mut document = Document {
        source,
        nodes: Vec::new(),
        parents: Vec::new(),
        anchors: BTreeMap::new(),
    };
    Parser::new_from_str(source)
        .load(&mut document, false)
        .map_err(|error| {
            ModFileErrors::Syntax(ModFileError {
                message: error.info().to_string(),
                line: error.marker().line() - 1,
                column: error.marker().col(),
            })
        })?;
    let mut fields = [None, None];
    if let Some(root) = document.nodes.first() {
        if root.kind == Kind::Mapping {
            document.fields(0, &mut fields, &mut Vec::new())?;
        } else if root.tag != "null" {
            let value = if root.value.is_empty() {
                String::new()
            } else {
                format!(" `{}`", root.value)
            };
            return Err(ModFileErrors::Decode {
                message: format!(
                    "yaml: unmarshal errors:\n  line {}: cannot unmarshal !!{}{value} into transformer.YAMLModFile",
                    root.marker.line(),
                    root.tag
                ),
                line: root.marker.line() - 1,
                column: root.marker.col(),
            });
        }
    }
    let mut errors = Vec::new();
    let schema = match fields[0].map(|index| &document.nodes[index]) {
        None => {
            errors.push(missing("schema"));
            None
        }
        Some(node) if node.tag != "str" => {
            errors.push(node.error(format!(
                "unexpected schema type, expected string got value {}",
                node.value
            )));
            None
        }
        Some(node) if node.value != "1.2" => {
            errors.push(node.error(
                "unsupported schema version, fga.mod only supported in version `1.2`".into(),
            ));
            None
        }
        Some(node) => Some(node.property(node.value.to_string())),
    };
    let contents = match fields[1].map(|index| &document.nodes[index]) {
        None => {
            errors.push(missing("contents"));
            None
        }
        Some(node) if node.tag != "seq" => {
            errors.push(node.error(format!(
                "unexpected contents type, expected list of strings got value {}",
                node.value
            )));
            None
        }
        Some(node) => {
            let mut value = Vec::with_capacity(node.children.len());
            for &index in &node.children {
                let item = &document.nodes[index];
                match item.path() {
                    Ok(path) => value.push(item.property(path)),
                    Err(error) => errors.push(error),
                }
            }
            Some(ModFileArrayProperty {
                value,
                line: node.marker.line() - 1,
                column: node.marker.col(),
            })
        }
    };
    match (schema, contents) {
        (Some(schema), Some(contents)) if errors.is_empty() => Ok(ModFile { schema, contents }),
        _ => Err(ModFileErrors::Validation { errors }),
    }
}

impl Document<'_> {
    /// Collects explicit fields before applying YAML merge mappings.
    fn fields(
        &self,
        index: usize,
        fields: &mut [Option<usize>; 2],
        visiting: &mut Vec<usize>,
    ) -> Result<(), ModFileErrors> {
        let node = &self.nodes[index];
        visiting.push(index);
        self.validate_keys(node)?;
        for pair in node.children.as_chunks::<2>().0 {
            let key = &self.nodes[pair[0]];
            let field = match key.value.as_ref() {
                "schema" => Some(0),
                "contents" => Some(1),
                _ => None,
            };
            if let Some(field) = field
                && fields[field].is_none()
            {
                fields[field] = Some(pair[1]);
            }
        }
        for pair in node.children.as_chunks::<2>().0 {
            if self.nodes[pair[0]].tag == "merge" {
                self.merge(pair[1], fields, visiting)?;
            }
        }
        visiting.pop();
        Ok(())
    }

    /// Collects duplicate mapping keys before decoding their values.
    fn validate_keys(&self, node: &Node<'_>) -> Result<(), ModFileErrors> {
        let mut duplicate: Option<(String, Marker)> = None;
        let pairs = node.children.as_chunks::<2>().0;
        for (position, pair) in pairs.iter().enumerate() {
            let previous = &self.nodes[pair[0]];
            for pair in &pairs[position + 1..] {
                let key = &self.nodes[pair[0]];
                if previous.kind == key.kind && previous.value == key.value {
                    let (message, _) = duplicate
                        .get_or_insert_with(|| ("yaml: unmarshal errors:".into(), key.marker));
                    let _ = write!(
                        message,
                        "\n  line {}: mapping key {:?} already defined at line {}",
                        key.marker.line(),
                        key.value,
                        previous.marker.line()
                    );
                }
            }
        }
        match duplicate {
            Some((message, marker)) => Err(ModFileErrors::Decode {
                message,
                line: marker.line() - 1,
                column: marker.col(),
            }),
            None => Ok(()),
        }
    }

    /// Applies mappings and alias sequences used by YAML merge keys.
    fn merge(
        &self,
        index: usize,
        fields: &mut [Option<usize>; 2],
        visiting: &mut Vec<usize>,
    ) -> Result<(), ModFileErrors> {
        let node = &self.nodes[index];
        if node.kind == Kind::Sequence {
            for &child in &node.children {
                self.merge_mapping(child, fields, visiting)?;
            }
            Ok(())
        } else {
            self.merge_mapping(index, fields, visiting)
        }
    }

    /// Resolves a merge alias and rejects non-mapping values.
    fn merge_mapping(
        &self,
        index: usize,
        fields: &mut [Option<usize>; 2],
        visiting: &mut Vec<usize>,
    ) -> Result<(), ModFileErrors> {
        let node = &self.nodes[index];
        let target = match node.kind {
            Kind::Alias(anchor) => self.anchors[&anchor],
            _ => index,
        };
        if visiting.contains(&target) {
            return Err(ModFileErrors::Decode {
                message: format!("yaml: anchor '{}' value contains itself", node.value),
                line: node.marker.line() - 1,
                column: node.marker.col(),
            });
        }
        if self.nodes[target].kind == Kind::Mapping {
            self.fields(target, fields, visiting)
        } else {
            Err(ModFileErrors::Decode {
                message: "yaml: map merge requires map or sequence of maps as the value".into(),
                line: node.marker.line() - 1,
                column: node.marker.col(),
            })
        }
    }
}

impl Node<'_> {
    /// Creates a diagnostic at this node's source position.
    fn error(&self, message: String) -> ModFileError {
        ModFileError {
            message,
            line: self.marker.line() - 1,
            column: self.marker.col(),
        }
    }

    /// Attaches this node's source position to a decoded value.
    fn property(&self, value: String) -> ModFileStringProperty {
        ModFileStringProperty {
            value,
            line: self.marker.line() - 1,
            column: self.marker.col(),
        }
    }

    /// Validates and normalizes a module path.
    #[expect(
        clippy::case_sensitive_file_extension_comparisons,
        reason = "Manifest paths require the lowercase .fga extension."
    )]
    fn path(&self) -> Result<String, ModFileError> {
        if self.tag != "str" {
            return Err(self.error(format!(
                "unexpected contents item type, expected string got value {}",
                self.value
            )));
        }
        let mut decoded = Vec::with_capacity(self.value.len());
        let mut bytes = self.value.bytes();
        while let Some(byte) = bytes.next() {
            let byte = match byte {
                b'+' => b' ',
                b'%' => {
                    let pair = bytes
                        .next()
                        .and_then(hexadecimal)
                        .zip(bytes.next().and_then(hexadecimal));
                    let Some((high, low)) = pair else {
                        return Err(self.error(format!("failed to decode path: {}", self.value)));
                    };
                    high * 16 + low
                }
                byte => byte,
            };
            decoded.push(if byte == b'\\' { b'/' } else { byte });
        }
        let normalized = match String::from_utf8(decoded) {
            Ok(normalized) => normalized,
            Err(error) => replace_invalid_utf8(error.as_bytes()),
        };
        if normalized.contains("../") || normalized.starts_with('/') {
            return Err(self.error(format!("invalid contents item {}", self.value)));
        }
        if !normalized.ends_with(".fga") {
            return Err(self.error(format!(
                "contents items should use fga file extension, got {}",
                self.value
            )));
        }
        Ok(normalized)
    }
}

/// Constructs a diagnostic for an absent manifest field.
fn missing(field: &str) -> ModFileError {
    ModFileError {
        message: format!("missing {field} field"),
        line: 0,
        column: 0,
    }
}

/// Decodes one hexadecimal URI digit.
fn hexadecimal(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Replaces each invalid UTF-8 byte with a replacement character.
fn replace_invalid_utf8(mut bytes: &[u8]) -> String {
    let mut result = String::with_capacity(bytes.len());
    while !bytes.is_empty() {
        match core::str::from_utf8(bytes) {
            Ok(valid) => {
                result.push_str(valid);
                break;
            }
            Err(error) => {
                let (valid, remaining) = bytes.split_at(error.valid_up_to());
                result.push_str(&String::from_utf8_lossy(valid));
                let invalid_length = error.error_len().unwrap_or(remaining.len());
                for _ in 0..invalid_length {
                    result.push('\u{fffd}');
                }
                bytes = &remaining[invalid_length..];
            }
        }
    }
    result
}
