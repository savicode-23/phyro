use crate::invalid;
use document_core::{PathNode, Shape, MAX_PATH_NODES};
use std::io;
mod arc;

struct Cursor<'a> {
    text: &'a str,
    offset: usize,
    after_number: bool,
}

impl<'a> Cursor<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            offset: 0,
            after_number: false,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.offset).copied()
    }

    fn space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.offset += 1;
        }
    }

    fn done(&mut self) -> bool {
        self.space();
        self.peek().is_none()
    }

    fn digits(&mut self) -> usize {
        let start = self.offset;
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.offset += 1;
        }
        self.offset - start
    }

    fn sign(&mut self) {
        if matches!(self.peek(), Some(b'+' | b'-')) {
            self.offset += 1;
        }
    }

    fn separator(&mut self) {
        self.space();
        if self.after_number && self.peek() == Some(b',') {
            self.offset += 1;
            self.space();
        }
    }

    fn flag(&mut self) -> io::Result<bool> {
        self.separator();
        let flag = match self.peek() {
            Some(b'0') => false,
            Some(b'1') => true,
            _ => return Err(invalid("SVG arc flags must be 0 or 1.")),
        };
        self.offset += 1;
        self.after_number = true;
        Ok(flag)
    }

    fn number(&mut self) -> io::Result<f32> {
        self.separator();
        let start = self.offset;
        self.sign();
        let mut digits = self.digits();
        if self.peek() == Some(b'.') {
            self.offset += 1;
            digits += self.digits();
        }
        if digits == 0 {
            return Err(invalid(format!("Expected an SVG number at byte {start}.")));
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.offset += 1;
            self.sign();
            if self.digits() == 0 {
                return Err(invalid(format!("Incomplete SVG exponent at byte {start}.")));
            }
        }
        let value = self.text[start..self.offset]
            .parse::<f32>()
            .map_err(invalid_number)?;
        self.after_number = true;
        finite(value)
    }

    fn command(&mut self) -> Option<char> {
        let letter = self.peek()?.is_ascii_alphabetic();
        if !letter {
            return None;
        }
        let command = self.text.as_bytes()[self.offset] as char;
        self.offset += 1;
        self.after_number = false;
        Some(command)
    }

    fn point(&mut self, origin: [f32; 2]) -> io::Result<[f32; 2]> {
        Ok([
            finite(origin[0] + self.number()?)?,
            finite(origin[1] + self.number()?)?,
        ])
    }
}

fn invalid_number(error: std::num::ParseFloatError) -> io::Error {
    invalid(format!("Invalid SVG number: {error}"))
}

fn finite(value: f32) -> io::Result<f32> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(invalid("SVG coordinate is outside the supported range."))
    }
}

/// SVG numbers can use exponents, omit a leading zero, and run together before signs.
pub(super) fn numbers(text: &str) -> io::Result<Vec<f32>> {
    let mut cursor = Cursor::new(text);
    let mut values = Vec::new();
    while !cursor.done() {
        values.push(cursor.number()?);
    }
    Ok(values)
}

fn corner(point: [f32; 2]) -> PathNode {
    PathNode::corner(point)
}

fn quadratic_handle(endpoint: [f32; 2], control: [f32; 2]) -> [f32; 2] {
    // A degree-elevated quadratic has cubic handles two thirds of the way to Q.
    std::array::from_fn(|i| ((f64::from(endpoint[i]) + 2.0 * f64::from(control[i])) / 3.0) as f32)
}

/// Parse cubic contours in absolute SVG coordinates.
pub(super) fn path_data(text: &str) -> io::Result<Shape> {
    let mut cursor = Cursor::new(text);
    let mut nodes = Vec::<PathNode>::new();
    let mut contours = Vec::new();
    let mut completed_nodes = 0;
    let mut repeat = None;
    let mut closed = false;

    while !cursor.done() {
        let command = cursor
            .command()
            .or(repeat)
            .ok_or_else(|| invalid("Expected an SVG path command."))?;
        let op = command.to_ascii_uppercase();
        let current = if closed {
            nodes[0].point
        } else {
            nodes.last().map_or([0.0; 2], |n| n.point)
        };
        let origin = if command.is_ascii_lowercase() {
            current
        } else {
            [0.0; 2]
        };
        if nodes.is_empty() && op != 'M' {
            return Err(invalid("An SVG path must begin with a moveto."));
        }
        if op == 'M' && !nodes.is_empty() {
            if !closed || nodes.len() < 2 {
                return Err(invalid(
                    "Compound paths require closed contours with at least two anchors.",
                ));
            }
            completed_nodes += nodes.len();
            contours.push(std::mem::take(&mut nodes));
            closed = false;
        } else if closed {
            return Err(invalid("Expected a moveto after a closed SVG contour."));
        }
        repeat = Some(command);
        match op {
            'M' => {
                nodes.push(corner(cursor.point(origin)?));
                repeat = Some(if command == 'M' { 'L' } else { 'l' });
            }
            'L' => nodes.push(corner(cursor.point(origin)?)),
            'H' => nodes.push(corner([finite(origin[0] + cursor.number()?)?, current[1]])),
            'V' => nodes.push(corner([current[0], finite(origin[1] + cursor.number()?)?])),
            'A' => {
                let radii = [cursor.number()?, cursor.number()?];
                let rotation = cursor.number()?;
                let large = cursor.flag()?;
                let sweep = cursor.flag()?;
                let end = cursor.point(origin)?;
                arc::append(&mut nodes, radii, rotation, large, sweep, end)?;
            }
            'C' | 'Q' => {
                // All parameters of a relative curve use the same initial point.
                let handle = cursor.point(origin)?;
                let second = cursor.point(origin)?;
                let (outgoing, incoming, point) = if op == 'C' {
                    (handle, second, cursor.point(origin)?)
                } else {
                    (
                        quadratic_handle(current, handle),
                        quadratic_handle(second, handle),
                        second,
                    )
                };
                nodes.last_mut().expect("moveto was checked").outgoing = outgoing;
                nodes.push(PathNode {
                    handle_style: Default::default(),
                    point,
                    incoming,
                    outgoing: point,
                });
            }
            'Z' => {
                closed = true;
                repeat = None;
                // A cubic returning explicitly to the start carries the first node's incoming
                // handle. Preserve two anchors for a single-segment loop.
                if nodes.len() > 2 && current == nodes[0].point {
                    let end = nodes.pop().expect("at least three nodes");
                    nodes[0].incoming = end.incoming;
                }
            }
            _ => {
                return Err(invalid(format!(
                    "Unsupported SVG path command '{command}'."
                )))
            }
        }
        if completed_nodes + nodes.len() > MAX_PATH_NODES {
            return Err(invalid("SVG path exceeds the node limit."));
        }
    }
    if nodes.len() < 2 {
        return Err(invalid("An editable SVG path needs at least two anchors."));
    }
    if contours.is_empty() {
        Ok(Shape::Path { nodes, closed })
    } else if closed {
        contours.push(nodes);
        Ok(Shape::Compound { contours })
    } else {
        Err(invalid(
            "All contours of a compound SVG path must be closed.",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_relative_segments_start_at_the_previous_endpoint() {
        let Shape::Path { nodes, closed } = path_data("m1e1,20 5-2 h4 8 v.5-.25 L1 2").unwrap()
        else {
            panic!()
        };
        let points: Vec<_> = nodes.iter().map(|n| n.point).collect();
        assert_eq!(
            points,
            [
                [10., 20.],
                [15., 18.],
                [19., 18.],
                [27., 18.],
                [27., 18.5],
                [27., 18.25],
                [1., 2.]
            ]
        );
        assert!(!closed);
    }

    #[test]
    fn relative_cubic_and_quadratic_handles_survive_closure() {
        let Shape::Path { nodes, closed } = path_data("M3,6 q3,-6 6,0 c0,3 -6,3 -6,0 z").unwrap()
        else {
            panic!()
        };
        assert!(closed);
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].outgoing, [5., 2.]);
        assert_eq!(nodes[1].incoming, [7., 2.]);
        assert_eq!(nodes[1].outgoing, [9., 9.]);
        assert_eq!(nodes[0].incoming, [3., 9.]);
    }

    #[test]
    fn arc_flags_choose_the_sweep_and_cubics_preserve_the_ellipse_and_endpoints() {
        // Adjacent flags and coordinates are valid SVG. Repeated relative arcs
        // begin at the preceding endpoint, including after a zero-radius line.
        let Shape::Path { nodes, .. } = path_data("M10 0 A10 10 0 0110 20 a0 4 5 001 0").unwrap()
        else {
            panic!()
        };
        assert_eq!(nodes[0].point, [10., 0.]);
        assert_eq!(nodes[1].point, [20., 10.]);
        assert_eq!(nodes[2].point, [10., 20.]);
        assert_eq!(nodes[3].point, [11., 20.]);
        assert!((nodes[0].outgoing[0] - 15.52285).abs() < 1e-4);

        for (flag, expected_parts) in [(0, 1), (1, 3)] {
            let data = format!("M0 0 A6 10 90 {flag} 0 10 6");
            let Shape::Path { nodes, .. } = path_data(&data).unwrap() else {
                panic!()
            };
            assert_eq!(nodes.len(), 1 + expected_parts);
            assert_eq!(nodes.last().unwrap().point, [10., 6.]);
        }
    }

    #[test]
    fn malformed_and_unsupported_input_fails() {
        for text in [
            "L0 0",
            "M0 0 L5",
            "M0 0, L5 5",
            "M1 2 V4,,3",
            "M0 0 A2 2 0 0 2 4 4",
            "M0 0 L1 1 M2 2 L3 3",
            "M0 0 l1e80 7",
            "M3e38 0 l3e38 0",
        ] {
            assert!(path_data(text).is_err(), "{text}");
        }
        assert_eq!(numbers(".5, -2e-1 +1.2").unwrap(), [0.5, -0.2, 1.2]);
        for text in ["1,", ",1", "1e,2", "1,,2", "NaN", "infinity", "23px"] {
            assert!(numbers(text).is_err(), "{text}");
        }
    }
}
