//! Minimal WGSL syntax highlighting: turns source code into HTML where each
//! token is wrapped in a `<span>` with a class (see the CSS in `index.html`).
//!
//! It's a keyword-based tokenizer, not a parser: good enough to color code
//! as you type, and fast enough to run on every keystroke.

const KEYWORDS: &[&str] = &[
    "alias",
    "break",
    "case",
    "const",
    "const_assert",
    "continue",
    "continuing",
    "default",
    "diagnostic",
    "discard",
    "else",
    "enable",
    "false",
    "fn",
    "for",
    "if",
    "let",
    "loop",
    "override",
    "requires",
    "return",
    "struct",
    "switch",
    "true",
    "var",
    "while",
];

const TYPES: &[&str] = &[
    "array", "atomic", "bool", "f16", "f32", "i32", "ptr", "sampler", "u32", "vec2", "vec3",
    "vec4", "vec2f", "vec3f", "vec4f", "vec2i", "vec3i", "vec4i", "vec2u", "vec3u", "vec4u",
    "mat2x2", "mat2x3", "mat2x4", "mat3x2", "mat3x3", "mat3x4", "mat4x2", "mat4x3", "mat4x4",
    "mat2x2f", "mat3x3f", "mat4x4f",
];

const BUILTINS: &[&str] = &[
    "abs",
    "acos",
    "all",
    "any",
    "asin",
    "atan",
    "atan2",
    "ceil",
    "clamp",
    "cos",
    "cosh",
    "cross",
    "degrees",
    "determinant",
    "distance",
    "dot",
    "dpdx",
    "dpdy",
    "exp",
    "exp2",
    "floor",
    "fma",
    "fract",
    "fwidth",
    "inverseSqrt",
    "length",
    "log",
    "log2",
    "max",
    "min",
    "mix",
    "modf",
    "normalize",
    "pow",
    "radians",
    "reflect",
    "refract",
    "round",
    "saturate",
    "select",
    "sign",
    "sin",
    "sinh",
    "smoothstep",
    "sqrt",
    "step",
    "tan",
    "tanh",
    "transpose",
    "trunc",
];

/// What the visuals get from the app (see `common.wgsl`).
const INPUTS: &[&str] = &["band", "band_at", "hue", "BAND_COUNT", "PI"];

pub fn highlight(source: &str) -> String {
    let mut html = String::with_capacity(source.len() * 2);
    let mut rest = source;
    while let Some(c) = rest.chars().next() {
        let (len, class) = if rest.starts_with("//") {
            (rest.find('\n').unwrap_or(rest.len()), Some("c"))
        } else if let Some(comment) = rest.strip_prefix("/*") {
            let end = comment.find("*/").map_or(rest.len(), |i| i + 4);
            (end, Some("c"))
        } else if c.is_ascii_digit()
            || (c == '.' && rest[1..].starts_with(|c: char| c.is_ascii_digit()))
        {
            (number_len(rest), Some("n"))
        } else if c == '@' {
            (1 + identifier_len(&rest[1..]), Some("a"))
        } else if c.is_alphabetic() || c == '_' {
            let len = identifier_len(rest);
            let word = &rest[..len];
            // Uniform fields: color `u.time` as a whole.
            if word == "u" && rest[len..].starts_with('.') {
                (len + 1 + identifier_len(&rest[len + 1..]), Some("i"))
            } else {
                (len, classify(word))
            }
        } else {
            (c.len_utf8(), None)
        };
        push(&mut html, &rest[..len], class);
        rest = &rest[len..];
    }
    html
}

fn classify(word: &str) -> Option<&'static str> {
    if KEYWORDS.contains(&word) {
        Some("k")
    } else if TYPES.contains(&word) {
        Some("t")
    } else if BUILTINS.contains(&word) {
        Some("f")
    } else if INPUTS.contains(&word) {
        Some("i")
    } else {
        None
    }
}

fn identifier_len(s: &str) -> usize {
    s.find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(s.len())
}

/// Length of the number at the start of `s`: `42`, `0x1F`, `1.5e-3`, `2u`…
fn number_len(s: &str) -> usize {
    let mut previous = ' ';
    s.char_indices()
        .find(|&(i, c)| {
            let exponent_sign = (c == '-' || c == '+')
                && (previous == 'e' || previous == 'E')
                && !s[..i].starts_with("0x");
            previous = c;
            !(c.is_ascii_alphanumeric() || c == '.' || exponent_sign)
        })
        .map_or(s.len(), |(i, _)| i)
}

fn push(html: &mut String, text: &str, class: Option<&str>) {
    if let Some(class) = class {
        html.push_str("<span class=\"");
        html.push_str(class);
        html.push_str("\">");
    }
    for c in text.chars() {
        match c {
            '&' => html.push_str("&amp;"),
            '<' => html.push_str("&lt;"),
            '>' => html.push_str("&gt;"),
            _ => html.push(c),
        }
    }
    if class.is_some() {
        html.push_str("</span>");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_tokens() {
        assert_eq!(
            highlight("let x = sin(1.5);"),
            r#"<span class="k">let</span> x = <span class="f">sin</span>(<span class="n">1.5</span>);"#
        );
    }

    #[test]
    fn escapes_html() {
        assert_eq!(
            highlight("vec4<f32> a && b"),
            r#"<span class="t">vec4</span>&lt;<span class="t">f32</span>&gt; a &amp;&amp; b"#
        );
    }

    #[test]
    fn comments_run_to_the_end_of_the_line() {
        assert_eq!(
            highlight("// fn <b>\nfn"),
            "<span class=\"c\">// fn &lt;b&gt;</span>\n<span class=\"k\">fn</span>"
        );
        assert_eq!(
            highlight("/* a\nb */x"),
            "<span class=\"c\">/* a\nb */</span>x"
        );
        // Unterminated block comment: everything left is a comment.
        assert_eq!(highlight("/* a"), "<span class=\"c\">/* a</span>");
    }

    #[test]
    fn numbers() {
        for number in ["42", "0x1F", "1.5e-3", "2u", "0.5f", ".25"] {
            assert_eq!(
                highlight(number),
                format!("<span class=\"n\">{number}</span>"),
                "{number}"
            );
        }
        // The minus of a subtraction isn't part of the number.
        assert_eq!(
            highlight("1-2"),
            "<span class=\"n\">1</span>-<span class=\"n\">2</span>"
        );
    }

    #[test]
    fn attributes_and_inputs() {
        assert_eq!(
            highlight("@fragment u.time band(i)"),
            r#"<span class="a">@fragment</span> <span class="i">u.time</span> <span class="i">band</span>(i)"#
        );
    }

    #[test]
    fn keeps_the_text_intact() {
        let source = include_str!("../../../render/src/shaders/radial.wgsl");
        let html = highlight(source);
        let text = html
            .replace("</span>", "")
            .split("<span class=\"")
            .map(|part| part.split_once("\">").map_or(part, |(_, text)| text))
            .collect::<String>()
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&");
        assert_eq!(text, source);
    }
}
