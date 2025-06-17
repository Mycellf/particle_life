use macroquad::{
    camera::{self, Camera2D},
    input::{self, KeyCode},
    math::{Vec2, vec2},
    time,
    window::{self, Conf},
};
use particle_simulation::{EdgeType, ParticleSimulation, ParticleSimulationParams, Real};
use std::{
    ops::RangeInclusive,
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
    simulation_from_size([30, 20], 4e-3)
}

#[macroquad::main(window_conf)]
async fn main() {
    let mut simulation = new_simulation();

    let mut simulation_camera = Camera2D::default();
    center_camera(&mut simulation_camera, simulation.size_vec2());

    let (simulation_tx, simulation_rx) = mpsc::channel::<ParticleSimulation>();
    let (user_input_tx, user_input_rx) = mpsc::channel::<ParticleSimulation>();

    let mut simulation_buffer = simulation.clone();

    let simulation_thread_builder = thread::Builder::new().name("simulation".to_owned());
    let simulation_thread = simulation_thread_builder.spawn(move || {
        let mut total_time = None;
        let mut tick_time = None;

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
                let updated;

                if simulation.metadata.is_active {
                    updated = true;
                    simulation.step_simulation();

                    simulation.metadata.total_time = total_time;
                    simulation.metadata.tick_time = tick_time;
                } else {
                    updated = !matches!(
                        simulation.metadata,
                        ParticleSimulationMetadata {
                            total_time: None,
                            tick_time: None,
                            ..
                        }
                    );

                    simulation.metadata.total_time = None;
                    simulation.metadata.tick_time = None;
                }

                if updated {
                    // Send simulation data to render thread
                    simulation_tx
                        .send(simulation.clone())
                        .expect("Error sending simulation to user input");
                }
            }

            tick_time = Some(Instant::now() - start);

            if let Some(frame_end) = frame_end {
                // Wait if there's time left
                thread::sleep(frame_end - Instant::now());
            }

            total_time = Some(Instant::now() - start);
        }
    });

    simulation_thread.unwrap();

    let mut debug = false;
    let mut info_window = true;

    let mut fullscreen = false;

    let mut tps_limit_buffer = simulation_buffer.metadata.tps_limit.unwrap();
    let mut tps_limit_input_buffer = tps_limit_buffer;

    // Rendering and user input
    loop {
        if input::is_key_pressed(KeyCode::F11) {
            fullscreen ^= true;
            window::set_fullscreen(fullscreen);
            input::show_mouse(!fullscreen);
        }

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
        let mut egui_focused = false;

        let mut window_toggled = false;

        if input::is_key_pressed(KeyCode::Escape)
            && !input::is_key_down(KeyCode::LeftShift)
            && !input::is_key_down(KeyCode::RightShift)
        {
            info_window ^= true;
            window_toggled = true;
        }

        egui_macroquad::ui(|egui| {
            egui.set_zoom_factor(macroquad::window::screen_dpi_scale());

            if !info_window || input::is_key_down(KeyCode::Escape) {
                tps_limit_input_buffer = tps_limit_buffer;
            }

            if !info_window {
                return;
            }

            let mut window = egui::Window::new("Info")
                .collapsible(false)
                .resizable(false);

            if window_toggled {
                window = window.current_pos([20.0, 20.0]);
            }

            window.show(egui, |ui| {
                const TPS_RANGE: RangeInclusive<usize> = 10..=240;
                const TPS_INPUT_RANGE: RangeInclusive<usize> =
                    *TPS_RANGE.start()..=*TPS_RANGE.end() + 1;

                // FPS/TPS Info
                ui.label(format!("FPS: {}", time::get_fps()));

                if let ParticleSimulationMetadata {
                    total_time: Some(total_time),
                    tick_time: Some(tick_time),
                    ..
                } = simulation_buffer.metadata
                {
                    let tps = (1.0 / total_time.as_secs_f64()).round();
                    let mspt = tick_time.as_millis();

                    ui.label(format!("TPS: {tps}"));
                    ui.label(format!("MSPT: {mspt}"));
                } else {
                    ui.label("TPS: --");
                    ui.label("MSPT: --");
                }

                // TPS Slider
                let slider_focused = ui
                    .add(
                        egui::Slider::new(&mut tps_limit_input_buffer, TPS_INPUT_RANGE)
                            .text("TPS Limit")
                            .custom_parser(|input| {
                                let input = input.trim();
                                if input == "unlimited" {
                                    Some(*TPS_INPUT_RANGE.end() as f64)
                                } else {
                                    input.parse().ok()
                                }
                            })
                            .custom_formatter(|number, _| {
                                if number <= 240.0 {
                                    (number as usize).to_string()
                                } else {
                                    "unlimited".to_owned()
                                }
                            }),
                    )
                    .has_focus();

                egui_focused |= slider_focused;

                if !slider_focused {
                    let tps_limit_input = if tps_limit_input_buffer <= *TPS_INPUT_RANGE.end() {
                        Some(tps_limit_input_buffer)
                    } else {
                        None
                    };

                    if tps_limit_input != simulation_buffer.metadata.tps_limit {
                        tps_limit_buffer = tps_limit_input_buffer;

                        simulation_buffer.metadata.tps_limit = tps_limit_input;
                        updated = true;
                    }
                }

                ui.separator();

                if ui.add(egui::Button::new("Reset")).clicked() {
                    let mut new_simulation = new_simulation();
                    new_simulation.metadata = simulation_buffer.metadata;
                    simulation_buffer = new_simulation;
                    updated = true;
                }

                // Window hiding instructions
                ui.add_enabled(
                    false,
                    egui::Label::new("Press escape to show/hide this window."),
                );
            });
        });

        if !egui_focused {
            update_camera_control(
                &mut simulation_camera,
                simulation_buffer.size_vec2(),
                1.0,
                1.1,
            );

            if input::is_key_pressed(KeyCode::Space) {
                simulation_buffer.metadata.is_active ^= true;
                updated = true;
            }
        }

        if updated {
            simulation_buffer.metadata.update_generation =
                (simulation_buffer.metadata.update_generation)
                    .checked_add(1)
                    .expect("update_generation overflowed (this should not happen for at least a billion years)");

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
            debug ^= true;
        }

        // Rendering
        simulation_buffer.draw_at(vec2(0.0, 0.0), &simulation_camera, debug);

        egui_macroquad::draw();

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
