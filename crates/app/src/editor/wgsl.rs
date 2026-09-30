//! What the editor knows about WGSL and about the app's inputs, shared by
//! highlighting, completion and hover docs.

/// A documented name: a built-in function, an input or a uniform field.
pub struct Doc {
    pub name: &'static str,
    /// Shown after the name, e.g. `(x: T) -> T`, or `: f32` for values.
    pub signature: &'static str,
    pub doc: &'static str,
}

const fn doc(name: &'static str, signature: &'static str, doc: &'static str) -> Doc {
    Doc {
        name,
        signature,
        doc,
    }
}

// Tables read best one entry per line.
#[rustfmt::skip]
pub const KEYWORDS: &[&str] = &[
    "alias", "break", "case", "const", "const_assert", "continue", "continuing", "default",
    "diagnostic", "discard", "else", "enable", "false", "fn", "for", "if", "let", "loop",
    "override", "requires", "return", "struct", "switch", "true", "var", "while",
];

#[rustfmt::skip]
pub const TYPES: &[&str] = &[
    "array", "atomic", "bool", "f16", "f32", "i32", "ptr", "sampler", "u32", "vec2", "vec3",
    "vec4", "vec2f", "vec3f", "vec4f", "vec2i", "vec3i", "vec4i", "vec2u", "vec3u", "vec4u",
    "mat2x2", "mat2x3", "mat2x4", "mat3x2", "mat3x3", "mat3x4", "mat4x2", "mat4x3", "mat4x4",
    "mat2x2f", "mat3x3f", "mat4x4f",
];

/// Built-in functions. `T` is `f32` or a float vector, applied per component.
#[rustfmt::skip]
pub const BUILTINS: &[Doc] = &[
    doc("abs", "(x: T) -> T", "Absolute value."),
    doc("acos", "(x: T) -> T", "Arc cosine, in radians."),
    doc("all", "(v: vecN<bool>) -> bool", "True if every component is true."),
    doc("any", "(v: vecN<bool>) -> bool", "True if at least one component is true."),
    doc("asin", "(x: T) -> T", "Arc sine, in radians."),
    doc("atan", "(x: T) -> T", "Arc tangent, in radians, from -PI/2 to PI/2."),
    doc("atan2", "(y: T, x: T) -> T", "Angle of the point (x, y), in radians, from -PI to PI."),
    doc("ceil", "(x: T) -> T", "Rounds up to the nearest integer."),
    doc("clamp", "(x: T, low: T, high: T) -> T", "Limits x to the range low..high."),
    doc("cos", "(x: T) -> T", "Cosine of an angle in radians."),
    doc("cosh", "(x: T) -> T", "Hyperbolic cosine."),
    doc("cross", "(a: vec3<f32>, b: vec3<f32>) -> vec3<f32>", "Cross product: a vector perpendicular to both."),
    doc("degrees", "(radians: T) -> T", "Converts radians to degrees."),
    doc("determinant", "(m: matNxN<f32>) -> f32", "Determinant of a square matrix."),
    doc("distance", "(a: T, b: T) -> f32", "Distance between two points."),
    doc("dot", "(a: vecN<f32>, b: vecN<f32>) -> f32", "Dot product: sum of the products of the components."),
    doc("dpdx", "(x: T) -> T", "How much x changes from this pixel to the next horizontally."),
    doc("dpdy", "(x: T) -> T", "How much x changes from this pixel to the next vertically."),
    doc("exp", "(x: T) -> T", "e raised to the power x."),
    doc("exp2", "(x: T) -> T", "2 raised to the power x."),
    doc("floor", "(x: T) -> T", "Rounds down to the nearest integer."),
    doc("fma", "(a: T, b: T, c: T) -> T", "a * b + c, in one step."),
    doc("fract", "(x: T) -> T", "Fractional part: x - floor(x), in 0..1. Handy for repeating patterns."),
    doc("fwidth", "(x: T) -> T", "abs(dpdx(x)) + abs(dpdy(x)): how much x changes per pixel. Useful for antialiasing."),
    doc("inverseSqrt", "(x: T) -> T", "1 / sqrt(x)."),
    doc("length", "(v: T) -> f32", "Length of a vector (distance from the origin)."),
    doc("log", "(x: T) -> T", "Natural logarithm."),
    doc("log2", "(x: T) -> T", "Base 2 logarithm."),
    doc("max", "(a: T, b: T) -> T", "The larger of a and b."),
    doc("min", "(a: T, b: T) -> T", "The smaller of a and b."),
    doc("mix", "(a: T, b: T, t: T) -> T", "Blends from a (t = 0) to b (t = 1): a + (b - a) * t."),
    doc("modf", "(x: T) -> struct", "Splits x into its fractional (.fract) and whole (.whole) parts."),
    doc("normalize", "(v: vecN<f32>) -> vecN<f32>", "Same direction, length 1."),
    doc("pow", "(x: T, y: T) -> T", "x raised to the power y."),
    doc("radians", "(degrees: T) -> T", "Converts degrees to radians."),
    doc("reflect", "(incident: T, normal: T) -> T", "Reflection of a direction off a surface."),
    doc("refract", "(incident: T, normal: T, eta: f32) -> T", "Refraction of a direction through a surface."),
    doc("round", "(x: T) -> T", "Rounds to the nearest integer (halves to even)."),
    doc("saturate", "(x: T) -> T", "Limits x to 0..1."),
    doc("select", "(if_false: T, if_true: T, condition: bool) -> T", "Picks if_true when the condition holds. Note the order!"),
    doc("sign", "(x: T) -> T", "-1, 0 or 1, depending on the sign of x."),
    doc("sin", "(x: T) -> T", "Sine of an angle in radians."),
    doc("sinh", "(x: T) -> T", "Hyperbolic sine."),
    doc("smoothstep", "(low: T, high: T, x: T) -> T", "0 below low, 1 above high, a smooth curve in between. Swap low and high to invert."),
    doc("sqrt", "(x: T) -> T", "Square root."),
    doc("step", "(edge: T, x: T) -> T", "0 if x < edge, 1 otherwise."),
    doc("tan", "(x: T) -> T", "Tangent of an angle in radians."),
    doc("tanh", "(x: T) -> T", "Hyperbolic tangent: a smooth curve from -1 to 1."),
    doc("transpose", "(m: matCxR<f32>) -> matRxC<f32>", "Swaps the rows and columns of a matrix."),
    doc("trunc", "(x: T) -> T", "Drops the fractional part (rounds towards 0)."),
];

/// What the visuals get from the app (see `common.wgsl`).
#[rustfmt::skip]
pub const INPUTS: &[Doc] = &[
    doc("band", "(i: u32) -> f32", "Loudness of band i, 0..1. Bands go from 0 (lows) to BAND_COUNT - 1 (highs)."),
    doc("band_at", "(t: f32) -> f32", "Loudness at t in 0..1 (lows to highs), interpolated between bands."),
    doc("hue", "(h: f32) -> vec3<f32>", "RGB color from a hue in 0..1 (red, green, blue, back to red)."),
    doc("note", "(n: u32) -> f32", "MIDI note n (60 = middle C, 69 = A 440 Hz): its velocity, 0..1, while held, then fading out after release."),
    doc("cc", "(n: u32) -> f32", "MIDI controller n (a knob or fader): its position, 0..1. Knob numbers depend on the device."),
    doc("BAND_COUNT", ": u32", "Number of frequency bands: 64."),
    doc("PI", ": f32", "3.14159..."),
];

/// The fields of the uniforms, `u.<field>`.
#[rustfmt::skip]
pub const FIELDS: &[Doc] = &[
    doc("resolution", ": vec2<f32>", "Canvas size, in pixels."),
    doc("time", ": f32", "Seconds since start."),
    doc("beat", ": f32", "1.0 on a beat, then fading out."),
    doc("bass", ": f32", "Average loudness below 250 Hz, 0..1."),
    doc("mid", ": f32", "Average loudness from 250 Hz to 4 kHz, 0..1."),
    doc("treble", ": f32", "Average loudness above 4 kHz, 0..1."),
    doc("params", ": vec4<f32>", "4 free values, 0..1, set with the sliders of the Tuning panel (.x .y .z .w)."),
];

/// A short explanation of a type name, if it is one.
pub fn type_doc(name: &str) -> Option<String> {
    let vector = |size: &str, of: &str| {
        let components = [".x", ".y", ".z", ".w"][..size.parse::<usize>().unwrap_or(4)].join(" ");
        format!("Vector of {size} {of}. Components: {components} (or .r .g .b .a).")
    };
    let doc = match name {
        "f32" => "32-bit floating point number.".to_owned(),
        "f16" => "16-bit floating point number (needs `enable f16;`).".to_owned(),
        "i32" => "32-bit signed integer.".to_owned(),
        "u32" => "32-bit unsigned integer. Literals end with u, e.g. 3u.".to_owned(),
        "bool" => "true or false.".to_owned(),
        "array" => "Fixed-size array: array<f32, 4>.".to_owned(),
        "atomic" => "Integer safe to modify from several threads.".to_owned(),
        "ptr" => "Pointer: ptr<function, f32>.".to_owned(),
        "sampler" => "How a texture is read (filtering, wrapping).".to_owned(),
        _ => {
            if let Some(rest) = name.strip_prefix("vec") {
                let (size, suffix) = rest.split_at(1.min(rest.len()));
                let of = match suffix {
                    "f" => "f32".to_owned(),
                    "i" => "i32".to_owned(),
                    "u" => "u32".to_owned(),
                    _ => format!("components, e.g. {name}<f32>"),
                };
                vector(size, &of)
            } else {
                let rest = name.strip_prefix("mat")?;
                let (columns, rows) = rest.trim_end_matches('f').split_once('x')?;
                let of = if rest.ends_with('f') {
                    "f32".to_owned()
                } else {
                    format!("components, e.g. {name}<f32>")
                };
                format!("Matrix of {columns} columns and {rows} rows, of {of}.")
            }
        }
    };
    TYPES.contains(&name).then_some(doc)
}
