use serde::{Deserialize, Serialize};
use std::ops::RangeInclusive;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GridKind {
    #[default]
    Rectangular,
    Axonometric,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GridVisibility {
    #[default]
    WhenSnapping,
    Visible,
    Hidden,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GridSettings {
    pub kind: GridKind,
    pub visibility: GridVisibility,
    pub origin: [f32; 2],
    pub spacing: f32,
    pub angle_x: f32,
    pub angle_z: f32,
}

impl Default for GridSettings {
    fn default() -> Self {
        Self {
            kind: GridKind::Rectangular,
            visibility: GridVisibility::WhenSnapping,
            origin: [0.; 2],
            spacing: 1.,
            angle_x: 30.,
            angle_z: 30.,
        }
    }
}

impl GridSettings {
    pub const SPACING_RANGE: RangeInclusive<f32> = 0.25..=4096.;
    pub const ANGLE_RANGE: RangeInclusive<f32> = 1.0..=89.0;
    pub const ORIGIN_RANGE: RangeInclusive<f32> = -1_000_000.0..=1_000_000.0;

    pub(crate) fn is_default(&self) -> bool {
        *self == Self::default()
    }

    pub fn validate(self) -> Result<(), String> {
        if !Self::SPACING_RANGE.contains(&self.spacing) {
            return Err("Grid spacing must be between 0.25 and 4096 px.".into());
        }
        if ![self.angle_x, self.angle_z]
            .iter()
            .all(|angle| Self::ANGLE_RANGE.contains(angle))
        {
            return Err("Grid angles must be between 1° and 89°.".into());
        }
        if !self.origin.iter().all(|p| Self::ORIGIN_RANGE.contains(p)) {
            return Err("Grid origin must be within ±1,000,000 px on each axis.".into());
        }
        Ok(())
    }

    pub fn geometry(self) -> GridGeometry {
        let slopes = (self.kind == GridKind::Axonometric).then(|| {
            [self.angle_x, -self.angle_z].map(|angle| f64::from(angle).to_radians().tan())
        });
        GridGeometry {
            origin: self.origin.map(f64::from),
            spacing: f64::from(self.spacing),
            slopes,
        }
    }
}

#[derive(Clone, Copy)]
pub struct GridGeometry {
    origin: [f64; 2],
    spacing: f64,
    slopes: Option<[f64; 2]>,
}

impl GridGeometry {
    /// Column separation, measured horizontally between vertical lines.
    pub fn column_spacing(self) -> f64 {
        self.slopes
            .map_or(self.spacing, |[x, z]| self.spacing / (x - z))
    }

    /// Rows intersect the vertical axis at multiples of the configured spacing.
    pub fn row_slopes(self) -> impl Iterator<Item = f64> {
        let [first, second] = self
            .slopes
            .map_or([Some(0.), None], |slopes| slopes.map(Some));
        [first, second].into_iter().flatten()
    }

    pub fn nearest_intersection(self, point: [f32; 2]) -> [f32; 2] {
        let [ox, oy] = self.origin;
        let [px, py] = point.map(f64::from);
        let (x, y) = (px - ox, py - oy);
        let step = self.column_spacing();
        let column = (x / step).round();
        let slope = self.slopes.map_or(0., |slopes| slopes[0]);
        let in_column = |column: f64| {
            let cx = column * step;
            let base = cx * slope;
            [
                cx,
                base + ((y - base) / self.spacing).round() * self.spacing,
            ]
        };
        let distance_sq = |[cx, cy]: [f64; 2]| (cx - x).powi(2) + (cy - y).powi(2);
        let mut best = in_column(column);
        if self.slopes.is_some() {
            // With a skewed lattice the closest vertex may be on a different
            // column. The first hit bounds how far horizontally we must look.
            let mut best_distance = distance_sq(best);
            let reach = best_distance.sqrt();
            let first = (((x - reach) / step).ceil() - column) as i32;
            let last = (((x + reach) / step).floor() - column) as i32;
            for offset in first..=last {
                let candidate = in_column(column + f64::from(offset));
                let distance = distance_sq(candidate);
                if distance < best_distance {
                    best = candidate;
                    best_distance = distance;
                }
            }
        }
        [(ox + best[0]) as f32, (oy + best[1]) as f32]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skewed_grid_can_snap_to_a_vertex_in_a_farther_column() {
        let grid = GridSettings {
            kind: GridKind::Axonometric,
            origin: [10., -20.],
            spacing: 24.,
            angle_x: 78.,
            angle_z: 23.,
            ..Default::default()
        }
        .geometry();
        let [x, y] = grid.nearest_intersection([11., -9.]);
        let column = grid.column_spacing();
        let slope = grid.row_slopes().next().unwrap();
        assert!((f64::from(x) - (10. - column)).abs() < 1e-5);
        assert!((f64::from(y) - (4. - slope * column)).abs() < 1e-5);
    }
}
