#[derive(Clone, Debug)]
pub enum Spline {
    Constant(f32),
    Multipoint {
        coordinate: u32,
        locations: Vec<f32>,
        values: Vec<Spline>,
        derivatives: Vec<f32>,
    },
}

#[inline]
fn lerp_f32(delta: f32, start: f32, end: f32) -> f32 {
    start + delta * (end - start)
}

fn find_interval_start(locations: &[f32], value: f32) -> i32 {
    let mut start = 0usize;
    let mut remaining = locations.len();
    while remaining > 0 {
        let half = remaining / 2;
        let middle = start + half;
        if value < locations[middle] {
            remaining = half;
        } else {
            start = middle + 1;
            remaining -= half + 1;
        }
    }
    start as i32 - 1
}

impl Spline {
    pub fn evaluate(&self, coordinate: &mut dyn FnMut(u32) -> f32) -> f32 {
        match self {
            Spline::Constant(value) => *value,
            Spline::Multipoint {
                coordinate: coordinate_node,
                locations,
                values,
                derivatives,
            } => {
                let input = coordinate(*coordinate_node);
                let interval = find_interval_start(locations, input);
                let last = locations.len() as i32 - 1;
                if interval < 0 {
                    values[0].evaluate(coordinate) + derivatives[0] * (input - locations[0])
                } else if interval == last {
                    let index = last as usize;
                    values[index].evaluate(coordinate)
                        + derivatives[index] * (input - locations[index])
                } else {
                    let index = interval as usize;
                    let start = locations[index];
                    let end = locations[index + 1];
                    let delta = (input - start) / (end - start);
                    let lower = values[index].evaluate(coordinate);
                    let upper = values[index + 1].evaluate(coordinate);
                    let slope_start = derivatives[index];
                    let slope_end = derivatives[index + 1];
                    let first = slope_start * (end - start) - (upper - lower);
                    let second = -slope_end * (end - start) + (upper - lower);
                    lerp_f32(delta, lower, upper)
                        + delta * (1.0 - delta) * lerp_f32(delta, first, second)
                }
            }
        }
    }

    pub fn for_each_coordinate(&self, visit: &mut dyn FnMut(u32)) {
        if let Spline::Multipoint {
            coordinate, values, ..
        } = self
        {
            visit(*coordinate);
            for value in values {
                value.for_each_coordinate(visit);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_point(slope_low: f32, slope_high: f32) -> Spline {
        Spline::Multipoint {
            coordinate: 0,
            locations: vec![-1.0, 1.0],
            values: vec![Spline::Constant(0.0), Spline::Constant(1.0)],
            derivatives: vec![slope_low, slope_high],
        }
    }

    fn at(spline: &Spline, input: f32) -> f32 {
        spline.evaluate(&mut |_| input)
    }

    #[test]
    fn constant_ignores_coordinate() {
        assert_eq!(at(&Spline::Constant(3.5), 100.0), 3.5);
    }

    #[test]
    fn midpoint_of_flat_segment() {
        assert_eq!(at(&two_point(0.0, 0.0), 0.0), 0.5);
    }

    #[test]
    fn interpolates_through_control_points() {
        let spline = two_point(0.25, -0.5);
        assert_eq!(at(&spline, -1.0), 0.0);
        assert!((at(&spline, 1.0) - 1.0).abs() < 1.0e-6);
    }

    #[test]
    fn extrapolates_with_end_derivatives() {
        let spline = two_point(0.5, 2.0);
        assert_eq!(at(&spline, -3.0), -1.0);
        assert_eq!(at(&spline, 2.0), 3.0);
    }

    #[test]
    fn interval_search_matches_linear_scan() {
        let locations = [-2.0_f32, -0.5, 0.0, 0.25, 3.0];
        for step in -60..80 {
            let value = step as f32 * 0.1;
            let expected = locations.iter().filter(|point| value >= **point).count() as i32 - 1;
            assert_eq!(
                find_interval_start(&locations, value),
                expected,
                "value {value}"
            );
        }
    }

    #[test]
    fn nested_splines_evaluate_children() {
        let inner = Spline::Multipoint {
            coordinate: 1,
            locations: vec![0.0, 1.0],
            values: vec![Spline::Constant(10.0), Spline::Constant(20.0)],
            derivatives: vec![0.0, 0.0],
        };
        let outer = Spline::Multipoint {
            coordinate: 0,
            locations: vec![0.0, 1.0],
            values: vec![inner, Spline::Constant(0.0)],
            derivatives: vec![0.0, 0.0],
        };
        let mut visited = Vec::new();
        outer.for_each_coordinate(&mut |node| visited.push(node));
        assert_eq!(visited, vec![0, 1]);
        let value = outer.evaluate(&mut |node| if node == 0 { 0.0 } else { 0.5 });
        assert_eq!(value, 15.0);
    }
}
