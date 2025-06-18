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
        window_width: 900,
        window_height: 600,
        ..Default::default()
    }
}

fn simulation_from_size(size: [usize; 2], density: Real) -> ParticleSimulation {
    let bucket_size: Real = 200.0;
    let mut particle_simulation = ParticleSimulation::new(
        bucket_size,
        size,
        ParticleSimulationParams {
            edge_type: EdgeType::Wrapping,
            prevent_particle_ejecting: true,
        },
        ParticleSimulationMetadata::default(),
        ParticleTypeData::new_random(NUM_PARTICLE_TYPES, PARTICLE_ATTRACTION_SCALE),
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
    simulation_from_size([15, 10], PARTICLE_DENSITY)
}

const PARTICLE_DENSITY: Real = 2e-3;
const PARTICLE_ATTRACTION_SCALE: Real = 1.0;
const NUM_PARTICLE_TYPES: usize = 50;

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

            let mut updated = false;

            let frame_end = (simulation.metadata.tps_limit)
                .map(|tps_limit| start + Duration::from_secs_f64(1.0 / tps_limit as f64));

            if simulation.metadata.is_active || simulation.metadata.steps > 0 {
                simulation.step_simulation();
                simulation.metadata.total_time = total_time;
                simulation.metadata.tick_time = Some(start.elapsed());

                if simulation.metadata.is_active {
                    simulation.metadata.steps = 0;
                } else {
                    simulation.metadata.steps -= 1;
                }

                updated = true;
            }

            if updated {
                // Send simulation data to render thread
                simulation_tx
                    .send(simulation.clone())
                    .expect("Error sending simulation to user input");
            }

            if simulation.metadata.is_active {
                if let Some(frame_end) = frame_end {
                    // Wait if there's time left
                    thread::sleep(frame_end - Instant::now());
                }

                total_time = Some(start.elapsed());
            } else {
                total_time = None;
            }
        }
    });

    simulation_thread.unwrap();

    egui_macroquad::cfg(|egui| {
        egui.style_mut(|style| {
            style.interaction.selectable_labels = false;

            style.visuals.window_shadow.offset = [0, 0];
            style.visuals.window_shadow.spread = 15;
        });
    });

    let mut draw_bucket_edges = false;

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
                    // Reject updates from outdated simulations
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
        let mut reset_position = false;

        if input::is_key_pressed(KeyCode::F1) {
            let shift =
                input::is_key_down(KeyCode::LeftShift) || input::is_key_down(KeyCode::RightShift);

            if shift {
                reset_position = true;
            }

            if !shift || !info_window {
                info_window ^= true;
                window_toggled = true;
            }
        }

        egui_macroquad::ui(|egui| {
            const WINDOW_WIDTH: f32 = 350.0;
            const MIN_SCREEN_WIDTH: f32 = WINDOW_WIDTH + 14.0;

            let scale = (window::screen_width() / MIN_SCREEN_WIDTH).min(1.0);
            egui.set_zoom_factor(window::screen_dpi_scale() * scale);

            let mut window = egui::Window::new("Info")
                .open(&mut info_window)
                .title_bar(false)
                .resizable(false)
                .max_width(WINDOW_WIDTH)
                .min_width(WINDOW_WIDTH);

            if reset_position {
                window = window.current_pos([16.0, 16.0]);
            }

            if window_toggled {
                tps_limit_input_buffer = tps_limit_buffer;
                bouncing_value_input_buffer = bouncing_value_buffer;
            }

            window.show(egui, |ui| {
                const TPS_RANGE: RangeInclusive<usize> = 10..=240;
                const TPS_INPUT_RANGE: RangeInclusive<usize> =
                    *TPS_RANGE.start()..=*TPS_RANGE.end() + 1;

                // FPS/TPS Info
                ui.columns(3, |columns| {
                    columns[0]
                        .label(format!("FPS: {}", time::get_fps()))
                        .on_hover_text("Frames per second");

                    if simulation_buffer.metadata.is_active {
                        if let ParticleSimulationMetadata {
                            total_time: Some(total_time),
                            ..
                        } = simulation_buffer.metadata
                        {
                            let tps = (1.0 / total_time.as_secs_f64()).round();

                            columns[1]
                                .label(format!("TPS: {tps}"))
                                .on_hover_text("Ticks per Second");
                        }

                        if let ParticleSimulationMetadata {
                            tick_time: Some(tick_time),
                            ..
                        } = simulation_buffer.metadata
                        {
                            let mspt = tick_time.as_millis();

                            columns[2]
                                .label(format!("MSPT: {mspt}"))
                                .on_hover_text("Milliseconds per tick");
                        }
                    } else {
                        columns[1].label("Paused").on_hover_text("Space to unpause");

                        let step_label =
                            if let Some(tick_time) = simulation_buffer.metadata.tick_time {
                                let mspt = tick_time.as_millis();

                                format!("Step ({mspt}ms)")
                            } else {
                                "Step".to_owned()
                            };

                        if columns[2]
                            .add(egui::Label::new(step_label).sense(egui::Sense::click()))
                            .on_hover_text("Shift + Space to step")
                            .clicked()
                        {
                            simulation_buffer.metadata.steps += 1;
                            updated = true;
                        }
                    }
                });

                // TPS Slider
                let slider_focused = ui
                    .add(
                        egui::Slider::new(&mut tps_limit_input_buffer, TPS_INPUT_RANGE)
                            .text("TPS Limit")
                            .custom_parser(|input| {
                                let input = input.trim();
                                if ["unlimited", "infinity", "∞", "max"].contains(&input) {
                                    Some(*TPS_INPUT_RANGE.end() as f64)
                                } else {
                                    parse_number_or_default(input, 30.0)
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
                        simulation_buffer.type_data = ParticleTypeData::new_random(
                            NUM_PARTICLE_TYPES,
                            PARTICLE_ATTRACTION_SCALE,
                        );
                        updated = true;
                    }

                    if ui.button("Fill to Density").clicked() {
                        fill_simulation_with_particles(&mut simulation_buffer, PARTICLE_DENSITY);
                        updated = true;
                    }
                });

                ui.separator();

                ui.horizontal(|ui| {
                    ui.label("Edge Type:");

                    updated |= ui
                        .selectable_value(
                            &mut simulation_buffer.params.edge_type,
                            EdgeType::Wrapping,
                            "Wrapping",
                        )
                        .on_hover_text("Move particles touching the edge to the other side")
                        .clicked();

                    updated |= ui
                        .selectable_value(
                            &mut simulation_buffer.params.edge_type,
                            EdgeType::Deleting,
                            "Deleting",
                        )
                        .on_hover_text("Destroy particles touching the edge")
                        .clicked();

                    updated |= ui
                        .selectable_value(
                            &mut simulation_buffer.params.edge_type,
                            bouncing_value_buffer,
                            "Bouncing",
                        )
                        .on_hover_text("Push particles away from the edge")
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
                            .text("Bounce velocity multiplier")
                            .custom_parser(|input| parse_number_or_default(input, 1.0)),
                    )
                    .on_hover_text("Multiplied before additional velocity is applied")
                    .has_focus();

                if *multiplier < 0.0 {
                    *multiplier = 0.0;
                }

                slider_focused |= ui
                    .add_enabled(
                        enable_bouncing_editor,
                        egui::Slider::new(pushback, 0.0..=10.0)
                            .clamping(egui::SliderClamping::Never)
                            .text("Bounce additional velocity")
                            .custom_parser(|input| parse_number_or_default(input, 2.5)),
                    )
                    .on_hover_text("Added after the multiplier is applied")
                    .has_focus();

                if enable_bouncing_editor
                    && !slider_focused
                    && !input::is_mouse_button_down(MouseButton::Left)
                    && bouncing_value_input_buffer != simulation_buffer.params.edge_type
                {
                    bouncing_value_buffer = bouncing_value_input_buffer;
                    simulation_buffer.params.edge_type = bouncing_value_input_buffer;
                    updated = true;
                }

                ui.separator();

                // Window hiding instructions
                ui.add_enabled(
                    false,
                    egui::Label::new("Press F1 to show/hide this window."),
                );
            });

            egui_focused |= egui.wants_keyboard_input();
            egui_hovered |= egui.wants_pointer_input();

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
                if input::is_key_down(KeyCode::LeftShift) || input::is_key_down(KeyCode::RightShift)
                {
                    simulation_buffer.metadata.steps += 1;
                } else {
                    if simulation_buffer.metadata.is_active {
                        simulation_buffer.metadata.tick_time = None;
                    }
                    simulation_buffer.metadata.total_time = None;

                    simulation_buffer.metadata.is_active ^= true;
                }

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
            draw_bucket_edges ^= true;
        }

        // Rendering
        // WORKAROUND: egui-macroquad prevents the screen from being cleared automatically when the
        // title bar of the window is disabled.
        window::clear_background(colors::BLACK);

        simulation_buffer.draw_at(vec2(0.0, 0.0), &simulation_camera, draw_bucket_edges);

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

fn parse_number_or_default(input: &str, default: f64) -> Option<f64> {
    let input = input.trim();

    if input.is_empty() {
        Some(default)
    } else {
        input.parse().ok()
    }
}
