use macroquad::{
    camera::{self, Camera2D},
    color::colors,
    input::{self, KeyCode, MouseButton},
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

use crate::particle_simulation::{ParticleSimulationMetadata, ParticleTypeData};

pub(crate) mod matrix;
pub(crate) mod particle_simulation;

fn window_conf() -> Conf {
    Conf {
        window_title: "Particle Life".to_string(),
        ..Default::default()
    }
}

fn simulation_from_size(size: [usize; 2], density: Real) -> ParticleSimulation {
    let bucket_size: Real = 100.0;
    let mut particle_simulation = ParticleSimulation::new(
        bucket_size,
        size,
        ParticleSimulationParams {
            edge_type: EdgeType::Wrapping,
            prevent_particle_ejecting: true,
        },
        ParticleSimulationMetadata::default(),
        ParticleTypeData::new_random(50, 5.0),
    );
    if density > 0.0 {
        fill_simulation_with_particles(&mut particle_simulation, density);
    }
    particle_simulation
}

fn fill_simulation_with_particles(particle_simulation: &mut ParticleSimulation, density: Real) {
    particle_simulation.clear_particles();

    let area = particle_simulation.size()[0] * particle_simulation.size()[1];
    let particle_count = (area * density) as usize;

    particle_simulation.add_random_particles(particle_count);
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
                        if simulation_buffer.metadata.update_id >= simulation.metadata.update_id {
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

            tick_time = Some(start.elapsed());

            if let Some(frame_end) = frame_end {
                // Wait if there's time left
                thread::sleep(frame_end - Instant::now());
            }

            total_time = Some(start.elapsed());
        }
    });

    simulation_thread.unwrap();

    egui_macroquad::cfg(|egui| {
        let base_visuals = egui::Visuals::dark();

        egui.set_visuals(egui::Visuals {
            window_shadow: egui::Shadow {
                offset: [0, 0],
                spread: 15,
                ..base_visuals.window_shadow
            },
            ..base_visuals
        });
    });

    let mut debug = false;

    let mut info_window = true;

    let mut fullscreen = false;

    let mut tps_limit_buffer = simulation_buffer.metadata.tps_limit.unwrap();
    let mut tps_limit_input_buffer = tps_limit_buffer;

    let mut bouncing_value_buffer = EdgeType::Bouncing {
        multiplier: 1.0,
        pushback: 2.5,
    };
    let mut bouncing_value_input_buffer = bouncing_value_buffer;

    // Rendering and user input
    loop {
        if input::is_key_pressed(KeyCode::F11) {
            fullscreen ^= true;
            window::set_fullscreen(fullscreen);
        }

        // Setup camera
        update_camera_aspect_ratio(&mut simulation_camera);
        camera::set_camera(&simulation_camera);

        // Copy latest simulation to buffer
        loop {
            match simulation_rx.try_recv() {
                Ok(simulation) => {
                    if simulation.metadata.update_id >= simulation_buffer.metadata.update_id {
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
        let mut egui_hovered = false;

        let mut window_toggled = false;

        if input::is_key_pressed(KeyCode::F1) {
            info_window ^= true;
            window_toggled = true;
        }

        egui_macroquad::ui(|egui| {
            egui.set_zoom_factor(window::screen_dpi_scale());

            let info_window_copy = info_window;

            let mut window = egui::Window::new("Info")
                .open(&mut info_window)
                .title_bar(false)
                .resizable(false)
                .movable(false);

            if window_toggled && info_window_copy {
                window = window.current_pos([20.0, 20.0]);
            }

            if window_toggled {
                tps_limit_input_buffer = tps_limit_buffer;
            }

            window.show(egui, |ui| {
                const TPS_RANGE: RangeInclusive<usize> = 10..=240;
                const TPS_INPUT_RANGE: RangeInclusive<usize> =
                    *TPS_RANGE.start()..=*TPS_RANGE.end() + 1;

                // FPS/TPS Info
                let num_columns = if simulation_buffer.metadata.is_active {
                    3
                } else {
                    2
                };

                ui.columns(num_columns, |columns| {
                    let mut clicked = false;

                    clicked |= columns[0]
                        .label(format!("FPS: {}", time::get_fps()))
                        .on_hover_text("Frames per second")
                        .clicked();

                    if simulation_buffer.metadata.is_active {
                        if let ParticleSimulationMetadata {
                            total_time: Some(total_time),
                            ..
                        } = simulation_buffer.metadata
                        {
                            let tps = (1.0 / total_time.as_secs_f64()).round();

                            clicked |= columns[1]
                                .label(format!("TPS: {tps}"))
                                .on_hover_text("Ticks per Second")
                                .clicked();
                        }

                        if let ParticleSimulationMetadata {
                            tick_time: Some(tick_time),
                            ..
                        } = simulation_buffer.metadata
                        {
                            let mspt = tick_time.as_millis();

                            clicked |= columns[2]
                                .label(format!("MSPT: {mspt}"))
                                .on_hover_text("Milliseconds per tick")
                                .clicked();
                        }
                    } else {
                        clicked |= columns[1]
                            .colored_label(columns[1].visuals().warn_fg_color, "Paused")
                            .on_hover_text("Space to unpause")
                            .clicked()
                    }

                    if clicked {
                        simulation_buffer.metadata.is_active ^= true;
                        updated = true;
                    }
                });

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

                if !slider_focused && !input::is_mouse_button_down(MouseButton::Left) {
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

                // Buttons
                ui.horizontal(|ui| {
                    if ui.button("Clear").clicked() {
                        simulation_buffer.clear_particles();
                        updated = true;
                    }

                    if ui.button("Randomize Attractions").clicked() {
                        simulation_buffer.type_data = ParticleTypeData::new_random(50, 5.0);
                        updated = true;
                    }

                    if ui.button("Fill to Density").clicked() {
                        fill_simulation_with_particles(&mut simulation_buffer, 4e-3);
                        updated = true;
                    }
                });

                ui.separator();

                ui.label("Edge Type:");

                ui.horizontal(|ui| {
                    updated |= ui
                        .selectable_value(
                            &mut simulation_buffer.params.edge_type,
                            EdgeType::Wrapping,
                            "Wrapping",
                        )
                        .clicked();

                    updated |= ui
                        .selectable_value(
                            &mut simulation_buffer.params.edge_type,
                            EdgeType::Deleting,
                            "Deleting",
                        )
                        .clicked();

                    updated |= ui
                        .selectable_value(
                            &mut simulation_buffer.params.edge_type,
                            bouncing_value_buffer,
                            "Bouncing",
                        )
                        .clicked();
                });

                let enable_bouncing_editor = matches!(
                    simulation_buffer.params.edge_type,
                    EdgeType::Bouncing { .. }
                );

                let EdgeType::Bouncing {
                    multiplier,
                    pushback,
                } = &mut bouncing_value_input_buffer
                else {
                    unreachable!();
                };

                let mut slider_focused = false;

                slider_focused |= ui
                    .add_enabled(
                        enable_bouncing_editor,
                        egui::Slider::new(multiplier, 0.0..=1.0)
                            .clamping(egui::SliderClamping::Never)
                            .text("Bounce multiplier"),
                    )
                    .has_focus();

                if *multiplier < 0.0 {
                    *multiplier = 0.0;
                }

                slider_focused |= ui
                    .add_enabled(
                        enable_bouncing_editor,
                        egui::Slider::new(pushback, 0.0..=10.0)
                            .clamping(egui::SliderClamping::Never)
                            .text("Bounce pushback"),
                    )
                    .has_focus();

                egui_focused |= slider_focused;

                if enable_bouncing_editor
                    && !slider_focused
                    && !input::is_mouse_button_down(MouseButton::Left)
                    && bouncing_value_input_buffer != simulation_buffer.params.edge_type
                {
                    bouncing_value_buffer = bouncing_value_input_buffer;
                    simulation_buffer.params.edge_type = bouncing_value_input_buffer;
                    updated = true;
                }

                // Window hiding instructions
                ui.add_enabled(
                    false,
                    egui::Label::new("Press F1 to show/hide this window."),
                );
            });

            egui_hovered |= egui.is_pointer_over_area();

            egui.set_cursor_icon(if fullscreen && !info_window {
                egui::CursorIcon::None
            } else {
                egui::CursorIcon::Default
            });
        });

        if !egui_focused {
            update_camera_control(
                &mut simulation_camera,
                simulation_buffer.size_vec2(),
                1.0,
                1.1,
            );

            if input::is_key_pressed(KeyCode::C) || input::is_key_pressed(KeyCode::Home) {
                center_camera(&mut simulation_camera, simulation_buffer.size_vec2());
            }

            if input::is_key_pressed(KeyCode::Space) {
                simulation_buffer.metadata.is_active ^= true;
                updated = true;
            }
        }

        if updated {
            simulation_buffer.metadata.update_id = (simulation_buffer.metadata.update_id)
                .checked_add(1)
                .expect(
                    "update_id overflowed (this should not happen for at least a billion years)",
                );

            // Send the simulation buffer back
            user_input_tx
                .send(simulation_buffer.clone())
                .expect("Error sending user input to simulation");
        }

        // Debug view control
        if input::is_key_pressed(KeyCode::F3) {
            debug ^= true;
        }

        // Rendering
        // WORKAROUND: egui-macroquad prevents the screen from being cleared automatically when the
        // title bar of the window is disabled.
        window::clear_background(colors::BLACK);

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
        u32::from(input::is_key_down(KeyCode::D)) as f32
            - u32::from(input::is_key_down(KeyCode::A)) as f32,
        u32::from(input::is_key_down(KeyCode::S)) as f32
            - u32::from(input::is_key_down(KeyCode::W)) as f32,
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
