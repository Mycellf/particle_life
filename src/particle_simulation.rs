use std::{sync::LazyLock, time::Duration};

use crate::matrix::Matrix;
use macroquad::{
    camera::Camera2D,
    color::{self, Color, colors},
    material::{self, Material},
    math::{Vec2, vec2},
    prelude::{MaterialParams, ShaderSource},
    shapes,
};
use rand::Rng;
use rayon::iter::{IndexedParallelIterator, IntoParallelRefMutIterator, ParallelIterator};

pub type Real = f64;

pub const PARTICLE_RADIUS: Real = 5.0;

#[derive(Clone, Debug)]
pub struct ParticleSimulation {
    buckets: Matrix<Vec<Particle>>,
    impulses: Matrix<Vec<[Real; 2]>>,
    bucket_size: Real,
    pub type_data: ParticleTypeData,
    pub params: ParticleSimulationParams,
    pub metadata: ParticleSimulationMetadata,
}

#[derive(Clone, Copy, Debug)]
pub struct ParticleSimulationParams {
    pub edge_type: EdgeType,
    pub prevent_particle_ejecting: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct ParticleSimulationMetadata {
    pub is_active: bool,
    pub steps: usize,
    pub total_time: Option<Duration>,
    pub tick_time: Option<Duration>,
    pub send_time: Option<Duration>,
    pub tps_limit: Option<usize>,
    pub update_id: u64,
    pub num_particles: usize,
}

impl Default for ParticleSimulationMetadata {
    fn default() -> Self {
        Self {
            is_active: true,
            steps: 0,
            total_time: None,
            tick_time: None,
            send_time: None,
            tps_limit: Some(30),
            update_id: 0,
            num_particles: 0,
        }
    }
}

impl ParticleSimulation {
    pub fn new(
        bucket_size: Real,
        buckets: [usize; 2],
        params: ParticleSimulationParams,
        metadata: ParticleSimulationMetadata,
        type_data: ParticleTypeData,
    ) -> Self {
        Self {
            buckets: Matrix::from_element(buckets, Vec::new()),
            impulses: Matrix::from_element(buckets, Vec::new()),
            bucket_size,
            type_data,
            params,
            metadata,
        }
    }

    pub fn step_simulation(&mut self) {
        self.move_particles();
        self.organize_particles();
    }

    fn move_particles(&mut self) {
        let maximum_distance_squared = self.bucket_size.powi(2);

        // Update particle impulses
        (self.impulses.data.par_iter_mut())
            .enumerate()
            .for_each(|(i, impulses)| {
                let bucket_index = [i % self.buckets.size[0], i / self.buckets.size[0]];
                let bucket = &self.buckets[bucket_index];

                impulses.clear();
                impulses.resize(bucket.len(), [0.0, 0.0]);

                (impulses.par_iter_mut().enumerate()).for_each(|(i, impulse)| {
                    let particle = bucket[i];

                    // Update from own bucket
                    for (j, &other) in bucket.iter().enumerate() {
                        if i == j {
                            continue;
                        }

                        particle.update_impulse_with_particle(
                            other,
                            &self.type_data,
                            &self.params,
                            maximum_distance_squared,
                            impulse,
                        );
                    }

                    #[rustfmt::skip]
                    pub const NEIGHBORS: [[isize; 2]; 8] = [
                        [-1,  1], [ 0,  1], [ 1,  1],
                        [-1,  0],           [ 1,  0],
                        [-1, -1], [ 0, -1], [ 1, -1],
                    ];

                    // Update from neighboring buckets
                    for bucket_relative_index in NEIGHBORS {
                        // SAFETY: wrapping_add_signed will wrap to a very big number if there is
                        // an overflow, as bucket_relative_index[i] is -1, 0, or 1
                        let neighbor_bucket_index = [
                            bucket_index[0].wrapping_add_signed(bucket_relative_index[0]),
                            bucket_index[1].wrapping_add_signed(bucket_relative_index[1]),
                        ];

                        if let Some(neighbor_bucket) = self.buckets.get(neighbor_bucket_index) {
                            for &other in neighbor_bucket {
                                particle.update_impulse_with_particle(
                                    other,
                                    &self.type_data,
                                    &self.params,
                                    maximum_distance_squared,
                                    impulse,
                                );
                            }
                        } else if matches!(self.params.edge_type, EdgeType::Wrapping) {
                            let neighbor_bucket_index = [
                                bucket_index[0].checked_add_signed(bucket_relative_index[0]),
                                bucket_index[1].checked_add_signed(bucket_relative_index[1]),
                            ];

                            let mut offset = [0.0, 0.0];

                            let wrapped_neighbor_bucket_index = [0, 1].map(|i| {
                                if let Some(index_x) = neighbor_bucket_index[i] {
                                    if index_x >= self.buckets.size[i] {
                                        // result was greater than the width
                                        offset[i] = self.bucket_size * self.buckets.size[i] as Real;
                                        0
                                    } else {
                                        // result was within bounds
                                        index_x
                                    }
                                } else {
                                    // result was less than 0
                                    offset[i] = -self.bucket_size * self.buckets.size[i] as Real;
                                    self.buckets.size[i] - 1
                                }
                            });

                            for &other in &self.buckets[wrapped_neighbor_bucket_index] {
                                particle.update_impulse_with_particle(
                                    Particle {
                                        position: [
                                            other.position[0] + offset[0],
                                            other.position[1] + offset[1],
                                        ],
                                        ..other
                                    },
                                    &self.type_data,
                                    &self.params,
                                    maximum_distance_squared,
                                    impulse,
                                );
                            }
                        }
                    }
                });
            });

        // Move particles
        for (bucket, impulses) in self.buckets.data.iter_mut().zip(&mut self.impulses.data) {
            for (particle, &impulse) in bucket.iter_mut().zip(impulses.iter()) {
                particle.apply_velocity(impulse);
            }

            // Clear impulses to make cloning the simulation to the render thread faster
            impulses.clear();
        }
    }

    fn organize_particles(&mut self) {
        let disorganized_particles = (self.buckets.data.par_iter_mut())
            .enumerate()
            .map(|(i, bucket)| {
                let bucket_index = [i % self.buckets.size[0], i / self.buckets.size[0]];

                let min_corner = bucket_index.map(|x| x as Real * self.bucket_size);
                let max_corner = min_corner.map(|x| x + self.bucket_size);

                let mut disorganized_particles = Vec::new();

                let mut i = 0;
                while i < bucket.len() {
                    let particle = bucket[i];

                    if particle.position[0] >= min_corner[0]
                        && particle.position[0] <= max_corner[0]
                        && particle.position[1] >= min_corner[1]
                        && particle.position[1] <= max_corner[1]
                    {
                        i += 1;
                    } else {
                        disorganized_particles.push(bucket.swap_remove(i));
                    }
                }

                disorganized_particles
            })
            .collect_vec_list();

        for mut particle in disorganized_particles.into_iter().flatten().flatten() {
            let index = self.bucket_index_of_position(particle.position);

            if let Some(index) = index {
                self.buckets[index].push(particle);
            } else {
                self.metadata.num_particles -= 1;

                // Edge handling
                match self.params.edge_type {
                    EdgeType::Wrapping => {
                        let size = self.size();
                        particle.position[0] = particle.position[0].rem_euclid(size[0]);
                        particle.position[1] = particle.position[1].rem_euclid(size[1]);
                        self.insert_particle(particle);
                    }
                    EdgeType::Bouncing {
                        multiplier,
                        pushback,
                    } => {
                        let direction = particle.constrain_to_size(self.size());
                        if direction[0] != 0.0 {
                            particle.velocity[0] = (particle.velocity[0].abs() * multiplier
                                + pushback)
                                .clamp(Real::MIN, Real::MAX)
                                * direction[0];
                        }
                        if direction[1] != 0.0 {
                            particle.velocity[1] = (particle.velocity[1].abs() * multiplier
                                + pushback)
                                .clamp(Real::MIN, Real::MAX)
                                * direction[1];
                        }
                        self.insert_particle(particle);
                    }
                    EdgeType::Deleting => (),
                }
            }
        }
    }

    pub fn draw_at(&self, position: Vec2, camera: &Camera2D, draw_debug_graphics: bool) {
        // Draw border
        let radius = (0.005 / camera.zoom[1]).max(2.0);
        let offset = radius / 2.0 + PARTICLE_RADIUS as f32;
        let size = self.size();
        shapes::draw_rectangle_lines(
            position.x - offset,
            position.y - offset,
            size[0] as f32 + offset * 2.0,
            size[1] as f32 + offset * 2.0,
            radius,
            colors::GRAY,
        );

        let min_corner =
            camera.target - 1.0 / camera.zoom - (PARTICLE_RADIUS + self.bucket_size) as f32;
        let max_corner = camera.target + 1.0 / camera.zoom + PARTICLE_RADIUS as f32;

        // Collect particles
        let mut particles = Vec::new();
        for bucket_x in 0..self.buckets.size[0] {
            for bucket_y in 0..self.buckets.size[1] {
                let bucket_index = [bucket_x, bucket_y];
                let bucket_position = self.position_of_bucket(bucket_index);
                let bucket_position = vec2(bucket_position[0] as f32, bucket_position[1] as f32);
                // Cull rendering of offscreen buckets
                if bucket_position.x > max_corner.x
                    || bucket_position.y > max_corner.y
                    || bucket_position.x < min_corner.x
                    || bucket_position.y < min_corner.y
                {
                    continue;
                }

                let bucket = &self.buckets[bucket_index];

                // Draw chunk debug
                if draw_debug_graphics {
                    shapes::draw_rectangle_lines(
                        bucket_position.x,
                        bucket_position.y,
                        self.bucket_size as f32,
                        self.bucket_size as f32,
                        radius,
                        colors::DARKGRAY,
                    );
                }

                // Select particles for rendering
                for &particle in bucket {
                    particles.push(particle);
                }
            }
        }

        // Sort particles (counting sort):
        // counting step
        let mut indecies = vec![0; self.type_data.num_types()].into_boxed_slice();

        for particle in &particles {
            indecies[particle.typ] += 1;
        }

        // indexing step
        let mut sum = 0;
        for index in &mut indecies {
            let temp = sum;
            sum += *index;
            *index = temp;
        }

        // filling step
        let mut particles_sorted = vec![Particle::default(); particles.len()].into_boxed_slice();
        for particle in particles {
            particles_sorted[indecies[particle.typ]] = particle;
            indecies[particle.typ] += 1;
        }

        // Draw particles
        material::gl_use_material(&PARTICLE_MATERIAL);

        for particle in particles_sorted {
            let position = [
                particle.position[0] as f32 + position.x,
                particle.position[1] as f32 + position.y,
            ];
            let color = self.type_data.colors[particle.typ];

            shapes::draw_rectangle(
                position[0] - PARTICLE_RADIUS as f32,
                position[1] - PARTICLE_RADIUS as f32,
                PARTICLE_RADIUS as f32 * 2.0,
                PARTICLE_RADIUS as f32 * 2.0,
                color,
            );
        }

        material::gl_use_default_material();
    }

    pub fn randomize_particles_above_type(&mut self, num_types: usize) {
        let mut rng = rand::rng();

        for bucket in &mut self.buckets.data {
            for particle in bucket {
                if particle.typ >= num_types {
                    particle.typ = rng.random_range(0..num_types);
                }
            }
        }
    }

    pub fn size(&self) -> [Real; 2] {
        self.buckets.size.map(|x| x as Real * self.bucket_size)
    }

    pub fn size_vec2(&self) -> Vec2 {
        let size = self.size();
        size.map(|x| x as f32).into()
    }

    pub fn insert_particle(&mut self, particle: Particle) -> Option<()> {
        self.metadata.num_particles += 1;

        let index = self.bucket_index_of_position(particle.position)?;
        self.buckets[index].push(particle);
        Some(())
    }

    pub fn clear_particles(&mut self) {
        self.metadata.num_particles = 0;

        for bucket in &mut self.buckets.data {
            bucket.clear();
        }
    }

    pub fn add_random_particles(&mut self, count: usize) {
        let mut rng = rand::rng();
        let size = self.size();
        for _ in 0..count {
            let position = [
                rng.random_range(0.0..size[0]),
                rng.random_range(0.0..size[1]),
            ];
            let particle = Particle::new(
                position,
                [0.0, 0.0],
                rng.random_range(0..self.type_data.num_types()),
            );
            self.insert_particle(particle);
        }
    }

    fn position_of_bucket(&self, index: [usize; 2]) -> [Real; 2] {
        [
            index[0] as Real * self.bucket_size,
            index[1] as Real * self.bucket_size,
        ]
    }

    fn bucket_index_of_position(&self, position: [Real; 2]) -> Option<[usize; 2]> {
        if position[0] < 0.0 && position[1] < 0.0 {
            return None;
        }

        let bucket_position = position.map(|x| x.div_euclid(self.bucket_size));
        if bucket_position.iter().any(|x| *x < 0.0) {
            return None;
        }
        let bucket_position = bucket_position.map(|x| x as usize);
        self.buckets.check_index_bounds(bucket_position)?;
        Some(bucket_position)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Particle {
    pub position: [Real; 2],
    pub velocity: [Real; 2],
    pub typ: usize,
}

impl Particle {
    pub fn new(position: [Real; 2], velocity: [Real; 2], typ: usize) -> Self {
        Self {
            position,
            velocity,
            typ,
        }
    }

    pub fn apply_velocity(&mut self, impulse: [Real; 2]) {
        self.velocity[0] += impulse[0];
        self.velocity[1] += impulse[1];

        self.position[0] += self.velocity[0] / 2.0;
        self.position[1] += self.velocity[1] / 2.0;

        debug_assert!(self.velocity[0].is_finite(), "nonfinite x velocity");
        debug_assert!(self.velocity[1].is_finite(), "nonfinite y velocity");
        debug_assert!(self.position[0].is_finite(), "nonfinite x position");
        debug_assert!(self.position[1].is_finite(), "nonfinite y position");

        self.velocity = self.velocity.map(|x| x * 0.9);
    }

    pub fn update_impulse_with_particle(
        &self,
        other: Particle,
        type_data: &ParticleTypeData,
        params: &ParticleSimulationParams,
        max_distance_squared: Real,
        impulse: &mut [Real; 2],
    ) {
        let delta_position = if self.position == other.position {
            #[cold]
            #[inline(never)]
            fn random_vector() -> [Real; 2] {
                fn random() -> Real {
                    use macroquad::rand;

                    rand::gen_range(1e-7, 0.1) * if rand::rand() & 1 != 0 { 1.0 } else { -1.0 }
                }

                [random(), random()]
            }

            random_vector()
        } else {
            [
                other.position[0] - self.position[0],
                other.position[1] - self.position[1],
            ]
        };

        let distance_squared = delta_position[0].powi(2) + delta_position[1].powi(2);
        if distance_squared > max_distance_squared {
            return;
        }

        const MINIMUM_DISTANCE: Real = PARTICLE_RADIUS * 2.0;
        const MINIMUM_DISTANCE_SQUARED: Real = MINIMUM_DISTANCE * MINIMUM_DISTANCE;

        let attraction = if distance_squared > MINIMUM_DISTANCE_SQUARED {
            type_data.get_attraction(self.typ, other.typ) / distance_squared
        } else if params.prevent_particle_ejecting && distance_squared < 1.0 {
            PARTICLE_RADIUS / distance_squared.sqrt()
        } else {
            -PARTICLE_RADIUS / distance_squared
        };

        impulse[0] += delta_position[0] * attraction;
        impulse[1] += delta_position[1] * attraction;
    }

    pub fn constrain_to_size(&mut self, size: [Real; 2]) -> [Real; 2] {
        let mut direction = [0.0; 2];
        let size = [size[0] - 1e-5, size[1] - 1e-5];

        if self.position[0] < 0.0 {
            direction[0] = 1.0;
            self.position[0] = 0.0;
        } else if self.position[0] > size[0] {
            direction[0] = -1.0;
            self.position[0] = size[0];
        }

        if self.position[1] < 0.0 {
            direction[1] = 1.0;
            self.position[1] = 0.0;
        } else if self.position[1] > size[1] {
            direction[1] = -1.0;
            self.position[1] = size[1];
        }

        direction
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EdgeType {
    Wrapping,
    Bouncing { multiplier: Real, pushback: Real },
    Deleting,
}

#[derive(Clone, Debug)]
pub struct ParticleTypeData {
    pub base_attractions: Matrix<Real>,
    pub scaled_attractions: Matrix<Real>,
    attraction_scale: Real,
    pub colors: Box<[Color]>,
}

impl ParticleTypeData {
    pub fn new_random(num_types: usize, attraction_scale: Real) -> Self {
        let mut rng = rand::rng();
        ParticleTypeData::new_from_fn(num_types, attraction_scale, |_| {
            rng.random_range(-1.0..=1.0)
        })
    }

    #[must_use]
    pub fn resize(&self, num_types: usize) -> Self {
        let mut rng = rand::rng();
        ParticleTypeData::new_from_fn(num_types, self.attraction_scale(), |index| {
            self.base_attractions
                .get(index)
                .copied()
                .unwrap_or_else(|| rng.random_range(-1.0..=1.0))
        })
    }

    pub fn new_from_fn<F>(num_types: usize, attraction_scale: Real, function: F) -> Self
    where
        F: FnMut([usize; 2]) -> f64,
    {
        let base_attractions = Matrix::from_fn([num_types; 2], function);
        let scaled_attractions =
            ParticleTypeData::scale_attractions(&base_attractions, attraction_scale);
        let colors = ParticleTypeData::generate_colors(num_types);
        Self {
            base_attractions,
            scaled_attractions,
            attraction_scale,
            colors,
        }
    }

    pub fn rescale_attractions(&mut self, attraction_scale: Real) {
        self.attraction_scale = attraction_scale;

        self.scaled_attractions =
            ParticleTypeData::scale_attractions(&self.base_attractions, attraction_scale);
    }

    fn scale_attractions(base_attractions: &Matrix<Real>, scale: Real) -> Matrix<Real> {
        Matrix::from_fn(base_attractions.size, |index| {
            base_attractions[index] * scale
        })
    }

    fn generate_colors(num_types: usize) -> Box<[Color]> {
        (0..num_types)
            .map(|typ| typ as f32 / num_types as f32)
            .map(|hue| color::hsl_to_rgb(hue, 1.0, 0.5))
            .collect()
    }

    pub fn get_attraction(&self, source: usize, target: usize) -> Real {
        self.scaled_attractions[[source, target]]
    }

    pub fn num_types(&self) -> usize {
        self.base_attractions.size[0]
    }

    pub fn attraction_scale(&self) -> Real {
        self.attraction_scale
    }
}

pub static PARTICLE_MATERIAL: LazyLock<Material> = LazyLock::new(|| {
    material::load_material(
        ShaderSource::Glsl {
            vertex: CIRCLE_VERTEX_SHADER,
            fragment: CIRCLE_FRAGMENT_SHADER,
        },
        MaterialParams::default(),
    )
    .unwrap()
});

const CIRCLE_VERTEX_SHADER: &str = r"
    #version 100
    precision lowp float;

    attribute vec2 position;
    attribute vec2 texcoord;
    attribute vec4 color0;

    varying lowp vec2 uv;
    varying lowp vec4 color;

    uniform mat4 Projection;

    void main() {
        gl_Position = Projection * vec4(position, 0, 1);
        color = color0 / 255.0;
        uv = texcoord;
    }
";

const CIRCLE_FRAGMENT_SHADER: &str = r"
    #version 100
    precision lowp float;

    varying lowp vec2 uv;
    varying lowp vec4 color;

    void main() {
        vec2 offset = uv - vec2(0.5, 0.5);

        if (dot(offset, offset) > 0.25) {
            discard;
        }

        gl_FragColor = color;
    }
";
