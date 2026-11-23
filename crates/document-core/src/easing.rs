use serde::{Deserialize, Serialize};

/// An outgoing key segment. The controls of a custom cubic are in normalized time and value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Easing {
    Hold,
    #[default]
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
    Cubic {
        out: [f32; 2],
        incoming: [f32; 2],
    },
}
type P = [f64; 2];
type Cubic = [P; 4];
impl Easing {
    pub fn label(self) -> &'static str {
        match self {
            Self::Hold => "Hold",
            Self::Linear => "Linear",
            Self::EaseIn => "Ease in",
            Self::EaseOut => "Ease out",
            Self::EaseInOut => "Ease in-out",
            Self::Cubic { .. } => "Custom Bézier",
        }
    }
    pub fn controls(self) -> ([f32; 2], [f32; 2]) {
        match self {
            Self::EaseIn => ([0.42, 0.], [1., 1.]),
            Self::EaseOut => ([0., 0.], [0.58, 1.]),
            Self::EaseInOut => ([0.42, 0.], [0.58, 1.]),
            Self::Cubic { out, incoming } => (out, incoming),
            _ => ([1. / 3.; 2], [2. / 3.; 2]),
        }
    }
    pub fn valid(self) -> bool {
        let (a, b) = self.controls();
        a.into_iter().chain(b).all(|n| (0. ..=1.).contains(&n))
    }
    fn points(self) -> Cubic {
        let (a, b) = self.controls();
        [[0.; 2], a.map(f64::from), b.map(f64::from), [1.; 2]]
    }
    /// Solve x(u)=time before evaluating y(u); the same evaluation drives the graph and renderer.
    pub fn sample(self, time: f64) -> f64 {
        let time = time.clamp(0., 1.);
        if time == 0. || time == 1. || self == Self::Linear {
            return time;
        }
        if self == Self::Hold {
            return 0.;
        }
        let points = self.points();
        let u = time_parameter(points, time);
        point(points, u)[1]
    }
    /// Reparameterize a portion after a timeline splice to keep the original motion within it.
    pub(crate) fn portion(self, from: f64, to: f64) -> Self {
        if matches!(self, Self::Linear | Self::Hold) || (from == 0. && to == 1.) {
            return self;
        }
        if from >= to {
            return Self::Hold;
        }
        let points = self.points();
        let start = time_parameter(points, from);
        let end = time_parameter(points, to);
        let (left, _) = split(points, end);
        let (_, segment) = split(left, start / end);
        let delta = [segment[3][0] - segment[0][0], segment[3][1] - segment[0][1]];
        if delta[0] <= f64::EPSILON || delta[1] <= f64::EPSILON {
            return Self::Linear;
        }
        let unit = |p: P| {
            std::array::from_fn(|i| ((p[i] - segment[0][i]) / delta[i]).clamp(0., 1.) as f32)
        };
        Self::Cubic {
            out: unit(segment[1]),
            incoming: unit(segment[2]),
        }
    }
}
fn point(p: Cubic, t: f64) -> P {
    let s = 1. - t;
    std::array::from_fn(|i| {
        s * s * s * p[0][i]
            + 3. * s * s * t * p[1][i]
            + 3. * s * t * t * p[2][i]
            + t * t * t * p[3][i]
    })
}
fn time_parameter(points: Cubic, t: f64) -> f64 {
    if t <= 0. || t >= 1. {
        return t.clamp(0., 1.);
    }
    let (mut left, mut right) = (0., 1.);
    for _ in 0..36 {
        let middle = (left + right) * 0.5;
        if point(points, middle)[0] < t {
            left = middle;
        } else {
            right = middle;
        }
    }
    (left + right) * 0.5
}
fn split(p: Cubic, t: f64) -> (Cubic, Cubic) {
    let mix = |a: P, b: P| std::array::from_fn(|i| a[i] * (1. - t) + b[i] * t);
    let a = mix(p[0], p[1]);
    let b = mix(p[1], p[2]);
    let c = mix(p[2], p[3]);
    let d = mix(a, b);
    let e = mix(b, c);
    let mid = mix(d, e);
    ([p[0], a, d, mid], [mid, e, c, p[3]])
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn curves_invert_time_and_split_without_changing_a_segment() {
        let curve = Easing::Cubic {
            out: [0.95, 0.],
            incoming: [0.95, 0.6],
        };
        assert!(curve.sample(0.5) < 0.15);
        assert_eq!(curve.sample(1.), 1.);
        assert!((Easing::EaseInOut.sample(0.5) - 0.5).abs() < 1e-7);
        let (a, b) = (0.2, 0.7);
        let y0 = curve.sample(a);
        let y1 = curve.sample(b);
        let partial = curve.portion(a, b);
        for i in 0..=100 {
            let t = f64::from(i) / 100.;
            assert!(
                (y0 + (y1 - y0) * partial.sample(t) - curve.sample(a + t * (b - a))).abs() < 1e-6
            );
        }
        assert!(!Easing::Cubic {
            out: [-0.1, 0.],
            incoming: [1., 1.]
        }
        .valid());
    }
}
