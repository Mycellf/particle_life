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
    let bucket_size: Real = 100.0;
    let mut particle_simulation = ParticleSimulation::new(
        bucket_size,
        size,
        ParticleSimulationParams {
            edge_type: EdgeType::Wrapping,
            prevent_particle_ejecting: true,
        },
        ParticleSimulationMetadata::default(),
        ParticleTypeData::new_random(25, 5.0),
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
    simulation_from_size([30, 20], PARTICLE_DENSITY)
}

const PARTICLE_DENSITY: Real = 4e-3;

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
        let mut send_time = None;

        loop {
            let start_update = Instant::now();

            // Copy latest simulation to buffer
            if !simulation.metadata.is_active && simulation.metadata.steps == 0 {
                match user_input_rx.recv() {
                    Ok(simulation_buffer) => {
                        if simulation_buffer.metadata.update_id >= simulation.metadata.update_id {
                            simulation = simulation_buffer;
                        }
                    }
                    Err(_) => {
                        panic!("User input thread disconnected");
                    }
                }
            }

            loop {
                match user_input_rx.try_recv() {
                    Ok(simulation_buffer) => {
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
                .map(|tps_limit| start_update + Duration::from_secs_f64(1.0 / tps_limit as f64));

            if simulation.metadata.is_active || simulation.metadata.steps > 0 {
                simulation.step_simulation();

                simulation.metadata.total_time = total_time;
                simulation.metadata.tick_time = Some(start_update.elapsed());
                simulation.metadata.send_time = send_time;

                if simulation.metadata.is_active {
                    simulation.metadata.steps = 0;
                } else {
                    simulation.metadata.steps -= 1;
                }

                updated = true;
            }

            if updated {
                // Send simulation data to render thread
                let start_send = Instant::now();

                simulation_tx
                    .send(simulation.clone())
                    .expect("Error sending simulation to user input");

                send_time = Some(start_send.elapsed());
            }

            if simulation.metadata.is_active {
                if let Some(frame_end) = frame_end {
                    // Wait if there's time left
                    thread::sleep(frame_end - Instant::now());
                }

                total_time = Some(start_update.elapsed());
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

            style.visuals.striped = true;
            style.spacing.scroll = egui::style::ScrollStyle::solid();

            style.visuals.slider_trailing_fill = true;
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

    let mut attractions_input_buffer = None;
    let mut num_types_input_buffer = simulation_buffer.type_data.num_types();

    let mut attraction_scale_buffer = simulation_buffer.type_data.attraction_scale();
    let mut attraction_scale_input_buffer = attraction_scale_buffer;

    let mut time_of_last_update = Instant::now();

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
                    time_of_last_update = Instant::now();

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
                attraction_scale_input_buffer = attraction_scale_buffer;
                attractions_input_buffer = None;
                num_types_input_buffer = simulation_buffer.type_data.num_types();
            }

            window.show(egui, |ui| {
                const TPS_RANGE: RangeInclusive<usize> = 5..=240;
                const TPS_INPUT_RANGE: RangeInclusive<usize> =
                    *TPS_RANGE.start()..=*TPS_RANGE.end() + 1;

                // FPS/TPS Info
                ui.columns(3, |columns| {
                    let response = columns[0]
                        .label(format!("FPS: {}", time::get_fps()))
                        .on_hover_text("Frames per second");

                    let time_since_last_update = time_of_last_update.elapsed();
                    if time_since_last_update > Duration::from_millis(100) {
                        response.on_hover_text(format!(
                            "Time since last update: {}ms",
                            time_since_last_update.as_millis(),
                        ));
                    }

                    if simulation_buffer.metadata.is_active {
                        if let ParticleSimulationMetadata {
                            total_time: Some(total_time),
                            ..
                        } = simulation_buffer.metadata
                        {
                            let tps = (1.0 / total_time.as_secs_f64()).ceil();

                            columns[1]
                                .label(format!("TPS: {tps}"))
                                .on_hover_text("Ticks per Second");
                        }

                        if let ParticleSimulationMetadata {
                            tick_time: Some(tick_time),
                            ..
                        } = simulation_buffer.metadata
                        {
                            let mspt_label_result = columns[2]
                                .label(format!("MSPT: {}", tick_time.as_millis()))
                                .on_hover_text("Milliseconds per tick");

                            if let ParticleSimulationMetadata {
                                send_time: Some(send_time),
                                ..
                            } = simulation_buffer.metadata
                            {
                                mspt_label_result.on_hover_text(format!(
                                    "Send Time: {}ms",
                                    send_time.as_millis()
                                ));
                            }
                        }
                    } else {
                        columns[1].label("Paused").on_hover_text("Space to unpause");

                        let step_label =
                            if let Some(tick_time) = simulation_buffer.metadata.tick_time {
                                format!("Step ({}ms)", tick_time.as_millis())
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
                            .text("Simulation Speed")
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
                                    format!("{} TPS", number as usize)
                                } else {
                                    "unlimited".to_owned()
                                }
                            }),
                    )
                    .on_hover_text("Maximum ticks per second of the simulation")
                    .has_focus();

                if !slider_focused && !input::is_mouse_button_down(MouseButton::Left) {
                    let tps_limit_input = if tps_limit_input_buffer <= *TPS_RANGE.end() {
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

                // Particle editor
                ui.label(format!(
                    "Particles: {}",
                    simulation_buffer.metadata.num_particles
                ));

                ui.horizontal(|ui| {
                    if ui.button("Clear").clicked() {
                        simulation_buffer.clear_particles();
                        updated = true;
                    }

                    if ui.button("Fill to Density").clicked() {
                        let area = simulation_buffer.size()[0] * simulation_buffer.size()[1];
                        let particle_count = (area * PARTICLE_DENSITY) as usize;

                        simulation_buffer.add_random_particles(
                            particle_count.saturating_sub(simulation_buffer.metadata.num_particles),
                        );

                        updated = true;
                    }
                });

                ui.separator();

                // Particle type editor
                let result = ui
                    .horizontal(|ui| {
                        ui.label("Colors:");

                        ui.add(egui::DragValue::new(&mut num_types_input_buffer))
                    })
                    .inner;

                num_types_input_buffer = num_types_input_buffer.clamp(1, 250);

                if !result.has_focus()
                    && !result.is_pointer_button_down_on()
                    && num_types_input_buffer != simulation_buffer.type_data.num_types()
                {
                    simulation_buffer.type_data =
                        simulation_buffer.type_data.resize(num_types_input_buffer);

                    simulation_buffer.randomize_particles_above_type(num_types_input_buffer);

                    attractions_input_buffer = None;

                    updated = true;
                }

                let slider_focused = ui
                    .add(
                        egui::Slider::new(&mut attraction_scale_input_buffer, 0.0..=10.0)
                            .text("Force Scale")
                            .clamping(egui::SliderClamping::Never),
                    )
                    .on_hover_text("The scale applied to forces between particles")
                    .has_focus();

                attraction_scale_input_buffer =
                    attraction_scale_input_buffer.clamp(-1000.0, 1000.0);

                if !slider_focused
                    && !input::is_mouse_button_down(MouseButton::Left)
                    && attraction_scale_buffer != attraction_scale_input_buffer
                {
                    attraction_scale_buffer = attraction_scale_input_buffer;

                    simulation_buffer
                        .type_data
                        .rescale_attractions(attraction_scale_buffer);
                    updated = true;
                }

                egui::CollapsingHeader::new("Color Attractions").show(ui, |ui| {
                    egui::ScrollArea::both().max_height(250.0).show(ui, |ui| {
                        ui.spacing_mut().item_spacing = [5.0, 10.0].into();

                        egui::Grid::new("Color Attractions").show(ui, |ui| {
                            if attractions_input_buffer.is_none() {
                                attractions_input_buffer =
                                    Some(simulation_buffer.type_data.base_attractions.clone());
                            }

                            let attractions_input_buffer =
                                attractions_input_buffer.as_mut().unwrap();

                            ui.label("");
                            for (i, &color) in simulation_buffer.type_data.colors.iter().enumerate()
                            {
                                ui.add(egui::Label::new(rich_text_from_color(
                                    color,
                                    &(i + 1).to_string(),
                                )));
                            }

                            for (i, &color) in simulation_buffer.type_data.colors.iter().enumerate()
                            {
                                ui.end_row();

                                let row_label = rich_text_from_color(color, &(i + 1).to_string());

                                ui.add(egui::Label::new(row_label.clone()));

                                for j in 0..simulation_buffer.type_data.num_types() {
                                    let value =
                                        simulation_buffer.type_data.base_attractions[[i, j]];
                                    let value_input = &mut attractions_input_buffer[[i, j]];

                                    let result = ui.add(
                                        egui::DragValue::new(value_input)
                                            .speed(0.01)
                                            .max_decimals(3),
                                    );

                                    *value_input = value_input.clamp(-100.0, 100.0);

                                    if !result.has_focus()
                                        && !result.is_pointer_button_down_on()
                                        && value != *value_input
                                    {
                                        simulation_buffer.type_data.base_attractions[[i, j]] =
                                            *value_input;

                                        simulation_buffer.type_data.scaled_attractions[[i, j]] =
                                            *value_input
                                                * simulation_buffer.type_data.attraction_scale();

                                        updated = true;
                                    }

                                    // Only generate the tooltip if it could possibly be shown
                                    if result.hovered() {
                                        result.on_hover_ui(|ui| {
                                            ui.horizontal(|ui| {
                                                ui.add(egui::Label::new(row_label.clone()));

                                                ui.add(egui::Label::new(rich_text_from_color(
                                                    simulation_buffer.type_data.colors[j],
                                                    &(j + 1).to_string(),
                                                )));
                                            });
                                        });
                                    }
                                }
                            }
                        });
                    });

                    ui.add_space(10.0);

                    ui.horizontal(|ui| {
                        if ui.button("Randomize").clicked() {
                            simulation_buffer.type_data = ParticleTypeData::new_random(
                                simulation_buffer.type_data.num_types(),
                                simulation_buffer.type_data.attraction_scale(),
                            );
                            attractions_input_buffer = None;
                            updated = true;
                        }

                        if ui.button("Clear").clicked() {
                            simulation_buffer.type_data = ParticleTypeData::new_from_fn(
                                simulation_buffer.type_data.num_types(),
                                simulation_buffer.type_data.attraction_scale(),
                                |_| 0.0,
                            );
                            attractions_input_buffer = None;
                            updated = true;
                        }
                    });
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
                ui.weak("Press F1 to show/hide this window.");
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
                if egui_hovered { 1.0 } else { 1.1 },
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

fn rich_text_from_color(color: macroquad::color::Color, text: &str) -> egui::RichText {
    egui::RichText::new(text)
        .background_color(macroquad_color_to_egui(color))
        .color(
            if is_bright(color) {
                egui::Visuals::light()
            } else {
                egui::Visuals::dark()
            }
            .strong_text_color(),
        )
}

fn macroquad_color_to_egui(color: macroquad::color::Color) -> egui::Color32 {
    let bytes: [u8; 4] = color.into();

    egui::Color32::from_rgba_premultiplied(bytes[0], bytes[1], bytes[2], bytes[3])
}

fn is_bright(color: macroquad::color::Color) -> bool {
    let luminance_squared =
        0.299 * color.r.powi(2) + 0.587 * color.g.powi(2) + 0.144 * color.b.powi(2);

    luminance_squared > 0.5 * 0.5
}
