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
    sync::{Arc, Mutex, mpsc},
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
    simulation_from_size([75, 50], 2e-3)
}

#[macroquad::main(window_conf)]
async fn main() {
    let mut simulation = new_simulation();
    let thread_data = SimulationThreadData::default();

    let mut simulation_camera = Camera2D::default();
    center_camera(&mut simulation_camera, simulation.size_vec2());

    let mut info_camera = Camera2D {
        zoom: [1.0, 2.0 / 800.0].into(),
        offset: [-1.0, 1.0].into(),
        ..Default::default()
    };

    let (simulation_tx, simulation_rx) = mpsc::channel::<ParticleSimulation>();
    let thread_data_mutex = Arc::new(Mutex::new(thread_data));

    let mut simulation_buffer = simulation.clone();

    // Simulation
    let thread_data_reference = Arc::clone(&thread_data_mutex);
    let simulation_thread = thread::spawn(move || {
        let update_time = Duration::from_secs_f64(1.0 / 30.0);

        let mut time = None;
        loop {
            // Set the target frame end time
            let frame_end = Instant::now() + update_time;

            let start = Instant::now();
            let limit_tps;

            'update: {
                'simulate: {
                    {
                        let mut thread_data = thread_data_reference.lock().unwrap();

                        limit_tps = thread_data.limit_tps;

                        if thread_data.active {
                            thread_data.tick_time = time;
                        } else {
                            thread_data.tick_time = None;
                        }

                        if thread_data.reset {
                            simulation = new_simulation();
                            thread_data.reset = false;
                            break 'simulate;
                        }

                        if !thread_data.active {
                            break 'update;
                        }
                    }

                    // Update buffer
                    simulation.step_simulation();
                }

                // Send simulation data to render thread
                simulation_tx
                    .send(simulation.clone())
                    .expect("Error sending simulation");
            }

            if limit_tps {
                // Wait if there's time left
                thread::sleep(frame_end - Instant::now());
            }

            time = Some(Instant::now() - start);
        }
    });

    let mut debug_level: u8 = 0;
    let mut fullscreen = false;

    // Rendering and user input
    let thread_data_reference = Arc::clone(&thread_data_mutex);
    loop {
        // Panic if the simulation thread is no longer running
        if simulation_thread.is_finished() {
            panic!("Simulation thread panicked");
        }

        if input::is_key_pressed(KeyCode::F11) {
            fullscreen ^= true;
            window::set_fullscreen(fullscreen);
            input::show_mouse(!fullscreen);
        }

        // Copy latest simulation to buffer
        loop {
            match simulation_rx.try_recv() {
                Ok(simulation) => {
                    simulation_buffer = simulation;
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

        // Update thread_data
        let (tick_time, limit_tps) = {
            let mut thread_data = thread_data_reference.lock().unwrap();

            thread_data.active ^= input::is_key_pressed(KeyCode::Space);
            thread_data.reset |= input::is_key_pressed(KeyCode::R);

            thread_data.limit_tps ^= input::is_key_pressed(KeyCode::L) && thread_data.active;

            (thread_data.tick_time, thread_data.limit_tps)
        };

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

            if let Some(tick_time) = tick_time {
                let tps = (1.0 / tick_time.as_secs_f64()).round();
                text::draw_text(&format!("TPS: {tps}"), 4.0, 50.0, 32.0, colors::WHITE);

                if !limit_tps {
                    text::draw_text(&format!("unlimited"), 4.0, 76.0, 32.0, colors::WHITE);
                }
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

pub struct SimulationThreadData {
    pub active: bool,
    pub reset: bool,
    pub tick_time: Option<Duration>,
    pub limit_tps: bool,
}

impl Default for SimulationThreadData {
    fn default() -> Self {
        Self {
            active: true,
            reset: false,
            tick_time: None,
            limit_tps: true,
        }
    }
}
