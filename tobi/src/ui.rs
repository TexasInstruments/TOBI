use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Gauge, List, ListItem, ListState, Paragraph, Wrap};

use crate::app::{App, ObstacleKind, ProxyConfigField, ProxyMode, RunnerObstacle, Screen};
use crate::custom_image::CustomImage;
use crate::device::{InstallTarget, format_bytes};
use crate::installer::RunMode;
use crate::manifest::ImageEntry;
use crate::memory::check_image_memory;
use crate::qr;

const TI_RED: Color = Color::Rgb(204, 0, 0);
const TI_TEAL: Color = Color::Rgb(0, 153, 160);
const TI_TEAL_DARK: Color = Color::Rgb(0, 96, 104);
const TI_WHITE: Color = Color::Rgb(245, 247, 250);
const TI_PROCESSORS_URL: &str = "https://www.ti.com/sitara";
const TI_SDK_DOCS_URL: &str = "https://texasinstruments.github.io/processor-sdk-doc/";

pub fn render(frame: &mut Frame, app: &App) {
    let root = frame.area();
    frame.render_widget(Clear, root);
    frame.render_widget(
        Block::default().style(Style::default().fg(TI_WHITE).bg(Color::Black)),
        root,
    );

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(12),
            Constraint::Length(3),
        ])
        .split(root);

    render_header(frame, app, chunks[0]);
    match app.screen() {
        Screen::Welcome => render_welcome(frame, app, chunks[1]),
        Screen::ImageSelect => render_image_select(frame, app, chunks[1]),
        Screen::CustomImageSelect => render_custom_image_select(frame, app, chunks[1]),
        Screen::TargetSelect => render_target_select(frame, app, chunks[1]),
        Screen::Confirm => render_confirm(frame, app, chunks[1]),
        Screen::Installing => render_installing(frame, app, chunks[1]),
        Screen::Complete => render_complete(frame, app, chunks[1]),
        Screen::Error => render_error(frame, app, chunks[1]),
        Screen::ProxyConfig => render_proxy_config(frame, app, chunks[1]),
    }
    render_footer(frame, app, chunks[2]);
    if app.has_warning() {
        render_warning(frame, app, root);
    }
}

fn render_header(frame: &mut Frame, app: &App, area: Rect) {
    let mode = match app.run_mode() {
        RunMode::Mock => "MOCK",
        RunMode::Live => "LIVE",
    };
    let title = Line::from(vec![
        Span::styled(
            "TOBI",
            Style::default()
                .fg(TI_WHITE)
                .bg(TI_RED)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            mode,
            Style::default().fg(if app.run_mode() == RunMode::Live {
                TI_RED
            } else {
                TI_TEAL
            }),
        ),
    ]);
    let board_line = Line::from(vec![
        Span::styled("Detected board: ", label_style()),
        Span::styled(
            app.board().name.clone(),
            Style::default().fg(TI_WHITE).add_modifier(Modifier::BOLD),
        ),
    ]);
    let status_line = Line::from(vec![
        Span::styled("Time: ", label_style()),
        Span::styled(
            app.system_status().time.clone(),
            Style::default().fg(TI_WHITE),
        ),
        Span::raw(" | "),
        Span::styled("IP: ", label_style()),
        Span::styled(
            app.system_status().ip.clone(),
            Style::default().fg(TI_WHITE),
        ),
    ]);
    let paragraph = Paragraph::new(vec![title, board_line, status_line])
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::default().fg(TI_RED)),
        )
        .alignment(Alignment::Center);
    frame.render_widget(paragraph, area);
}

fn render_welcome(frame: &mut Frame, app: &App, area: Rect) {
    let popup = centered_rect(92, 94, area);
    frame.render_widget(Clear, popup);
    let block = panel_block(" Welcome ");
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let split_for_qr = inner.width >= 86 && inner.height >= 19;
    let chunks = if split_for_qr {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
            .split(inner)
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(100)])
            .split(inner)
    };
    let compact = !split_for_qr && inner.height < 16;

    let mut lines = welcome_intro_lines(app);
    if compact {
        lines.extend([
            Line::from("Flash an OS image from the internet or local media."),
            Line::from("Plug in Ethernet and a keyboard to proceed."),
            Line::from("No network? Attach FAT32 USB media with a compatible image."),
        ]);
    } else {
        lines.extend([
            Line::from("Pick and flash a fresh OS image from the internet,"),
            Line::from("or install a compatible local image from attached media."),
            Line::from(""),
            Line::from("Please plug in an Ethernet cable and keyboard to proceed."),
            Line::from("TOBI can be used with an external display or from the serial console."),
            Line::from(""),
            Line::from("If no network is available, plug in a FAT32-formatted USB drive"),
            Line::from("containing compatible image files and flash that way."),
        ]);
    }
    lines.extend([
        Line::from(""),
        Line::from(vec![
            Span::styled("TI processors: ", label_style()),
            Span::raw(TI_PROCESSORS_URL),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Press Enter to continue.",
            Style::default().fg(TI_TEAL).add_modifier(Modifier::BOLD),
        )),
    ]);

    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false }),
        chunks[0],
    );

    if split_for_qr {
        render_qr_panel(
            frame,
            chunks[1],
            "TI processors",
            TI_PROCESSORS_URL,
            "Scan for TI processor docs.",
        );
    }
}

fn welcome_intro_lines(app: &App) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            "Welcome to TOBI",
            Style::default().fg(TI_RED).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "Texas Instruments Out of Box Installer",
            Style::default().fg(TI_WHITE).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("Board: ", label_style()),
            Span::styled(
                app.board().name.clone(),
                Style::default().fg(TI_WHITE).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Ethernet: ", label_style()),
            Span::raw(app.system_status().ethernet.clone()),
        ]),
        Line::from(""),
    ]
}

fn render_image_select(frame: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(area);

    let (items, selected_row) = image_list_items(app);

    let mut state = ListState::default();
    state.select(selected_row);
    let list = List::new(items)
        .block(panel_block(" OS Images "))
        .highlight_style(
            Style::default()
                .fg(TI_WHITE)
                .bg(TI_RED)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(">");
    frame.render_stateful_widget(list, chunks[0], &mut state);

    let details = app
        .selected_catalog_image()
        .map(|image| image_details(image, chunks[1]))
        .unwrap_or_else(|| vec![Line::from("No image selected.")]);
    frame.render_widget(
        Paragraph::new(details)
            .block(panel_block(" Details "))
            .wrap(Wrap { trim: false }),
        chunks[1],
    );
}

fn render_custom_image_select(frame: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(area);

    let items = if app.custom_images().is_empty() {
        vec![ListItem::new(vec![
            Line::from(Span::styled(
                "No custom images found",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from("Attach media and press R to rescan."),
        ])]
    } else {
        app.custom_images()
            .iter()
            .map(|image| {
                ListItem::new(vec![
                    Line::from(Span::styled(
                        image.name.clone(),
                        Style::default().add_modifier(Modifier::BOLD),
                    )),
                    Line::from(format!(
                        "{}  {}",
                        image.format.label(),
                        format_bytes(image.size_bytes)
                    )),
                ])
            })
            .collect::<Vec<_>>()
    };

    let mut state = ListState::default();
    if !app.custom_images().is_empty() {
        state.select(Some(app.custom_image_index()));
    }
    let list = List::new(items)
        .block(panel_block(" Custom Images "))
        .highlight_style(
            Style::default()
                .fg(TI_WHITE)
                .bg(TI_RED)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(">");
    frame.render_stateful_widget(list, chunks[0], &mut state);

    let details = app
        .selected_custom_image()
        .map(custom_image_details)
        .unwrap_or_else(|| {
            vec![
                Line::from(Span::styled(
                    "No supported image files found.",
                    Style::default().fg(TI_RED).add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from("Supported file types:"),
                Line::from(".wic.xz, .img.xz, .wic.zst, .img.zst"),
                Line::from(".wic.gz, .img.gz, .wic, .img, .raw, .bin"),
                Line::from(""),
                Line::from("Scan roots: /Volumes on macOS; /run/media, /media, and /mnt on Linux."),
            ]
        });

    frame.render_widget(
        Paragraph::new(details)
            .block(panel_block(" Custom Image Details "))
            .wrap(Wrap { trim: false }),
        chunks[1],
    );
}

fn render_target_select(frame: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(area);

    let (items, selected_row) = target_list_items(app);

    let mut state = ListState::default();
    state.select(selected_row);
    let list = List::new(items)
        .block(panel_block(" Target Media "))
        .highlight_style(
            Style::default()
                .fg(TI_WHITE)
                .bg(TI_RED)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(">");
    frame.render_stateful_widget(list, chunks[0], &mut state);

    let details = app
        .selected_target()
        .map(|target| target_details(app, target))
        .unwrap_or_else(|| vec![Line::from("No target selected.")]);

    frame.render_widget(
        Paragraph::new(details)
            .block(panel_block(" Target Details "))
            .wrap(Wrap { trim: false }),
        chunks[1],
    );
}

fn render_confirm(frame: &mut Frame, app: &App, area: Rect) {
    let selected_image = app.selected_image();
    let selected_target = app.selected_target();
    let image_name = selected_image
        .map(|image| image.name.as_str())
        .unwrap_or("unknown image");
    let target_path = selected_target
        .map(|target| target.path.display().to_string())
        .unwrap_or_else(|| "unknown target".to_string());
    let target_name = selected_target
        .map(|target| target.name.as_str())
        .unwrap_or("unknown target");

    let mut lines = vec![
        Line::from(Span::styled(
            "Ready to install",
            Style::default().fg(TI_RED).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("Image: ", label_style()),
            Span::raw(image_name),
        ]),
        Line::from(vec![
            Span::styled("Target: ", label_style()),
            Span::raw(target_name),
        ]),
        Line::from(vec![
            Span::styled("Path: ", label_style()),
            Span::raw(target_path),
        ]),
        Line::from(vec![
            Span::styled("RAM: ", label_style()),
            Span::raw(
                app.memory_check()
                    .map(|check| check.summary())
                    .unwrap_or_else(|| "unknown".to_string()),
            ),
        ]),
        Line::from(""),
        Line::from("TOBI streams images - the full image does not need to fit in RAM."),
        Line::from(""),
    ];

    if app.run_mode() == RunMode::Live {
        lines.push(Line::from(Span::styled(
            "LIVE MODE WILL OVERWRITE THE SELECTED TARGET.",
            Style::default().fg(TI_RED).add_modifier(Modifier::BOLD),
        )));
    } else {
        lines.push(Line::from(
            "Mock mode will simulate the install without writing storage.",
        ));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(
        "Press Enter to start. Press Backspace to choose a different target.",
    ));

    frame.render_widget(
        Paragraph::new(lines)
            .block(panel_block(" Confirm "))
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: false }),
        centered_rect(70, 55, area),
    );
}

fn render_proxy_config(frame: &mut Frame, app: &App, area: Rect) {
    let popup = centered_rect(78, 84, area);
    frame.render_widget(Clear, popup);
    let time_active = app.proxy_config_field() == ProxyConfigField::Time;
    let choice_active = app.proxy_config_field() == ProxyConfigField::ProxyChoice;
    let proxy_active = app.proxy_config_field() == ProxyConfigField::Proxy;

    let ti_style = if app.proxy_mode() == ProxyMode::Ti {
        Style::default().fg(TI_WHITE).bg(TI_RED)
    } else {
        Style::default().fg(TI_TEAL)
    };
    let manual_style = if app.proxy_mode() == ProxyMode::Manual {
        Style::default().fg(TI_WHITE).bg(TI_RED)
    } else {
        Style::default().fg(TI_TEAL)
    };
    let choice_label = if choice_active {
        "> Proxy mode"
    } else {
        "  Proxy mode"
    };
    let mut lines = vec![
        Line::from(Span::styled(
            "Set Time and Configure Proxy",
            Style::default().fg(TI_RED).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        input_line("UTC time", app.proxy_time_input(), time_active, 19),
        Line::from("Format: YYYY-MM-DD HH:MM:SS"),
        Line::from(""),
        Line::from(vec![
            Span::styled(choice_label, Style::default().fg(TI_TEAL)),
            Span::raw(": "),
            Span::styled("TI proxy", ti_style),
            Span::raw("   "),
            Span::styled("Manual", manual_style),
        ]),
        Line::from(format!("TI proxy: {}", crate::app::TI_PROXY_URL)),
        Line::from(""),
        input_line("Manual proxy", app.proxy_input(), proxy_active, 36),
        Line::from("Example: http://proxy.example.com:8080"),
        Line::from(""),
        Line::from(
            "Enter advances/retries. T selects TI. M selects manual. Up/Down switches fields.",
        ),
        Line::from("Esc keeps local images."),
    ];

    if let Some(warning) = app.warning() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Catalog load failed:",
            Style::default().fg(TI_RED).add_modifier(Modifier::BOLD),
        )));
        lines.extend(warning.lines().map(|line| Line::from(line.to_string())));
    }

    frame.render_widget(
        Paragraph::new(lines)
            .block(panel_block(" Proxy "))
            .wrap(Wrap { trim: false }),
        popup,
    );
}

fn render_installing(frame: &mut Frame, app: &App, area: Rect) {
    let popup = centered_rect(72, 48, area);
    frame.render_widget(Clear, popup);

    let inner = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(9),
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(popup);

    let image_name = app
        .selected_image()
        .map(|image| image.name.as_str())
        .unwrap_or("selected image");
    let target = app
        .selected_target()
        .map(|target| target.path.display().to_string())
        .unwrap_or_else(|| "selected target".to_string());
    let activity = activity_symbol(
        app.install_elapsed()
            .map(|elapsed| (elapsed.as_millis() / 200) as u64)
            .unwrap_or(0),
    );
    let details = vec![
        Line::from(Span::styled(
            "Installing",
            Style::default().fg(TI_RED).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("Image: ", label_style()),
            Span::raw(image_name),
        ]),
        Line::from(vec![
            Span::styled("Target: ", label_style()),
            Span::raw(target),
        ]),
        Line::from(vec![
            Span::styled("Phase: ", label_style()),
            Span::raw(app.status()),
        ]),
        Line::from(vec![
            Span::styled("Activity: ", label_style()),
            Span::styled(
                activity,
                Style::default().fg(TI_TEAL).add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled("Elapsed: ", label_style()),
            Span::raw(format_duration(app.install_elapsed().unwrap_or_default())),
            Span::raw("  "),
            Span::styled("Rate: ", label_style()),
            Span::raw(
                app.install_rate_bytes_per_second()
                    .map(|rate| format!("{}/s", format_bytes(Some(rate))))
                    .unwrap_or_else(|| "starting".to_string()),
            ),
        ]),
        Line::from(vec![
            Span::styled("Downloaded: ", label_style()),
            Span::raw(
                app.progress()
                    .and_then(|progress| {
                        progress
                            .source_current
                            .map(|current| (current, progress.source_total))
                    })
                    .map(|(current, total)| format_progress_bytes(current, total))
                    .unwrap_or_else(|| "starting".to_string()),
            ),
            Span::raw("  "),
            Span::styled("Written: ", label_style()),
            Span::raw(
                app.progress()
                    .map(|progress| format_progress_bytes(progress.current, progress.total))
                    .unwrap_or_else(|| "starting".to_string()),
            ),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(details).block(panel_block(" Install ")),
        inner[0],
    );

    let ratio = app
        .progress()
        .and_then(|progress| {
            progress
                .total
                .map(|total| progress.current as f64 / total.max(1) as f64)
                .or_else(|| {
                    progress.source_current.and_then(|current| {
                        progress
                            .source_total
                            .map(|total| current as f64 / total.max(1) as f64)
                    })
                })
        })
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    let label = app
        .progress()
        .map(|progress| match progress.total {
            Some(total) => {
                let percent = progress.current as f64 * 100.0 / total.max(1) as f64;
                format!(
                    "{activity}  {} / {}  ({percent:.2}%)",
                    format_bytes(Some(progress.current)),
                    format_bytes(Some(total))
                )
            }
            None => match (progress.source_current, progress.source_total) {
                (Some(current), Some(total)) => {
                    let percent = current as f64 * 100.0 / total.max(1) as f64;
                    format!(
                        "{activity}  downloaded {} / {}  ({percent:.2}%)",
                        format_bytes(Some(current)),
                        format_bytes(Some(total))
                    )
                }
                _ => format!(
                    "{activity}  written {}",
                    format_bytes(Some(progress.current))
                ),
            },
        })
        .unwrap_or_else(|| format!("{activity}  starting"));
    frame.render_widget(
        Gauge::default()
            .block(
                Block::default()
                    .title(
                        app.progress()
                            .map(|progress| format!(" {} ", progress.phase))
                            .unwrap_or_else(|| " Progress ".to_string()),
                    )
                    .borders(Borders::ALL),
            )
            .gauge_style(Style::default().fg(TI_TEAL))
            .ratio(ratio)
            .label(label),
        inner[1],
    );

    if app.lite_mode() {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    "TOBI-lite compact install display",
                    Style::default().fg(TI_TEAL).add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from("Memory snapshots are written to /run/tobi-memory.log."),
                Line::from("Serial handoff remains available from ttyS2 with SERIAL."),
            ])
            .block(panel_block(" Low Memory ")),
            inner[2],
        );
    } else {
        render_runner_game(frame, app, inner[2]);
    }

    frame.render_widget(
        Paragraph::new(if app.lite_mode() {
            "Do not power off the board during a live install."
        } else {
            "Do not power off the board during a live install. Space/Up jumps."
        })
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: false }),
        inner[3],
    );
}

fn render_runner_game(frame: &mut Frame, app: &App, area: Rect) {
    // TODO: Replace this quick ASCII runner with a more satisfying install-time mini-game.
    let available_rows = usize::from(area.height.saturating_sub(2));
    let game_height = available_rows.saturating_sub(1).clamp(3, 6);
    let width = usize::from(area.width.saturating_sub(2)).max(24);
    let grid = runner_game_grid(
        width,
        game_height,
        app.runner().runner_y(),
        app.runner().obstacles(),
    );

    let mut lines = vec![Line::from(vec![
        Span::styled("Score: ", label_style()),
        Span::raw(app.runner().score().to_string()),
        Span::raw("   "),
        Span::styled("Space/Up/W jump", Style::default().fg(TI_TEAL)),
    ])];
    lines.extend(grid.into_iter().map(Line::from));

    frame.render_widget(
        Paragraph::new(lines)
            .block(panel_block(" Waiting Game "))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn runner_game_grid(
    width: usize,
    game_height: usize,
    runner_y: i16,
    obstacles: &[RunnerObstacle],
) -> Vec<String> {
    let mut grid = vec![vec![' '; width]; game_height];
    let ground = game_height - 1;

    draw_runner_ground(&mut grid, ground);

    let runner_x = 5_i16;
    let max_runner_y = ground.saturating_sub(3).min(4);
    let runner_y = usize::try_from(runner_y.clamp(0, max_runner_y as i16)).unwrap_or(0);
    let runner_top = ground.saturating_sub(runner_y + 3);
    draw_sprite(&mut grid, runner_x, runner_top, &[" o ", "/|\\", "/ \\"]);
    draw_runner_obstacles(&mut grid, ground, obstacles);
    draw_runner_ground(&mut grid, ground);

    grid.into_iter()
        .map(|row| row.into_iter().collect::<String>())
        .collect()
}

fn draw_runner_obstacles(grid: &mut [Vec<char>], ground: usize, obstacles: &[RunnerObstacle]) {
    if ground < 2 {
        return;
    }

    for obstacle in obstacles {
        match obstacle.kind() {
            ObstacleKind::Cactus => {
                draw_sprite(grid, obstacle.x(), ground - 2, &[" \\|/ ", "  |  "]);
            }
            ObstacleKind::Rock => {
                draw_sprite(grid, obstacle.x(), ground - 2, &["/^^\\", "\\__/"]);
            }
        }
    }
}

fn draw_runner_ground(grid: &mut [Vec<char>], ground: usize) {
    for cell in &mut grid[ground] {
        *cell = '_';
    }
}

fn activity_symbol(tick: u64) -> &'static str {
    match tick % 4 {
        0 => "|",
        1 => "/",
        2 => "-",
        _ => "\\",
    }
}

fn format_duration(duration: std::time::Duration) -> String {
    let total = duration.as_secs();
    let minutes = total / 60;
    let seconds = total % 60;
    format!("{minutes:02}:{seconds:02}")
}

fn format_progress_bytes(current: u64, total: Option<u64>) -> String {
    match total {
        Some(total) => format!(
            "{} / {}",
            format_bytes(Some(current)),
            format_bytes(Some(total))
        ),
        None => format_bytes(Some(current)),
    }
}

fn draw_sprite(grid: &mut [Vec<char>], x: i16, y: usize, sprite: &[&str]) {
    for (row_offset, row) in sprite.iter().enumerate() {
        let row_index = y + row_offset;
        if row_index >= grid.len() {
            continue;
        }

        for (col_offset, ch) in row.chars().enumerate() {
            if ch == ' ' {
                continue;
            }
            let col = x + i16::try_from(col_offset).unwrap_or(0);
            if col < 0 {
                continue;
            }
            let col = usize::try_from(col).unwrap_or(0);
            if col < grid[row_index].len() {
                grid[row_index][col] = ch;
            }
        }
    }
}

fn render_complete(frame: &mut Frame, app: &App, area: Rect) {
    let prompt = if let Some(seconds) = app.complete_auto_reboot_seconds() {
        vec![
            Line::from(Span::styled(
                format!("Auto-rebooting in {seconds} seconds."),
                Style::default().fg(TI_TEAL).add_modifier(Modifier::BOLD),
            )),
            Line::from("Press Enter to reboot now."),
            Line::from("Press R to start over."),
        ]
    } else {
        vec![Line::from("Press Enter or R to start over.")]
    };
    render_result(frame, app, area, TI_TEAL, "Success", prompt);
}

fn render_error(frame: &mut Frame, app: &App, area: Rect) {
    render_result(
        frame,
        app,
        area,
        TI_RED,
        "Install Failed",
        vec![
            Line::from("Press Enter or R to start over."),
            Line::from("Press Q to quit."),
        ],
    );
}

fn render_result(
    frame: &mut Frame,
    app: &App,
    area: Rect,
    color: Color,
    title: &str,
    prompt: Vec<Line<'static>>,
) {
    let popup = centered_rect(72, 45, area);
    frame.render_widget(Clear, popup);
    let mut lines = vec![
        Line::from(Span::styled(
            title,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];
    lines.extend(
        app.status()
            .lines()
            .map(|line| Line::from(line.to_string())),
    );
    lines.push(Line::from(""));
    lines.extend(prompt);

    frame.render_widget(
        Paragraph::new(lines)
            .block(panel_block(" Result "))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false }),
        popup,
    );
}

fn render_footer(frame: &mut Frame, app: &App, area: Rect) {
    let keys = match app.screen() {
        Screen::Welcome => "Enter continue | Q quit".to_string(),
        Screen::Installing => "installing | Space/Up/W jump | Ctrl-C quit".to_string(),
        Screen::ImageSelect => {
            "Up/Down choose image | Enter continue | R refresh targets | Q quit".to_string()
        }
        Screen::CustomImageSelect => {
            "Up/Down choose image | Enter continue | Backspace back | R rescan media | Q quit"
                .to_string()
        }
        Screen::ProxyConfig => {
            "type field value | Enter next/retry | Up/Down field | Esc continue offline"
                .to_string()
        }
        Screen::TargetSelect => {
            "Up/Down choose target | Left/Right hide/show partitions | Enter continue | R refresh | Q quit".to_string()
        }
        Screen::Confirm => "Enter install | Backspace back | Q quit".to_string(),
        Screen::Complete if app.can_reboot_after_complete() => match app.complete_auto_reboot_seconds() {
            Some(seconds) => format!("auto reboot in {seconds}s | Enter reboot | R start over | Q quit"),
            None => "Enter reboot | R start over | Q quit".to_string(),
        },
        Screen::Complete => "Enter/R start over | Q quit".to_string(),
        Screen::Error => "Enter/R start over | Q quit".to_string(),
    };
    frame.render_widget(
        Paragraph::new(Line::from(keys))
            .style(Style::default().fg(TI_TEAL))
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(TI_RED)),
            )
            .alignment(Alignment::Center),
        area,
    );
}

fn target_list_items(app: &App) -> (Vec<ListItem<'static>>, Option<usize>) {
    let mut items = Vec::new();
    let mut selected_row = None;

    for (target_index, target) in app.devices().iter().enumerate() {
        if target_index == app.target_index() {
            selected_row = Some(items.len());
        }
        items.push(target_root_item(app, target));

        if app.is_target_expanded(&target.id) {
            for partition in &target.partitions {
                items.push(ListItem::new(vec![
                    Line::from(vec![
                        Span::raw("  - "),
                        Span::styled(
                            partition.name.clone(),
                            Style::default().add_modifier(Modifier::BOLD),
                        ),
                    ]),
                    Line::from(format!(
                        "    partition  {}  {}",
                        format_bytes(partition.size_bytes),
                        partition.path.display()
                    )),
                ]));
            }
        }
    }

    (items, selected_row)
}

fn target_root_item(app: &App, target: &InstallTarget) -> ListItem<'static> {
    let marker = if target.partitions.is_empty() {
        "   "
    } else if app.is_target_expanded(&target.id) {
        "[-]"
    } else {
        "[+]"
    };

    ListItem::new(vec![
        Line::from(vec![
            Span::raw(format!("{marker} ")),
            Span::styled(
                target.name.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(format!(
            "    {}  {}  {}",
            target.kind.label(),
            format_bytes(target.size_bytes),
            target.path.display()
        )),
    ])
}

fn target_details(app: &App, target: &InstallTarget) -> Vec<Line<'static>> {
    let expanded = app.is_target_expanded(&target.id);
    let partition_count = target.partitions.len();
    let partition_state = if partition_count == 0 {
        "none".to_string()
    } else if expanded {
        format!("{partition_count} shown")
    } else {
        format!("{partition_count} hidden")
    };

    let mut lines = vec![
        Line::from(vec![
            Span::styled("ID: ", label_style()),
            Span::raw(target.id.clone()),
        ]),
        Line::from(vec![
            Span::styled("Name: ", label_style()),
            Span::raw(target.name.clone()),
        ]),
        Line::from(vec![
            Span::styled("Path: ", label_style()),
            Span::raw(target.path.display().to_string()),
        ]),
        Line::from(vec![
            Span::styled("Type: ", label_style()),
            Span::raw(target.kind.label()),
        ]),
        Line::from(vec![
            Span::styled("Size: ", label_style()),
            Span::raw(format_bytes(target.size_bytes)),
        ]),
        Line::from(vec![
            Span::styled("Removable: ", label_style()),
            Span::raw(if target.removable { "yes" } else { "no" }),
        ]),
        Line::from(vec![
            Span::styled("Partitions: ", label_style()),
            Span::raw(partition_state),
        ]),
        Line::from(""),
        Line::from(
            target
                .warning
                .clone()
                .unwrap_or_else(|| "All data on this target will be destroyed.".to_string()),
        ),
    ];

    if partition_count > 0 {
        lines.push(Line::from(""));
        lines.push(Line::from(if expanded {
            "Left hides partitions. The selected install target remains the whole device."
        } else {
            "Right shows partitions. The selected install target remains the whole device."
        }));

        if expanded {
            lines.push(Line::from(""));
            for partition in &target.partitions {
                lines.push(Line::from(format!(
                    "{}  {}  {}",
                    partition.name,
                    format_bytes(partition.size_bytes),
                    partition.path.display()
                )));
            }
        }
    }

    lines
}

fn image_details(image: &ImageEntry, area: Rect) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(vec![
            Span::styled("Name: ", label_style()),
            Span::raw(image.name.clone()),
        ]),
        Line::from(vec![
            Span::styled("Category: ", label_style()),
            Span::raw(image.category_label().to_string()),
        ]),
        Line::from(vec![
            Span::styled("Version: ", label_style()),
            Span::raw(image.version.clone()),
        ]),
        Line::from(vec![
            Span::styled("Channel: ", label_style()),
            Span::raw(image.channel.clone()),
        ]),
        Line::from(vec![
            Span::styled("Release: ", label_style()),
            Span::raw(image.release_date.clone()),
        ]),
        Line::from(vec![
            Span::styled("Format: ", label_style()),
            Span::raw(image.format.label()),
        ]),
        Line::from(vec![
            Span::styled("Download: ", label_style()),
            Span::raw(format_bytes(image.image_download_size)),
        ]),
        Line::from(vec![
            Span::styled("Written: ", label_style()),
            Span::raw(format_bytes(image.extract_size)),
        ]),
        Line::from(vec![
            Span::styled("RAM: ", label_style()),
            Span::raw(check_image_memory(image).summary()),
        ]),
        Line::from(""),
        Line::from(
            "TOBI streams images directly to target media - the full image is not loaded into RAM.",
        ),
        Line::from(""),
        Line::from(image.description.clone()),
        Line::from(""),
        Line::from(vec![
            Span::styled("URL: ", label_style()),
            Span::raw(image.url.clone()),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("TI SDK docs: ", label_style()),
            Span::raw(TI_SDK_DOCS_URL),
        ]),
    ];

    append_qr_lines(&mut lines, area, TI_SDK_DOCS_URL);
    lines
}

fn render_warning(frame: &mut Frame, app: &App, area: Rect) {
    let popup = centered_rect(74, 52, area);
    frame.render_widget(Clear, popup);
    let warning = app.warning().unwrap_or_default();
    let mut lines = vec![
        Line::from(Span::styled(
            "Network Warning",
            Style::default().fg(TI_RED).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];
    lines.extend(warning.lines().map(|line| Line::from(line.to_string())));
    lines.push(Line::from(""));
    lines.push(Line::from("Enter: continue with local/custom images"));
    lines.push(Line::from(
        "P: set UTC time and proxy, then retry online catalog",
    ));

    frame.render_widget(
        Paragraph::new(lines)
            .block(panel_block(" Warning "))
            .wrap(Wrap { trim: false }),
        popup,
    );
}

fn custom_image_details(image: &CustomImage) -> Vec<Line<'static>> {
    vec![
        Line::from(vec![
            Span::styled("Name: ", label_style()),
            Span::raw(image.name.clone()),
        ]),
        Line::from(vec![
            Span::styled("Format: ", label_style()),
            Span::raw(image.format.label()),
        ]),
        Line::from(vec![
            Span::styled("Size: ", label_style()),
            Span::raw(format_bytes(image.size_bytes)),
        ]),
        Line::from(vec![
            Span::styled("Path: ", label_style()),
            Span::raw(image.path.display().to_string()),
        ]),
        Line::from(""),
        Line::from(
            "This image will be read from attached storage and flashed to the target media you choose next.",
        ),
    ]
}

fn image_list_items(app: &App) -> (Vec<ListItem<'static>>, Option<usize>) {
    let mut items = Vec::new();
    let mut selected_row = None;
    let mut last_category: Option<String> = None;

    for (image_index, image) in app.catalog().images.iter().enumerate() {
        let category = image.category_label().to_string();
        if last_category.as_deref() != Some(category.as_str()) {
            items.push(
                ListItem::new(Line::from(Span::styled(
                    format!(" {} ", category.to_uppercase()),
                    Style::default()
                        .fg(Color::Black)
                        .bg(TI_TEAL)
                        .add_modifier(Modifier::BOLD),
                )))
                .style(
                    Style::default()
                        .fg(Color::Black)
                        .bg(TI_TEAL)
                        .add_modifier(Modifier::BOLD),
                ),
            );
            last_category = Some(category);
        }

        if image_index == app.image_index() {
            selected_row = Some(items.len());
        }

        let mut name = vec![Span::styled(
            image.name.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        )];
        if image.recommended {
            name.push(Span::raw("  "));
            name.push(Span::styled(
                "Recommended",
                Style::default()
                    .fg(TI_WHITE)
                    .bg(TI_RED)
                    .add_modifier(Modifier::BOLD),
            ));
        }

        items.push(ListItem::new(vec![
            Line::from(name),
            Line::from(format!(
                "{}  {}  {}",
                image.version,
                image.channel,
                image.format.label()
            )),
        ]));
    }

    (items, selected_row)
}

fn label_style() -> Style {
    Style::default().fg(TI_TEAL).add_modifier(Modifier::BOLD)
}

fn field_marker(active: bool) -> Span<'static> {
    if active {
        Span::styled(
            "> ",
            Style::default().fg(TI_RED).add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw("  ")
    }
}

fn input_line(label: &'static str, value: &str, active: bool, min_width: usize) -> Line<'static> {
    let mut content = value.to_string();
    if content.len() < min_width {
        content.push_str(&" ".repeat(min_width - content.len()));
    }

    Line::from(vec![
        field_marker(active),
        Span::styled(format!("{label}: "), label_style()),
        Span::styled("[", input_border_style(active)),
        Span::styled(content, input_style(active)),
        Span::styled("]", input_border_style(active)),
    ])
}

fn input_border_style(active: bool) -> Style {
    let style = Style::default().fg(TI_TEAL).add_modifier(Modifier::BOLD);
    if active { style.fg(TI_RED) } else { style }
}

fn input_style(active: bool) -> Style {
    let style = Style::default()
        .fg(TI_WHITE)
        .bg(Color::Rgb(18, 31, 34))
        .add_modifier(Modifier::BOLD);
    if active {
        style.bg(TI_TEAL_DARK)
    } else {
        style
    }
}

fn panel_block(title: &'static str) -> Block<'static> {
    Block::default()
        .title(Span::styled(
            title,
            Style::default().fg(TI_WHITE).bg(TI_RED),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(TI_TEAL_DARK))
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

fn render_qr_panel(frame: &mut Frame, area: Rect, title: &str, url: &str, caption: &str) {
    let qr_lines = qr::render_qr(
        url,
        area.width.saturating_sub(2),
        area.height.saturating_sub(6),
    );
    let mut lines = vec![
        Line::from(Span::styled(
            title.to_string(),
            Style::default().fg(TI_TEAL).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];
    if let Some(qr_lines) = qr_lines {
        lines.extend(qr_lines.into_iter().map(|line| {
            Line::from(Span::styled(
                line,
                Style::default().fg(TI_WHITE).bg(Color::Black),
            ))
        }));
        lines.push(Line::from(""));
    }
    lines.push(Line::from(caption.to_string()));
    lines.push(Line::from(url.to_string()));

    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn append_qr_lines(lines: &mut Vec<Line<'static>>, area: Rect, url: &str) {
    if area.width < 48 || area.height < 30 {
        return;
    }
    let Some(qr_lines) = qr::render_qr(url, area.width.saturating_sub(4), area.height / 2) else {
        return;
    };

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Scan for TI Processor SDK docs:",
        Style::default().fg(TI_TEAL).add_modifier(Modifier::BOLD),
    )));
    lines.extend(qr_lines.into_iter().map(|line| {
        Line::from(Span::styled(
            line,
            Style::default().fg(TI_WHITE).bg(Color::Black),
        ))
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runner_jump_keeps_ground_and_obstacles_fixed() {
        let obstacles = [
            RunnerObstacle::new(18, ObstacleKind::Cactus),
            RunnerObstacle::new(34, ObstacleKind::Rock),
            RunnerObstacle::new(52, ObstacleKind::Cactus),
        ];
        let grounded = runner_game_grid(80, 6, 0, &obstacles);
        let jumping = runner_game_grid(80, 6, 2, &obstacles);

        let expected_ground = "_".repeat(80);
        assert_eq!(
            grounded.last().map(String::as_str),
            Some(expected_ground.as_str())
        );
        assert_eq!(
            jumping.last().map(String::as_str),
            Some(expected_ground.as_str())
        );

        for (row_index, (grounded_row, jumping_row)) in
            grounded.iter().zip(jumping.iter()).enumerate()
        {
            for (column_index, (grounded_cell, jumping_cell)) in
                grounded_row.chars().zip(jumping_row.chars()).enumerate()
            {
                if (5..=7).contains(&column_index) {
                    continue;
                }

                assert_eq!(
                    grounded_cell, jumping_cell,
                    "static scene moved at row {row_index}, column {column_index}"
                );
            }
        }
    }
}
