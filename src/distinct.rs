use core::f64 as scalar;
use std::io::{self, Write};


use rand::prelude::*;

use crate::delta_e;
use crate::ansi::{Brush,Stream,ToAnsiStyle};
use crate::error::{PastelError,Result};
use crate::random::{self, RandomizationStrategy};
use crate::{Color, Lab};

type Scalar = f64;

#[derive(Clone)]
pub struct DistanceResult {
    /// The closest distance between any two colors
    pub min_closest_distance: Scalar,

    /// The average over all nearest-neighbor distances
    pub mean_closest_distance: Scalar,

    /// Indices of the colors that were closest to each other
    pub closest_pair: (usize, usize),

    /// The closest distance and the index of the nearest neighbor
    pub closest_distances: Vec<(Scalar, usize)>,

    pub distance_metric: DistanceMetric,
}

pub struct IterationStatistics<'a> {
    pub iteration: usize,
    pub temperature: Scalar,
    pub distance_result: &'a DistanceResult,
    pub colors: Vec<Color>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OptimizationTarget {
    Mean,
    Min,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OptimizationMode {
    Global,
    Local,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DistanceMetric {
    CIE76,
    CIEDE2000,
}

pub struct SimulationParameters {
    pub initial_temperature: Scalar,
    pub cooling_rate: Scalar,
    pub num_iterations: usize,
    pub opt_target: OptimizationTarget,
    pub opt_mode: OptimizationMode,
    pub distance_metric: DistanceMetric,
}

pub struct SimulatedAnnealing {
    colors: Vec<(Color, Lab)>,
    temperature: Scalar,
    pub parameters: SimulationParameters,
}

impl SimulatedAnnealing {
    pub fn new(initial_colors: &[Color], parameters: SimulationParameters) -> Self {
        let colors = initial_colors
            .iter()
            .map(|c| (c.clone(), c.to_lab()))
            .collect();

        SimulatedAnnealing {
            colors,
            temperature: parameters.initial_temperature,
            parameters,
        }
    }

    pub fn get_colors(&self) -> Vec<Color> {
        self.colors.iter().map(|(c, _)| c.clone()).collect()
    }

    fn modify_channel(c: &mut u8) {
        if random::<bool>() {
            *c = c.saturating_add(random::<u8>() % 10);
        } else {
            *c = c.saturating_sub(random::<u8>() % 10);
        }
    }

    fn modify_color(&self, color: &mut (Color, Lab)) {
        const STRATEGY: random::strategies::UniformRGB = random::strategies::UniformRGB {};

        match self.parameters.opt_mode {
            OptimizationMode::Local => {
                let mut rgb = color.0.to_rgba();
                Self::modify_channel(&mut rgb.r);
                Self::modify_channel(&mut rgb.g);
                Self::modify_channel(&mut rgb.b);
                color.0 = Color::from_rgb(rgb.r, rgb.g, rgb.b);
            }
            OptimizationMode::Global => {
                color.0 = STRATEGY.generate();
            }
        }
        color.1 = color.0.to_lab();
    }

    pub fn run(&mut self, callback: &mut dyn FnMut(&IterationStatistics)) -> DistanceResult {
        self.temperature = self.parameters.initial_temperature;

        let mut result = DistanceResult::new(&self.colors, self.parameters.distance_metric);

        for iter in 0..self.parameters.num_iterations {
            let random_index = if self.parameters.opt_target == OptimizationTarget::Mean {
                random::<usize>() % self.colors.len()
            } else {
                if random::<bool>() {
                    result.closest_pair.0
                } else {
                    result.closest_pair.1
                }
            };

            let mut new_colors = self.colors.clone();

            self.modify_color(&mut new_colors[random_index]);

            let new_result = result.update(&new_colors, random_index);

            let (score, new_score) = match self.parameters.opt_target {
                OptimizationTarget::Mean => (
                    result.mean_closest_distance,
                    new_result.mean_closest_distance,
                ),
                OptimizationTarget::Min => {
                    (result.min_closest_distance, new_result.min_closest_distance)
                }
            };

            if new_score > score {
                result = new_result;
                self.colors = new_colors;
            } else {
                let bolzmann = Scalar::exp(-(score - new_score) / self.temperature);
                if random::<Scalar>() <= bolzmann {
                    result = new_result;
                    self.colors = new_colors;
                }
            }

            if iter % 5_000 == 0 {
                let statistics = IterationStatistics {
                    iteration: iter,
                    temperature: self.temperature,
                    distance_result: &result,
                    colors: self.get_colors(),
                };
                callback(&statistics);
            }

            if iter % 1_000 == 0 {
                self.temperature *= self.parameters.cooling_rate;
            }
        }

        result
    }
}

fn print_iteration(out: &mut dyn Write, brush: &Brush, stats: &IterationStatistics) -> Result<()> {
    let result = stats.distance_result;
    write!(
        out,
        "[{:10.}] D_mean = {:<6.2}; D_min = {:<6.2}; T = {:.6} ",
        stats.iteration,
        result.mean_closest_distance,
        result.min_closest_distance,
        stats.temperature
    )?;
    print_colors(out, brush, &stats.colors, Some(result.closest_pair))?;
    Ok(())
}

fn print_colors(
    out: &mut dyn Write,
    brush: &Brush,
    colors: &[Color],
    closest_pair: Option<(usize, usize)>,
) -> Result<()> {
    let mut ci = 0;
    for c in colors.iter() {
        let tc = c.text_color();
        let mut style = tc.ansi_style();
        style.on(c);

        if let Some(pair) = closest_pair {
            if pair.0 == ci || pair.1 == ci {
                style.bold(true);
                style.underline(true);
            }
        }

        write!(
            out,
            "{} ",
            brush.paint(format!("{}", c.to_rgb_hex_string(false)), style)
        )?;

        ci += 1;
    }
    writeln!(out, "")?;
    Ok(())
}

pub fn distinct_colors(
    count: usize,
    distance_metric: DistanceMetric,
    verbose: bool,
    arrange_colors: bool
) -> Result<(Vec<Color>, DistanceResult)> {

    if count < 2 {
        return Err(PastelError::DistinctColorCountMustBeLargerThanOne);
    }

    let stderr = io::stderr();
    let brush_stderr = Brush::from_environment(Stream::Stderr);

    let mut colors = Vec::new();
    for _ in 0..count {
        colors.push(random::strategies::UniformRGB.generate());
    }

    let mut annealing = SimulatedAnnealing::new(
        &colors,
        SimulationParameters {
            initial_temperature: 3.0,
            cooling_rate: 0.95,
            num_iterations: 100_000,
            opt_target: OptimizationTarget::Mean,
            opt_mode: OptimizationMode::Global,
            distance_metric,
        },
    );

    let mut callback: Box<dyn FnMut(&IterationStatistics)> = if verbose {
        Box::new(|stats: &IterationStatistics| {
            print_iteration(&mut stderr.lock(), &brush_stderr, stats).ok();
        })
    } else {
        Box::new(|_: &IterationStatistics| {})
    };

    annealing.run(callback.as_mut());

    annealing.parameters.initial_temperature = 0.5;
    annealing.parameters.cooling_rate = 0.98;
    annealing.parameters.num_iterations = 200_000;
    annealing.parameters.opt_target = OptimizationTarget::Min;
    annealing.parameters.opt_mode = OptimizationMode::Local;

    let result = annealing.run(callback.as_mut());

    let mut colors = annealing.get_colors();
    if arrange_colors {
        rearrange_sequence(&mut colors, distance_metric);
    }

    Ok((colors, result))
}


/// Re-arrange the sequence of colors such that the minimal difference between a given color and
/// any of its predecessors is maximized.
///
/// Note: this is only a heuristic and will not yield optimal results (especially at the end of
/// the sequence).
///
/// See: https://en.wikipedia.org/wiki/Farthest-first_traversal
pub fn rearrange_sequence(colors: &mut Vec<Color>, metric: DistanceMetric) {
    let distance = |c1: &Color, c2: &Color| match metric {
        DistanceMetric::CIE76 => c1.distance_delta_e_cie76(c2),
        DistanceMetric::CIEDE2000 => c1.distance_delta_e_ciede2000(c2),
    };

    // vector where the i-th element contains the minimum distance to the colors from 0 to i-1.
    let mut min_distances = vec![i32::max_value(); colors.len()];

    for i in 1..colors.len() {
        let mut max_i = colors.len();
        let mut max_d = i32::min_value();

        for j in i..colors.len() {
            min_distances[j] =
                min_distances[j].min((distance(&colors[j], &colors[i - 1]) * 1000.0) as i32);

            if min_distances[j] > max_d {
                max_i = j;
                max_d = min_distances[j];
            }
        }

        colors.swap(i, max_i);
        min_distances.swap(i, max_i);
    }
}

impl DistanceResult {
    fn new(colors: &[(Color, Lab)], distance_metric: DistanceMetric) -> Self {
        let mut result = DistanceResult {
            closest_distances: vec![(scalar::MAX, std::usize::MAX); colors.len()],
            closest_pair: (std::usize::MAX, std::usize::MAX),
            mean_closest_distance: 0.0,
            min_closest_distance: scalar::MAX,
            distance_metric,
        };

        for i in 0..colors.len() {
            result.update_distances(colors, i, false);
        }
        result.update_totals();

        result
    }

    fn update(&self, colors: &[(Color, Lab)], changed_color: usize) -> Self {
        let mut result = self.clone();
        result.update_distances(colors, changed_color, true);
        result.update_totals();
        result
    }

    fn update_distances(&mut self, colors: &[(Color, Lab)], color: usize, changed: bool) {
        self.closest_distances[color] = (scalar::MAX, std::usize::MAX);

        // we need to recalculate distances for nodes where the previous min dist was with
        // changed_color but it's not anymore (potentially).
        let mut to_recalc = Vec::with_capacity(colors.len());

        for (i, c) in colors.iter().enumerate() {
            if i == color {
                continue;
            }

            let dist = self.distance(c, &colors[color]);

            if dist < self.closest_distances[i].0 {
                self.closest_distances[i] = (dist, color);
            } else if changed && self.closest_distances[i].1 == color {
                // changed_color was the best before, but unfortunately we cannot say it now for
                // sure because the distance between the two increased. Play it safe and just
                // recalculate its distances.
                to_recalc.push(i);
            }

            if dist < self.closest_distances[color].0 {
                self.closest_distances[color] = (dist, i);
            }
        }

        for i in to_recalc {
            self.update_distances(colors, i, false);
        }
    }

    fn update_totals(&mut self) {
        self.mean_closest_distance = 0.0;
        self.min_closest_distance = scalar::MAX;

        for (i, (dist, closest_i)) in self.closest_distances.iter().enumerate() {
            self.mean_closest_distance += *dist;

            if *dist < self.min_closest_distance {
                self.min_closest_distance = *dist;
                self.closest_pair = (i, *closest_i);
            }
        }

        self.mean_closest_distance /= self.closest_distances.len() as Scalar;
    }

    fn distance(&self, a: &(Color, Lab), b: &(Color, Lab)) -> Scalar {
        match self.distance_metric {
            DistanceMetric::CIE76 => delta_e::cie76(&a.1, &b.1),
            DistanceMetric::CIEDE2000 => delta_e::ciede2000(&a.1, &b.1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{rearrange_sequence, DistanceMetric};
    use crate::Color;

    #[test]
    fn test_rearrange_sequence() {
        let mut colors = vec![
            Color::white(),
            Color::graytone(0.25),
            Color::graytone(0.5),
            Color::graytone(0.8),
            Color::black(),
        ];

        rearrange_sequence(&mut colors, DistanceMetric::CIE76);

        assert_eq!(
            colors,
            vec![
                Color::white(),
                Color::black(),
                Color::graytone(0.5),
                Color::graytone(0.25),
                Color::graytone(0.8),
            ]
        );
    }
}
