//! Completion and hover docs for the editor. Pure functions over the source
//! text; the popups that show the results are in `web.rs`.
//!
//! Offsets are byte offsets into the source. Positions are (line, column)
//! as displayed: lines from 0, and columns counting a tab as up to 4 spaces.

use super::wgsl::{self, BUILTINS, Doc, FIELDS, INPUTS, KEYWORDS, TYPES};

/// Most suggestions shown at once.
const MAX_SUGGESTIONS: usize = 12;
/// Must match `tab-size` in the editor's CSS.
const TAB_SIZE: usize = 4;

/// Keywords that declare a name, followed by that name.
const DECLARATIONS: &[&str] = &["let", "var", "const", "fn", "struct", "alias", "override"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// Declared in the shader's own code.
    Local,
    /// A field of the uniforms, `u.<field>`.
    Field,
    /// Provided by the app, like `band`.
    Input,
    Function,
    Type,
    Keyword,
}

impl Kind {
    /// CSS class, matching the highlighting colors.
    pub fn class(self) -> &'static str {
        match self {
            Kind::Local => "l",
            Kind::Field | Kind::Input => "i",
            Kind::Function => "f",
            Kind::Type => "t",
            Kind::Keyword => "k",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub label: String,
    pub kind: Kind,
    /// Signature or origin, shown next to the label.
    pub detail: String,
    pub doc: String,
}

pub struct Completion {
    /// Where the word being completed starts; accepting a suggestion replaces
    /// the text from here to the cursor.
    pub start: usize,
    pub suggestions: Vec<Suggestion>,
}

/// What to show when hovering a word.
#[derive(Debug, PartialEq)]
pub struct Hover {
    /// Where the hovered word starts, to place the tooltip.
    pub start: usize,
    pub title: String,
    pub doc: String,
}

/// Suggestions for the word ending at `cursor`, if any are worth showing.
pub fn complete(source: &str, cursor: usize) -> Option<Completion> {
    let before = &source[..cursor];
    let line = &before[before.rfind('\n').map_or(0, |i| i + 1)..];
    if line.contains("//") {
        return None;
    }
    let start = word_start(before);
    let prefix = &before[start..];
    if prefix.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }

    let candidates = if after_uniforms(&before[..start]) {
        FIELDS
            .iter()
            .map(|doc| documented(doc, Kind::Field))
            .collect()
    } else if before[..start].ends_with('.') || prefix.is_empty() {
        // A field of something we know nothing about, or no word yet.
        return None;
    } else {
        let locals = declarations(source)
            .into_iter()
            .filter(|declaration| declaration.name != prefix)
            .map(|declaration| declaration.suggestion());
        let inputs = INPUTS.iter().map(|doc| documented(doc, Kind::Input));
        let functions = BUILTINS.iter().map(|doc| documented(doc, Kind::Function));
        let types = TYPES.iter().map(|&name| Suggestion {
            label: name.to_owned(),
            kind: Kind::Type,
            detail: "type".to_owned(),
            doc: wgsl::type_doc(name).unwrap_or_default(),
        });
        let keywords = KEYWORDS.iter().map(|&name| Suggestion {
            label: name.to_owned(),
            kind: Kind::Keyword,
            detail: "keyword".to_owned(),
            doc: String::new(),
        });
        let mut all: Vec<_> = locals
            .chain(inputs)
            .chain(functions)
            .chain(types)
            .chain(keywords)
            .collect();
        all.dedup_by(|a, b| a.label == b.label);
        all
    };

    let lower = prefix.to_lowercase();
    // How well a candidate matches: exact prefix, then prefix ignoring case,
    // then anywhere in the name.
    let rank = |label: &str| {
        if label.starts_with(prefix) {
            Some(0)
        } else if label.to_lowercase().starts_with(&lower) {
            Some(1)
        } else if label.to_lowercase().contains(&lower) {
            Some(2)
        } else {
            None
        }
    };
    let mut ranked: Vec<_> = candidates
        .into_iter()
        .filter_map(|suggestion| Some((rank(&suggestion.label)?, suggestion)))
        .collect();
    // Stable: within a rank and kind, the lists' own order (declaration order
    // for local names, alphabetical for built-ins).
    ranked.sort_by_key(|(rank, suggestion)| (*rank, suggestion.kind));
    let suggestions: Vec<_> = ranked
        .into_iter()
        .map(|(_, suggestion)| suggestion)
        .take(MAX_SUGGESTIONS)
        .collect();

    // Nothing to add when the word is already complete.
    match suggestions.as_slice() {
        [] => None,
        [only] if only.label == prefix => None,
        _ => Some(Completion { start, suggestions }),
    }
}

/// Docs for the word at `offset`, if it has any.
pub fn hover(source: &str, offset: usize) -> Option<Hover> {
    let is_word = |c: char| is_identifier(c);
    if !source[offset..].starts_with(is_word) {
        return None;
    }
    let start = word_start(&source[..offset]);
    let end = offset
        + source[offset..]
            .find(|c: char| !is_word(c))
            .unwrap_or(source.len() - offset);
    let word = &source[start..end];
    if word.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let hover = |title: String, doc: String| Some(Hover { start, title, doc });
    let find = |docs: &[Doc]| {
        docs.iter()
            .find(|doc| doc.name == word)
            .map(|doc| (doc.signature, doc.doc))
    };

    if after_uniforms(&source[..start]) {
        let (signature, doc) = find(FIELDS)?;
        return hover(format!("u.{word}{signature}"), doc.to_owned());
    }
    if let Some((signature, doc)) = find(INPUTS).or_else(|| find(BUILTINS)) {
        return hover(format!("{word}{signature}"), doc.to_owned());
    }
    if let Some(doc) = wgsl::type_doc(word) {
        return hover(word.to_owned(), doc);
    }
    let declaration = declarations(source)
        .into_iter()
        .find(|declaration| declaration.name == word)?;
    hover(
        format!("{} {}", declaration.keyword, declaration.name),
        format!("Line {}: {}", declaration.line + 1, declaration.text),
    )
}

/// The displayed (line, column) of a byte offset.
pub fn position(source: &str, offset: usize) -> (usize, usize) {
    let before = &source[..offset];
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let line = before.matches('\n').count();
    let column = before[line_start..].chars().fold(0, advance);
    (line, column)
}

/// The byte offset of the character displayed at (line, column), if there's
/// one there.
pub fn offset_at(source: &str, line: usize, column: usize) -> Option<usize> {
    let line_start = if line == 0 {
        0
    } else {
        source.match_indices('\n').nth(line - 1)?.0 + 1
    };
    let mut current = 0;
    for (i, c) in source[line_start..].char_indices() {
        if c == '\n' {
            return None;
        }
        let next = advance(current, c);
        if column < next {
            return Some(line_start + i);
        }
        current = next;
    }
    None
}

/// Converts an offset counted in UTF-16 units (as the browser does) to bytes.
pub fn utf16_to_byte(source: &str, utf16: usize) -> usize {
    let mut units = 0;
    for (i, c) in source.char_indices() {
        if units >= utf16 {
            return i;
        }
        units += c.len_utf16();
    }
    source.len()
}

/// Converts a byte offset to UTF-16 units (as the browser counts them).
pub fn byte_to_utf16(source: &str, byte: usize) -> usize {
    source[..byte].chars().map(char::len_utf16).sum()
}

pub fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn advance(column: usize, c: char) -> usize {
    if c == '\t' {
        (column / TAB_SIZE + 1) * TAB_SIZE
    } else {
        column + 1
    }
}

fn is_identifier(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Start of the identifier that `text` ends with (`text.len()` if none).
fn word_start(text: &str) -> usize {
    text.char_indices()
        .rev()
        .take_while(|&(_, c)| is_identifier(c))
        .last()
        .map_or(text.len(), |(i, _)| i)
}

/// Whether `text` ends with `u.`, the uniforms (and not e.g. `menu.`).
fn after_uniforms(text: &str) -> bool {
    text.strip_suffix("u.")
        .is_some_and(|rest| !rest.ends_with(is_identifier))
}

fn documented(doc: &Doc, kind: Kind) -> Suggestion {
    Suggestion {
        label: doc.name.to_owned(),
        kind,
        detail: doc.signature.to_owned(),
        doc: doc.doc.to_owned(),
    }
}

struct Declaration<'a> {
    keyword: &'a str,
    name: &'a str,
    line: usize,
    /// The whole line, trimmed.
    text: &'a str,
}

impl Declaration<'_> {
    fn suggestion(&self) -> Suggestion {
        Suggestion {
            label: self.name.to_owned(),
            kind: Kind::Local,
            detail: format!("{} · line {}", self.keyword, self.line + 1),
            doc: self.text.to_owned(),
        }
    }
}

/// Names declared in the code (`let x`, `fn f`…), first declaration first.
fn declarations(source: &str) -> Vec<Declaration<'_>> {
    let mut found: Vec<Declaration> = Vec::new();
    for (line, text) in source.lines().enumerate() {
        let code = text.split("//").next().unwrap_or("");
        let mut words = code
            .split(|c: char| !is_identifier(c))
            .filter(|w| !w.is_empty());
        while let Some(word) = words.next() {
            if !DECLARATIONS.contains(&word) {
                continue;
            }
            if let Some(name) = words.next() {
                let is_new = !found.iter().any(|d| d.name == name);
                if is_new && !name.starts_with(|c: char| c.is_ascii_digit()) {
                    found.push(Declaration {
                        keyword: word,
                        name,
                        line,
                        text: text.trim(),
                    });
                }
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Completes at the `|` in `source`.
    fn labels(source: &str) -> Vec<String> {
        let cursor = source.find('|').unwrap();
        let source = source.replace('|', "");
        complete(&source, cursor)
            .map(|c| c.suggestions.into_iter().map(|s| s.label).collect())
            .unwrap_or_default()
    }

    #[test]
    fn completes_builtins_by_prefix() {
        let found = labels("let a = smo|");
        assert_eq!(found, ["smoothstep"]);
        let found = labels("let a = s|");
        assert!(
            found.contains(&"sin".to_owned()) && found.contains(&"sqrt".to_owned()),
            "{found:?}"
        );
    }

    #[test]
    fn replaces_from_the_start_of_the_word() {
        let completion = complete("x = mi", 6).unwrap();
        assert_eq!(completion.start, 4);
        assert_eq!(completion.suggestions[0].label, "min");
    }

    #[test]
    fn completes_uniform_fields_after_u() {
        // In the order they're documented.
        let names: Vec<_> = FIELDS.iter().map(|doc| doc.name).collect();
        assert_eq!(labels("u.|"), names);
        assert_eq!(labels("u.tr|"), ["treble"]);
        // `menu.` isn't the uniforms.
        assert!(labels("menu.tr|").is_empty());
        // Fields of other values are unknown.
        assert!(labels("p.x|").is_empty());
    }

    #[test]
    fn suggests_names_from_the_code_first() {
        let found = labels("let brightness = 1.0;\nlet c = br|");
        assert_eq!(found[0], "brightness");
        let completion = complete("fn shape() {}\nsh", 16).unwrap();
        assert_eq!(completion.suggestions[0].detail, "fn · line 1");
    }

    #[test]
    fn ranks_exact_case_prefix_before_substring() {
        // `band` and `band_at` start with it; `BAND_COUNT` only ignoring case.
        assert_eq!(labels("ban|"), ["band", "band_at", "BAND_COUNT"]);
    }

    #[test]
    fn nothing_in_comments_numbers_or_complete_words() {
        assert!(labels("// smo|").is_empty());
        assert!(labels("let a = 1|").is_empty());
        assert!(labels("let a = |").is_empty());
        assert!(labels("smoothstep|").is_empty());
    }

    #[test]
    fn hover_docs() {
        let source = "let edge = smoothstep(0.1, 0.2, u.bass) * band(3u);";
        let at = |word: &str| hover(source, source.find(word).unwrap() + 1).map(|h| h.title);
        assert_eq!(
            at("smoothstep").unwrap(),
            "smoothstep(low: T, high: T, x: T) -> T"
        );
        assert_eq!(at("bass").unwrap(), "u.bass: f32");
        assert_eq!(at("band").unwrap(), "band(i: u32) -> f32");
        assert_eq!(at("edge").unwrap(), "let edge");
        assert_eq!(at("0.1"), None);
        assert_eq!(hover(source, source.find('(').unwrap()), None);
    }

    #[test]
    fn hover_on_types_and_declarations() {
        let source = "fn glow(x: f32) -> vec4<f32> {}\nlet c = glow(1.0);";
        let glow = hover(source, source.rfind("glow").unwrap()).unwrap();
        assert_eq!(glow.title, "fn glow");
        assert_eq!(glow.doc, "Line 1: fn glow(x: f32) -> vec4<f32> {}");
        assert_eq!(glow.start, source.rfind("glow").unwrap());
        let vec4 = hover(source, source.find("vec4").unwrap()).unwrap();
        assert!(
            vec4.doc
                .starts_with("Vector of 4 components, e.g. vec4<f32>"),
            "{}",
            vec4.doc
        );
        assert_eq!(
            wgsl::type_doc("vec2f").unwrap(),
            "Vector of 2 f32. Components: .x .y (or .r .g .b .a)."
        );
        assert_eq!(
            wgsl::type_doc("mat4x4f").unwrap(),
            "Matrix of 4 columns and 4 rows, of f32."
        );
    }

    #[test]
    fn positions_count_tabs_as_display_columns() {
        let source = "ab\n\tcd";
        assert_eq!(position(source, 1), (0, 1));
        assert_eq!(position(source, 4), (1, 4));
        assert_eq!(offset_at(source, 1, 4), Some(4));
        // Anywhere over the tab is the tab.
        assert_eq!(offset_at(source, 1, 2), Some(3));
        // Past the end of the line, or of the text.
        assert_eq!(offset_at(source, 0, 5), None);
        assert_eq!(offset_at(source, 7, 0), None);
    }

    #[test]
    fn converts_between_utf16_and_bytes() {
        // é is 2 bytes and 1 UTF-16 unit; 🎵 is 4 bytes and 2 units.
        let source = "é🎵x";
        assert_eq!(utf16_to_byte(source, 1), 2);
        assert_eq!(utf16_to_byte(source, 3), 6);
        assert_eq!(byte_to_utf16(source, 6), 3);
        assert_eq!(utf16_to_byte(source, 99), source.len());
    }
}
