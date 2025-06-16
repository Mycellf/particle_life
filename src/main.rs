use macroquad::{
    camera::{self, Camera2D},
    color::colors,
    input::{self, KeyCode},
    math::{Vec2, vec2},
    text, time,
    window::{self, Conf},
};
use particle_simulation::{EdgeType, ParticleSimulation, ParticleSimulationParams, Real};
use std::{
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use crate::particle_simulation::ParticleSimulationMetadata;

pub(crate) mod matrix;
pub(crate) mod particle_simulation;

fn window_conf() -> Conf {
    Conf {
        window_title: "Particle Life".to_string(),
        window_width: 900,
        window_height: 600,
        ..Default::default()
    }
}

fn simulation_from_size(size: [usize; 2], density: Real) -> ParticleSimulation {
    let bucket_size: Real = 100.0;
    let buckets = size[0] * size[1];
    let area = buckets as Real * bucket_size.powi(2);
    let particle_count = area * density;
    let particle_count = particle_count as usize;
    let mut particle_simulation = ParticleSimulation::new(
        bucket_size,
        size,
        ParticleSimulationParams {
            edge_type: EdgeType::Wrapping,
            // edge_type: EdgeType::Bouncing {
            //     multiplier: 1.0,
            //     pushback: 2.5,
            // },
            prevent_particle_ejecting: true,
        },
        ParticleSimulationMetadata::default(),
        50,
        5.0,
    );
    particle_simulation.add_random_particles(particle_count);
    particle_simulation
}

fn new_simulation() -> ParticleSimulation {
    simulation_from_size([60, 40], 4e-3)
}

#[macroquad::main(window_conf)]
async fn main() {
    let mut simulation = new_simulation();

    let mut simulation_camera = Camera2D::default();
    center_camera(&mut simulation_camera, simulation.size_vec2());

    let mut info_camera = Camera2D {
        zoom: [1.0, 2.0 / 800.0].into(),
        offset: [-1.0, 1.0].into(),
        ..Default::default()
    };

    let (simulation_tx, simulation_rx) = mpsc::channel::<ParticleSimulation>();
    let (user_input_tx, user_input_rx) = mpsc::channel::<ParticleSimulation>();

    let mut simulation_buffer = simulation.clone();

    let simulation_thread_builder = thread::Builder::new().name("simulation".to_owned());
    let simulation_thread = simulation_thread_builder.spawn(move || {
        let mut time = None;
        loop {
            let start = Instant::now();

            // Copy latest simulation to buffer
            loop {
                match user_input_rx.try_recv() {
                    Ok(simulation_buffer) => {
                        // simulation = simulation_buffer;
                        if simulation_buffer.metadata.update_generation
                            >= simulation.metadata.update_generation
                        {
                            simulation = simulation_buffer;
                        }
                    }
                    Err(error) => match error {
                        mpsc::TryRecvError::Empty => {
                            break;
                        }
                        mpsc::TryRecvError::Disconnected => {
                            panic!("User input thread disconnected");
                        }
                    },
                }
            }

            let frame_end = (simulation.metadata.tps_limit)
                .map(|tps_limit| start + Duration::from_secs_f64(1.0 / tps_limit as f64));

            {
                if simulation.metadata.is_active {
                    simulation.step_simulation();

                    simulation.metadata.tick_time = time;

                    // Send simulation data to render thread
                    simulation_tx
                        .send(simulation.clone())
                        .expect("Error sending simulation to user input");
                } else {
                    simulation.metadata.tick_time = None;
                }
            }

            if let Some(frame_end) = frame_end {
                // Wait if there's time left
                thread::sleep(frame_end - Instant::now());
            }

            time = Some(Instant::now() - start);
        }
    });

    simulation_thread.unwrap();

    let mut debug_level: u8 = 0;
    let mut fullscreen = false;

    // Rendering and user input
    loop {
        if input::is_key_pressed(KeyCode::F11) {
            fullscreen ^= true;
            window::set_fullscreen(fullscreen);
            input::show_mouse(!fullscreen);
        }

        // Camera control
        update_camera_control(
            &mut simulation_camera,
            simulation_buffer.size_vec2(),
            1.0,
            1.1,
        );

        // Setup camera
        update_camera_aspect_ratio(&mut simulation_camera);
        camera::set_camera(&simulation_camera);

        // Copy latest simulation to buffer
        loop {
            match simulation_rx.try_recv() {
                Ok(simulation) => {
                    if simulation.metadata.update_generation
                        >= simulation_buffer.metadata.update_generation
                    {
                        simulation_buffer = simulation;
                    }
                }
                Err(error) => match error {
                    mpsc::TryRecvError::Empty => {
                        break;
                    }
                    mpsc::TryRecvError::Disconnected => {
                        panic!("Simulation thread disconnected");
                    }
                },
            }
        }

        let mut updated = false;

        if input::is_key_pressed(KeyCode::Space) {
            simulation_buffer.metadata.is_active ^= true;
            updated = true;
        }

        if input::is_key_pressed(KeyCode::R) {
            let mut new_simulation = new_simulation();
            new_simulation.metadata = simulation_buffer.metadata;
            simulation_buffer = new_simulation;
            updated = true;
        }

        if input::is_key_pressed(KeyCode::L) {
            simulation_buffer.metadata.tps_limit = if simulation_buffer.metadata.tps_limit.is_none()
            {
                ParticleSimulationMetadata::default().tps_limit
            } else {
                None
            };
            updated = true;
        }

        if updated {
            simulation_buffer.metadata.update_generation =
                (simulation_buffer.metadata.update_generation)
                    .checked_add(1)
                    .expect("Too many updates (update_generation overflowed)");

            // Send the simulation buffer back
            user_input_tx
                .send(simulation_buffer.clone())
                .expect("Error sending user input to simulation");
        }

        // Center control
        if input::is_key_pressed(KeyCode::C) {
            center_camera(&mut simulation_camera, simulation_buffer.size_vec2());
        }

        // Debug view control
        if input::is_key_pressed(KeyCode::F3) {
            let selected_debug_level = if input::is_key_down(KeyCode::LeftShift) {
                2
            } else {
                1
            };

            if debug_level >= selected_debug_level {
                debug_level = 0;
            } else {
                debug_level = selected_debug_level;
            }
        }

        // Rendering
        simulation_buffer.draw_at(vec2(0.0, 0.0), &simulation_camera, debug_level > 1);

        // Draw debug
        if debug_level > 0 {
            update_camera_aspect_ratio(&mut info_camera);
            camera::set_camera(&info_camera);
            text::draw_text(
                &format!("FPS: {}", time::get_fps()),
                4.0,
                24.0,
                32.0,
                colors::WHITE,
            );

            if let Some(tick_time) = simulation_buffer.metadata.tick_time {
                let tps = (1.0 / tick_time.as_secs_f64()).round();
                text::draw_text(&format!("TPS: {tps}"), 4.0, 50.0, 32.0, colors::WHITE);

                let tps_message = if let Some(tps_limit) = simulation_buffer.metadata.tps_limit {
                    &format!("target: {tps_limit}")
                } else {
                    "unlimited"
                };

                text::draw_text(tps_message, 4.0, 76.0, 32.0, colors::WHITE);
            }
        }

        window::next_frame().await;
    }
}

fn update_camera_control(
    camera: &mut Camera2D,
    simulation_size: Vec2,
    pan_speed: f32,
    zoom_base: f32,
) {
    let motion = vec2(
        input::is_key_down(KeyCode::D) as u32 as f32 - input::is_key_down(KeyCode::A) as u32 as f32,
        input::is_key_down(KeyCode::S) as u32 as f32 - input::is_key_down(KeyCode::W) as u32 as f32,
    ) * (time::get_frame_time() * pan_speed / camera.zoom.y)
        * if input::is_key_down(KeyCode::LeftShift) {
            2.0
        } else {
            1.0
        };

    camera.target += motion;

    let scroll = zoom_base.powf(input::mouse_wheel().1.clamp(-1.0, 1.0));

    let max_zoom = 1.0 / 2.0 / particle_simulation::PARTICLE_RADIUS as f32;
    let min_zoom = 1.0 / simulation_size.y;

    camera.zoom.y *= scroll;
    camera.zoom.y = camera.zoom.y.clamp(min_zoom, max_zoom);
}

fn center_camera(camera: &mut Camera2D, simulation_size: Vec2) {
    camera.target = simulation_size / 2.0;
    camera.zoom = 2.0 / simulation_size;
}

fn update_camera_aspect_ratio(camera: &mut Camera2D) {
    camera.zoom.x = camera.zoom.y * window::screen_height() / window::screen_width();
}
