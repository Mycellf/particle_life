use std::sync::LazyLock;

use crate::matrix::Matrix;
use macroquad::{
    camera::Camera2D,
    color::{self, Color, colors},
    material::{self, Material},
    math::{Vec2, vec2},
    prelude::{MaterialParams, PipelineParams, ShaderSource},
    shapes,
};
use rand::{Rng, rngs::ThreadRng};
use rayon::iter::{IndexedParallelIterator, IntoParallelRefMutIterator, ParallelIterator};

pub const PARTICLE_RADIUS: f64 = 5.0;

#[rustfmt::skip]
pub const NEIGHBORS: [[isize; 2]; 8] = [
    [-1,  1], [ 0,  1], [ 1,  1],
    [-1,  0],           [ 1,  0],
    [-1, -1], [ 0, -1], [ 1, -1],
];

#[derive(Clone, Debug)]
pub struct ParticleSimulation {
    buckets: Matrix<Vec<Particle>>,
    impulses: Matrix<Vec<[f64; 2]>>,
    type_data: ParticleTypeData,
    bucket_size: f64,
    pub params: ParticleSimulationParams,
}

#[derive(Clone, Copy, Debug)]
pub struct ParticleSimulationParams {
    pub edge_type: EdgeType,
    pub prevent_particle_ejecting: bool,
}

impl ParticleSimulation {
    pub fn new(
        bucket_size: f64,
        buckets: [usize; 2],
        params: ParticleSimulationParams,
        num_types: usize,
        attraction_intensity: f64,
    ) -> Self {
        Self {
            buckets: Matrix::from_element(buckets, Vec::new()),
            impulses: Matrix::from_element(buckets, Vec::new()),
            type_data: ParticleTypeData::new_random(num_types, attraction_intensity),
            bucket_size,
            params,
        }
    }

    pub fn step_simulation(&mut self) {
        // (the unsafe blocks that cast a reference to a raw pointer and back are to skip the
        // borrow checker)

        // Cache the rng (is used when particles have 0 distance)

        // Update particle velocity
        (self.impulses.data.par_iter_mut())
            .enumerate()
            .for_each(|(i, impulses)| {
                let mut rng = rand::thread_rng();

                let bucket_index = [i % self.buckets.size[0], i / self.buckets.size[0]];
                let bucket = &self.buckets[bucket_index];

                impulses.clear();
                impulses.resize(bucket.len(), [0.0, 0.0]);

                // Update from own bucket
                for i in 1..bucket.len() {
                    let particle = &bucket[i];

                    // Iterate over each index up to but not including i
                    for j in 0..i {
                        particle.update_impulse_with_particle(
                            bucket[j],
                            &self.type_data,
                            &self.params,
                            self.bucket_size,
                            &mut rng,
                            &mut impulses[i],
                        );
                        bucket[j].update_impulse_with_particle(
                            *particle,
                            &self.type_data,
                            &self.params,
                            self.bucket_size,
                            &mut rng,
                            &mut impulses[j],
                        );
                    }
                }

                // Update from neighboring buckets
                for i in 0..bucket.len() {
                    let particle = &bucket[i];

                    for bucket_relative_index in NEIGHBORS {
                        let neighbor_bucket_index = {
                            let index = [
                                bucket_index[0].checked_add_signed(bucket_relative_index[0]),
                                bucket_index[1].checked_add_signed(bucket_relative_index[1]),
                            ];
                            if index.contains(&None) {
                                continue;
                            }
                            // SAFETY: Just checked if bucket_index contains None
                            index.map(|x| x.unwrap())
                        };

                        if let Some(neighbor_bucket) = self.buckets.get(neighbor_bucket_index) {
                            for &other in neighbor_bucket {
                                particle.update_impulse_with_particle(
                                    other,
                                    &self.type_data,
                                    &self.params,
                                    self.bucket_size,
                                    &mut rng,
                                    &mut impulses[i],
                                );
                            }
                        }
                    }
                }
            });

        // Move particles
        for (bucket, impulses) in self.buckets.data.iter_mut().zip(self.impulses.data.iter()) {
            for (particle, &impulse) in bucket.iter_mut().zip(impulses.iter()) {
                particle.apply_velocity(impulse);
            }
        }

        // Clear impulses to make cloning the simulation to the render thread faster
        for impulses in &mut self.impulses.data {
            impulses.clear();
        }

        // Organize particles
        for bucket_x in 0..self.buckets.size[0] {
            for bucket_y in 0..self.buckets.size[1] {
                let bucket_index = [bucket_x, bucket_y];
                // SAFETY: bucket is never accessed by index again
                let bucket = unsafe {
                    ((&mut self.buckets[bucket_index]) as *mut Vec<Particle>)
                        .as_mut()
                        .unwrap()
                };

                let mut i = 0;
                while i < bucket.len() {
                    let particle = &mut bucket[i];

                    let index = self.bucket_index_of_position(particle.position);
                    if index != Some(bucket_index) {
                        if index.is_some() {
                            self.insert_particle(*particle);
                        } else {
                            // Edge handling
                            match self.params.edge_type {
                                EdgeType::Wrapping => {
                                    let size = self.size();
                                    particle.position[0] = particle.position[0].rem_euclid(size[0]);
                                    particle.position[1] = particle.position[1].rem_euclid(size[1]);
                                    self.insert_particle(*particle);
                                }
                                EdgeType::Bouncing {
                                    multiplier,
                                    pushback,
                                } => {
                                    let direction = particle.constrain_to_size(self.size());
                                    if direction[0] != 0.0 {
                                        particle.velocity[0] =
                                            (particle.velocity[0].abs() * multiplier + pushback)
                                                * direction[0];
                                    }
                                    if direction[1] != 0.0 {
                                        particle.velocity[1] =
                                            (particle.velocity[1].abs() * multiplier + pushback)
                                                * direction[1];
                                    }
                                    self.insert_particle(*particle);
                                }
                                EdgeType::Deleting => (),
                            }
                        }
                        bucket.swap_remove(i);
                    } else {
                        i += 1;
                    }
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
                for particle in bucket {
                    particles.push(particle);
                }
            }
        }

        // Sort particles (counting sort):
        // counting step
        let mut indecies: Box<[usize]> = (0..self.type_data.num_types()).map(|_| 0).collect();
        for particle in particles.iter() {
            indecies[particle.typ] += 1;
        }

        // indexing step
        let mut sum = 0;
        for index in indecies.iter_mut() {
            let temp = sum;
            sum += *index;
            *index = temp;
        }

        // filling step
        let mut particles_sorted: Box<[_]> =
            (0..particles.len()).map(|_| Particle::default()).collect();
        for particle in particles {
            particles_sorted[indecies[particle.typ]] = *particle;
            indecies[particle.typ] += 1;
        }

        material::gl_use_material(&PARTICLE_MATERIAL);

        // Draw particles
        for &particle in particles_sorted.iter() {
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

    pub fn size(&self) -> [f64; 2] {
        self.buckets.size.map(|x| x as f64 * self.bucket_size)
    }

    pub fn size_vec2(&self) -> Vec2 {
        let size = self.size();
        vec2(size[0] as f32, size[1] as f32)
    }

    pub fn insert_particle(&mut self, particle: Particle) -> Option<()> {
        let index = self.bucket_index_of_position(particle.position)?;
        self.buckets.get_mut(index)?.push(particle);
        Some(())
    }

    pub fn add_random_particles(&mut self, count: usize) {
        let mut rng = rand::thread_rng();
        let size = self.size();
        for _ in 0..count {
            let position = [rng.gen_range(0.0..size[0]), rng.gen_range(0.0..size[1])];
            let particle = Particle::new(
                position,
                [0.0, 0.0],
                rng.gen_range(0..self.type_data.num_types()),
            );
            self.insert_particle(particle);
        }
    }

    fn position_of_bucket(&self, index: [usize; 2]) -> [f64; 2] {
        [
            index[0] as f64 * self.bucket_size,
            index[1] as f64 * self.bucket_size,
        ]
    }

    fn bucket_index_of_position(&self, position: [f64; 2]) -> Option<[usize; 2]> {
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
    pub position: [f64; 2],
    pub velocity: [f64; 2],
    pub typ: usize,
}

impl Particle {
    pub fn new(position: [f64; 2], velocity: [f64; 2], typ: usize) -> Self {
        Self {
            position,
            velocity,
            typ,
        }
    }

    pub fn apply_velocity(&mut self, impulse: [f64; 2]) {
        self.velocity[0] += impulse[0];
        self.velocity[1] += impulse[1];

        self.position[0] += self.velocity[0] / 2.0;
        self.position[1] += self.velocity[1] / 2.0;

        self.velocity = self.velocity.map(|x| x * 0.9);
    }

    pub fn update_impulse_with_particle(
        &self,
        other: Particle,
        type_data: &ParticleTypeData,
        params: &ParticleSimulationParams,
        max_distance: f64,
        rng: &mut ThreadRng,
        impulse: &mut [f64; 2],
    ) {
        #[cold]
        fn randomize_vector(delta_position: &mut [f64; 2], rng: &mut ThreadRng) {
            delta_position[0] = rng.gen_range(-0.1..=0.1);
            delta_position[1] = rng.gen_range(-0.1..=0.1);
        }

        let mut delta_position = [
            other.position[0] - self.position[0],
            other.position[1] - self.position[1],
        ];
        // Prevent division by 0 (this has an astronomically low chance to block for some time)
        while delta_position == [0.0, 0.0] {
            randomize_vector(&mut delta_position, rng);
        }

        let distance_squared = delta_position[0].powi(2) + delta_position[1].powi(2);
        if distance_squared > max_distance.powi(2) {
            return;
        }

        let attraction;
        if distance_squared > PARTICLE_RADIUS.powi(2) * 4.0 {
            attraction = type_data.get_attraction(self.typ, other.typ) / distance_squared;
        } else if params.prevent_particle_ejecting && distance_squared < 1.0 {
            attraction = PARTICLE_RADIUS / distance_squared.sqrt();
        } else {
            attraction = -PARTICLE_RADIUS / distance_squared;
        }

        impulse[0] += delta_position[0] * attraction;
        impulse[1] += delta_position[1] * attraction;
    }

    pub fn constrain_to_size(&mut self, size: [f64; 2]) -> [f64; 2] {
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

#[allow(unused)]
#[derive(Clone, Copy, Debug)]
pub enum EdgeType {
    Wrapping,
    Bouncing { multiplier: f64, pushback: f64 },
    Deleting,
}

#[derive(Clone, Debug)]
pub struct ParticleTypeData {
    types: Matrix<f64>,
    colors: Box<[Color]>,
}

impl ParticleTypeData {
    pub fn new_random(num_types: usize, attraction_intensity: f64) -> Self {
        let mut rng = rand::thread_rng();
        let types = Matrix::from_fn([num_types; 2], |_| {
            rng.gen_range(-attraction_intensity..=attraction_intensity)
        });
        let colors = (0..num_types)
            .map(|typ| typ as f32 / num_types as f32)
            .map(|hue| color::hsl_to_rgb(hue, 1.0, 0.5))
            .collect();
        Self { types, colors }
    }

    pub fn get_attraction(&self, source: usize, target: usize) -> f64 {
        self.types[[source, target]]
    }

    pub fn num_types(&self) -> usize {
        self.types.size[0]
    }
}

pub static PARTICLE_MATERIAL: LazyLock<Material> = LazyLock::new(|| {
    material::load_material(
        ShaderSource::Glsl {
            vertex: CIRCLE_VERTEX_SHADER,
            fragment: CIRCLE_FRAGMENT_SHADER,
        },
        MaterialParams {
            pipeline_params: PipelineParams::default(),
            ..Default::default()
        },
    )
    .unwrap()
});

const CIRCLE_VERTEX_SHADER: &str = r#"
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
"#;

const CIRCLE_FRAGMENT_SHADER: &str = r#"
    #version 100
    precision lowp float;

    varying lowp vec2 uv;
    varying lowp vec4 color;

    void main() {
        vec2 offset = uv - vec2(0.5, 0.5);

        if (dot(offset, offset) > 0.5 * 0.5) {
            discard;
        }

        gl_FragColor = color;
    }
"#;
