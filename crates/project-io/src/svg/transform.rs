use super::pathdata::numbers;
use crate::invalid;
use std::io;

/// SVG's column-vector matrix [a c e; b d f; 0 0 1].
#[derive(Clone, Copy)]
pub(super) struct Matrix(pub [f32; 6]);

impl Matrix {
    pub(super) fn transform(self) -> tiny_skia::Transform {
        let [a, b, c, d, x, y] = self.0;
        tiny_skia::Transform::from_row(a, b, c, d, x, y)
    }
    pub const IDENTITY: Self = Self([1., 0., 0., 1., 0., 0.]);

    pub fn translate(x: f32, y: f32) -> Self {
        Self([1., 0., 0., 1., x, y])
    }

    pub fn scale(x: f32, y: f32) -> Self {
        Self([x, 0., 0., y, 0., 0.])
    }

    pub fn append(self, rhs: Self) -> Self {
        let [a, b, c, d, e, f] = self.0;
        let [u, v, w, x, y, z] = rhs.0;
        Self([
            a * u + c * v,
            b * u + d * v,
            a * w + c * x,
            b * w + d * x,
            a * y + c * z + e,
            b * y + d * z + f,
        ])
    }

    pub fn point(self, [x, y]: [f32; 2]) -> [f32; 2] {
        let [a, b, c, d, e, f] = self.0;
        [a * x + c * y + e, b * x + d * y + f]
    }

    pub fn axes(self) -> (f32, f32, bool) {
        let [a, b, c, d, _, _] = self.0;
        let sx = a.hypot(b);
        let sy = c.hypot(d);
        (sx, sy, (a * c + b * d).abs() <= 0.00001 * sx * sy)
    }

    pub fn parse(mut text: &str) -> io::Result<Self> {
        let mut matrix = Self::IDENTITY;
        while !text.trim().is_empty() {
            let (name, args) = text
                .trim()
                .split_once('(')
                .ok_or_else(|| invalid("Invalid SVG transform."))?;
            let (args, rest) = args
                .split_once(')')
                .ok_or_else(|| invalid("Unclosed SVG transform."))?;
            let values = numbers(args)?;
            let step = match (name.trim(), values.as_slice()) {
                ("matrix", &[a, b, c, d, e, f]) => Self([a, b, c, d, e, f]),
                ("translate", &[x]) => Self::translate(x, 0.),
                ("translate", &[x, y]) => Self::translate(x, y),
                ("scale", &[s]) => Self::scale(s, s),
                ("scale", &[x, y]) => Self::scale(x, y),
                ("rotate", &[angle]) | ("rotate", &[angle, _, _]) => {
                    let (s, c) = angle.to_radians().sin_cos();
                    let (x, y) = if values.len() == 3 {
                        (values[1], values[2])
                    } else {
                        (0., 0.)
                    };
                    Self::translate(x, y)
                        .append(Self([c, s, -s, c, 0., 0.]))
                        .append(Self::translate(-x, -y))
                }
                _ => {
                    return Err(invalid(format!(
                        "Unsupported SVG transform '{name}' or invalid arguments."
                    )))
                }
            };
            matrix = matrix.append(step);
            text = rest.trim_start().strip_prefix(',').unwrap_or(rest);
        }
        if !matrix.0.iter().all(|n| n.is_finite()) {
            return Err(invalid("SVG transform overflow."));
        }
        Ok(matrix)
    }
}
