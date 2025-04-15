use macroquad::{
    camera::{self, Camera2D},
    color::colors,
    input::{self, KeyCode},
    math::{Vec2, vec2},
    text, time,
    window::{self, Conf},
};
use particle_simulation::{EdgeType, ParticleSimulation, ParticleSimulationParams};
use std::{
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

pub(crate) mod matrix;
pub(crate) mod particle_simulation;

fn window_conf() -> Conf {
    Conf {
        window_title: "Particle Life".to_string(),
        ..Default::default()
    }
}

fn simulation_from_size(size: [usize; 2], density: f64) -> ParticleSimulation {
    let bucket_size: f64 = 100.0;
    let buckets = size[0] * size[1];
    let area = buckets as f64 * bucket_size.powi(2);
    let particle_count = area * density;
    let particle_count = particle_count as usize;
    let mut particle_simulation = ParticleSimulation::new(
        bucket_size,
        size,
        ParticleSimulationParams {
            edge_type: EdgeType::Bouncing {
                multiplier: 1.0,
                pushback: 2.5,
            },
            prevent_particle_ejecting: true,
        },
        50,
        5.0,
    );
    particle_simulation.add_random_particles(particle_count);
    particle_simulation
}

fn new_simulation() -> ParticleSimulation {
    simulation_from_size([90, 60], 2e-3)
}

#[macroquad::main(window_conf)]
async fn main() {
    let simulation = new_simulation();
    let thread_data = SimulationThreadData::default();

    let mut camera = Camera2D::default();
    center_camera(&mut camera, simulation.size_vec2());

    let simulation_mutex = Arc::new(Mutex::new(simulation));
    let thread_data_mutex = Arc::new(Mutex::new(thread_data));

    // Simulation
    let simulation_reference = Arc::clone(&simulation_mutex);
    let thread_data_reference = Arc::clone(&thread_data_mutex);
    let simulation_thread = thread::spawn(move || {
        let update_time = Duration::from_secs_f64(1.0 / 30.0);

        let mut simulation_buffer = (*simulation_reference.lock().unwrap()).clone();

        let mut time = None;
        let mut frame_end;
        loop {
            // Set the target frame end time
            frame_end = Instant::now() + update_time;

            let start = Instant::now();

            'update: {
                'simulate: {
                    {
                        let mut thread_data = thread_data_reference.lock().unwrap();
                        if thread_data.active {
                            thread_data.tick_time = time;
                        } else {
                            thread_data.tick_time = None;
                        }

                        if thread_data.reset {
                            simulation_buffer = new_simulation();
                            thread_data.reset = false;
                            break 'simulate;
                        }

                        if !thread_data.active {
                            break 'update;
                        }
                    }

                    // Update buffer
                    simulation_buffer.step_simulation();
                }

                // Copy buffer to shared state
                {
                    let mut simulation_reference = simulation_reference.lock().unwrap();
                    *simulation_reference = simulation_buffer.clone();
                }
            }

            // Wait if there's time left
            thread::sleep(frame_end - Instant::now());

            time = Some(Instant::now() - start);
        }
    });

    let mut debug_mode: u8 = 0;
    let mut fullscreen = false;

    // Rendering and user input
    let simulation_reference = Arc::clone(&simulation_mutex);
    let thread_data_reference = Arc::clone(&thread_data_mutex);
    loop {
        // Panic if the simulation thread is no longer running
        if simulation_thread.is_finished() {
            panic!("Simulation thread panicked");
        }

        // Camera control
        update_camera_control(&mut camera, 1.0, 0.1);

        // Setup camera
        update_camera_aspect_ratio(&mut camera);
        camera::set_camera(&camera);

        // Copy simulation to buffer
        let simulation_buffer = {
            let simulation = simulation_reference.lock().unwrap();
            (*simulation).clone()
        };

        // Update thread_data
        let tick_time;
        {
            let mut thread_data = thread_data_reference.lock().unwrap();
            thread_data.active ^= input::is_key_pressed(KeyCode::Space);
            thread_data.reset |= input::is_key_pressed(KeyCode::R);
            tick_time = thread_data.tick_time;
        }

        if input::is_key_pressed(KeyCode::F11) {
            fullscreen ^= true;
            window::set_fullscreen(fullscreen);
            input::show_mouse(!fullscreen);
        }

        // Center control
        if input::is_key_pressed(KeyCode::C) {
            center_camera(&mut camera, simulation_buffer.size_vec2());
        }

        if input::is_key_pressed(KeyCode::F3) {
            let mode;
            if input::is_key_down(KeyCode::LeftShift) {
                mode = 2;
            } else {
                mode = 1;
            }

            if debug_mode >= mode {
                debug_mode = 0;
            } else {
                debug_mode = mode;
            }
        }

        // Rendering
        simulation_buffer.draw_at(vec2(0.0, 0.0), &camera, debug_mode > 1);

        // Draw debug
        if debug_mode > 0 {
            camera::set_default_camera();
            text::draw_text(
                &format!("FPS: {}", time::get_fps()),
                4.0,
                24.0,
                32.0,
                colors::WHITE,
            );

            if let Some(tick_time) = tick_time {
                let tps = (1.0 / tick_time.as_secs_f64()).round();
                text::draw_text(&format!("TPS: {tps}"), 4.0, 50.0, 32.0, colors::WHITE);
            }
        }

        window::next_frame().await;
    }
}

fn update_camera_control(camera: &mut Camera2D, pan_speed: f32, zoom_speed: f32) {
    let motion = vec2(
        input::is_key_down(KeyCode::D) as u32 as f32 - input::is_key_down(KeyCode::A) as u32 as f32,
        input::is_key_down(KeyCode::S) as u32 as f32 - input::is_key_down(KeyCode::W) as u32 as f32,
    ) * (time::get_frame_time() * pan_speed / camera.zoom.y)
        * if input::is_key_down(KeyCode::LeftShift) {
            2.0
        } else {
            1.0
        };

    let scroll = 1.0 + input::mouse_wheel().1 * zoom_speed;

    camera.target += motion;
    camera.zoom *= scroll;
}

fn center_camera(camera: &mut Camera2D, size: Vec2) {
    camera.target = size / 2.0;
    camera.zoom = 2.0 / size;
}

fn update_camera_aspect_ratio(camera: &mut Camera2D) {
    camera.zoom.x = camera.zoom.y * window::screen_height() / window::screen_width();
}

pub struct SimulationThreadData {
    pub active: bool,
    pub reset: bool,
    pub tick_time: Option<Duration>,
}

impl Default for SimulationThreadData {
    fn default() -> Self {
        Self {
            active: true,
            reset: false,
            tick_time: None,
        }
    }
}
