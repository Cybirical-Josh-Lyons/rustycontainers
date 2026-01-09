use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Axis, Block, Borders, Cell, Chart, Dataset, GraphType, Paragraph, Row, Scrollbar,
        ScrollbarOrientation, ScrollbarState, Table, Tabs, Wrap,
    },
    Frame,
};
use std::time::Instant;

use crate::app::AppState;
use crate::app::selected_container_is_running;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Containers,
    Images,
    Volumes,
    Logs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceTab {
    Stats,
    Graphs,
}

impl ResourceTab {
    pub fn index(self) -> usize {
        match self {
            ResourceTab::Stats => 0,
            ResourceTab::Graphs => 1,
        }
    }
}
impl Tab {
    pub fn next(self) -> Self {
        match self {
            Tab::Containers => Tab::Images,
            Tab::Images => Tab::Volumes,
            Tab::Volumes => Tab::Logs,
            Tab::Logs => Tab::Containers,
        }
    }
    pub fn prev(self) -> Self {
        match self {
            Tab::Containers => Tab::Logs,
            Tab::Images => Tab::Containers,
            Tab::Volumes => Tab::Images,
            Tab::Logs => Tab::Volumes,
        }
    }
}

pub fn draw(f: &mut Frame, app: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(8), // logo
            Constraint::Length(3), // tabs
            Constraint::Min(1),    // main
            Constraint::Length(3), // status
        ])
        .split(f.size());

    draw_logo(f, chunks[0]);
    draw_tabs(f, chunks[1], app);
    draw_main(f, chunks[2], app);
    draw_status(f, chunks[3], app);
}

fn draw_logo(f: &mut Frame, area: Rect) {
    let logo = [
        "██████╗ ██╗   ██╗███████╗████████╗██╗   ██╗    ██████╗ ███████╗███╗   ██╗████████╗ █████╗ ██╗███╗   ██╗███████╗██████╗ ███████╗",
        "██╔══██╗██║   ██║██╔════╝╚══██╔══╝╚██╗ ██╔╝    ██╔═══╝ ██   ██║████╗  ██║╚══██╔══╝██╔══██╗██║████╗  ██║██╔════╝██╔══██╗██╔════╝",
        "██████╔╝██║   ██║███████╗   ██║    ╚████╔╝     ██║     ██   ██║██╔██╗ ██║   ██║   ███████║██║██╔██╗ ██║█████╗  ██████╔╝███████╗",
        "██╔══██╗██║   ██║╚════██║   ██║     ╚██╔╝      ██║     ██   ██║██║╚██╗██║   ██║   ██╔══██║██║██║╚██╗██║██╔══╝  ██╔══██╗╚════██║",
        "██║  ██║╚██████╔╝███████║   ██║      ██║       ██████╗ ███████║██║ ╚████║   ██║   ██║  ██║██║██║ ╚████║███████╗██║  ██║███████║",
        "╚═╝  ╚═╝ ╚═════╝ ╚══════╝   ╚═╝      ╚═╝       ╚═════╝ ╚══════╝╚═╝  ╚═══╝   ╚═╝   ╚═╝  ╚═╝╚═╝╚═╝  ╚═══╝╚══════╝╚═╝  ╚═╝╚══════╝",
    ];

    let colors = [
        Color::LightRed,
        Color::Red,
        Color::Yellow,
        Color::Green,
        Color::Cyan,
        Color::LightBlue,
    ];

    let text = Text::from(
        logo.iter()
            .enumerate()
            .map(|(i, l)| Line::from(Span::styled(*l, Style::default().fg(colors[i]))))
            .collect::<Vec<_>>(),
    );

    let p = Paragraph::new(text)
        .block(Block::default().borders(Borders::ALL))
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

fn draw_tabs(f: &mut Frame, area: Rect, app: &AppState) {
    draw_tabs_widget(f, area, app);
}

fn draw_tabs_widget(f: &mut Frame, area: Rect, app: &AppState) {
    let tabs = [
        (Tab::Containers, "Containers"),
        (Tab::Images, "Images"),
        (Tab::Volumes, "Volumes"),
        (Tab::Logs, "Logs"),
    ];

    let selected_idx = match app.tab {
        Tab::Containers => 0,
        Tab::Images => 1,
        Tab::Volumes => 2,
        Tab::Logs => 3,
    };

    let titles: Vec<Line> = tabs
        .iter()
        .enumerate()
        .map(|(i, (_, name))| {
            if i == selected_idx {
                Line::from(vec![
                    Span::styled("▶ ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                    Span::styled(*name, Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                ])
            } else {
                Line::from(Span::raw(*name))
            }
        })
        .collect();

    let widget = Tabs::new(titles)
        .select(selected_idx)
        .block(ratatui::widgets::Block::default().borders(ratatui::widgets::Borders::ALL))
        .divider("|");

    f.render_widget(widget, area);
}


fn draw_main(f: &mut Frame, area: Rect, app: &AppState) {
    if app.shell_active {
        draw_shell(f, area, app);
        return;
    }

    match app.tab {
        Tab::Containers => draw_containers(f, area, app),
        Tab::Images => draw_images(f, area, app),
        Tab::Volumes => draw_volumes(f, area, app),
        Tab::Logs => draw_logs(f, area, app),
    }
}

fn draw_containers(f: &mut Frame, area: Rect, app: &AppState) {
    const SPIN: [&str; 10] = ["⠋","⠙","⠹","⠸","⠼","⠴","⠦","⠧","⠇","⠏"];

    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(68), Constraint::Percentage(32)])
        .split(area);

    let header = Row::new(vec!["", "Name", "Image", "State", "Status", "ID"])
        .style(Style::default().add_modifier(Modifier::BOLD));

    let rows = app.containers.iter().enumerate().map(|(i, c)| {
        // If this container is in the stopping set, show spinner + yellow state.
        let stopping_idx = app.stopping_containers.get(&c.id).copied();
        let spinner = stopping_idx.map(|idx| SPIN[idx % SPIN.len()]).unwrap_or(" ");

        // Base icon/style from actual state
        let (base_icon, base_style) = match c.state.as_str() {
            "running" => ("●", Style::default().fg(Color::Green)),
            "paused" => ("⏸", Style::default().fg(Color::Cyan)),
            "created" => ("●", Style::default().fg(Color::Yellow)),
            "exited" | "dead" => ("●", Style::default().fg(Color::Red)),
            _ => ("●", Style::default().fg(Color::Gray)),
        };

        // If stopping, override to yellow dot (but keep your real state text columns)
        let (icon, style) = if stopping_idx.is_some() {
            ("●", Style::default().fg(Color::Yellow))
        } else {
            (base_icon, base_style)
        };

        let id_short = if c.id.len() > 12 { &c.id[..12] } else { &c.id };

        let mut r = Row::new(vec![
            // Column 1 now shows spinner + icon
            Cell::from(format!("{spinner}{icon}")).style(style),
            Cell::from(c.name.clone()),
            Cell::from(c.image.clone()),
            Cell::from(c.state.clone()),
            Cell::from(c.status.clone()),
            Cell::from(id_short.to_string()),
        ]);

        if c.state != "running" {
            r = r.style(Style::default().fg(Color::DarkGray));
        }

        if i == app.selected_container {
            r = r.style(Style::default().add_modifier(Modifier::REVERSED));
        }

        r
    });

    let table = Table::new(
        rows,
        [
            Constraint::Length(2),  // enough for spinner+icon
            Constraint::Length(24),
            Constraint::Length(30),
            Constraint::Length(10),
            Constraint::Min(20),
            Constraint::Length(14),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .title("Containers (S=start, X=stop, Shift+S=start all, Shift+X=stop all, R=restart, F5=refresh)")
            .borders(Borders::ALL),
    );

    f.render_widget(table, panes[0]);
    draw_container_resources(f, panes[1], app);
}

fn draw_images(f: &mut Frame, area: Rect, app: &AppState) {
    let header = Row::new(vec!["Tags", "Size", "Created", "ID"])
        .style(Style::default().add_modifier(Modifier::BOLD));

    let rows = app.images.iter().enumerate().map(|(i, img)| {
        let id_short = if img.id.len() > 12 { &img.id[..12] } else { &img.id };
        let mut r = Row::new(vec![
            img.tags.clone(),
            img.size.clone(),
            img.created.clone(),
            id_short.to_string(),
        ]);
        if i == app.selected_image {
            r = r.style(Style::default().add_modifier(Modifier::REVERSED));
        }
        r
    });

    let table = Table::new(
        rows,
        [
            Constraint::Min(40),
            Constraint::Length(12),
            Constraint::Length(14),
            Constraint::Length(14),
        ],
    )
    .header(header)
    .block(Block::default().title("Images (Del=remove, F5=refresh)").borders(Borders::ALL));

    f.render_widget(table, area);
}

fn draw_volumes(f: &mut Frame, area: Rect, app: &AppState) {
    let header = Row::new(vec!["Name", "Driver", "Mountpoint"])
        .style(Style::default().add_modifier(Modifier::BOLD));

    let rows = app.volumes.iter().enumerate().map(|(i, v)| {
        let mut r = Row::new(vec![v.name.clone(), v.driver.clone(), v.mountpoint.clone()]);
        if i == app.selected_volume {
            r = r.style(Style::default().add_modifier(Modifier::REVERSED));
        }
        r
    });

    let table = Table::new(
        rows,
        [Constraint::Length(30), Constraint::Length(10), Constraint::Min(20)],
    )
    .header(header)
    .block(Block::default().title("Volumes (Del=remove, F5=refresh)").borders(Borders::ALL));

    f.render_widget(table, area);
}

fn draw_logs(f: &mut Frame, area: Rect, app: &AppState) {
    let lines = app
        .logs_lines
        .iter()
        .cloned()
        .map(Line::from)
        .collect::<Vec<_>>();
    let text = Text::from(lines);

    let block = Block::default()
        .title(app.logs_title.clone())
        .borders(Borders::ALL);
    let inner = block.inner(area);

    let viewport_height = inner.height as usize;
    let max_scroll = text.lines.len().saturating_sub(viewport_height);
    let scroll = if app.logs_follow {
        max_scroll
    } else {
        app.logs_scroll.min(max_scroll)
    }
    .min(u16::MAX as usize) as u16;

    let p = Paragraph::new(text)
        .scroll((scroll, 0))
        .wrap(Wrap { trim: false });

    f.render_widget(block, area);
    f.render_widget(p, inner);

    let content_len = app.logs_lines.len().max(1);
    let mut sb_state = ScrollbarState::new(content_len).position(scroll as usize);
    let sb = Scrollbar::default().orientation(ScrollbarOrientation::VerticalRight);
    f.render_stateful_widget(sb, area, &mut sb_state);
}

fn draw_shell(f: &mut Frame, area: Rect, app: &AppState) {
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        "Shell active (type exit or Ctrl+D to return)",
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
    )));

    let mut buffer = app.shell_lines.iter().cloned().collect::<Vec<_>>();
    if !app.shell_current.is_empty() {
        buffer.push(app.shell_current.clone());
    }

    let max_lines = area.height.saturating_sub(2) as usize;
    if buffer.len() > max_lines {
        buffer = buffer.split_off(buffer.len().saturating_sub(max_lines));
    }
    for line in &buffer {
        lines.push(Line::from(line.clone()));
    }

    let text = Text::from(lines);

    let p = Paragraph::new(text)
        .block(Block::default().title(app.shell_title.as_str()).borders(Borders::ALL))
        .wrap(Wrap { trim: false });

    f.render_widget(p, area);

    if app.shell_active {
        let cursor_y = area.y.saturating_add(1 + buffer.len() as u16);
        let cursor_x = area.x.saturating_add(1 + app.shell_current.chars().count() as u16);
        if cursor_y < area.y + area.height && cursor_x < area.x + area.width {
            f.set_cursor(cursor_x, cursor_y);
        }
    }
}

fn draw_container_resources(f: &mut Frame, area: Rect, app: &AppState) {
    let block = Block::default()
        .title("Resource Usage")
        .borders(Borders::ALL);
    let inner = block.inner(area);

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(inner);

    let titles = vec![Line::from("Stats"), Line::from("Graphs")];
    let tabs = Tabs::new(titles)
        .select(app.resource_tab.index())
        .divider("|");

    f.render_widget(block, area);
    f.render_widget(tabs, sections[0]);

    match app.resource_tab {
        ResourceTab::Stats => draw_container_stats(f, sections[1], app),
        ResourceTab::Graphs => draw_container_graphs(f, sections[1], app),
    }
}

fn draw_container_stats(f: &mut Frame, area: Rect, app: &AppState) {
    let mut lines = Vec::new();

    let row = app.containers.get(app.selected_container);
    if let Some(container) = row {
        lines.push(Line::from(vec![
            Span::styled("Name: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(container.name.as_str()),
        ]));
        lines.push(Line::from(vec![
            Span::styled("State: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(container.state.as_str()),
        ]));
        lines.push(Line::from(""));

        if let Some(err) = &app.container_stats_error {
            lines.push(Line::from(Span::styled(
                err.as_str(),
                Style::default().fg(Color::Yellow),
            )));
        } else if app.container_stats_loading {
            lines.push(Line::from(Span::styled(
                "Loading stats...",
                Style::default().fg(Color::Yellow),
            )));
        } else if let Some(stats) = &app.container_stats {
            lines.push(stat_line("CPU", &stats.cpu_percent));
            lines.push(stat_line(
                "Memory",
                &format!("{} ({})", stats.mem_usage, stats.mem_percent),
            ));
            lines.push(stat_line("Net I/O", &stats.net_io));
            lines.push(stat_line("Block I/O", &stats.block_io));
            lines.push(stat_line("PIDs", &stats.pids));
            lines.push(stat_line("GPU", &stats.gpu));
        } else {
            lines.push(Line::from("No stats available."));
        }
    } else {
        lines.push(Line::from("No containers."));
    }

    let text = Text::from(lines);
    let p = Paragraph::new(text).wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

fn stat_line(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{label}: "),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw(value.to_string()),
    ])
}

fn draw_container_graphs(f: &mut Frame, area: Rect, app: &AppState) {
    let now = Instant::now();
    let empty = std::collections::VecDeque::new();
    let history = app
        .containers
        .get(app.selected_container)
        .and_then(|row| app.container_stats_history.get(&row.id))
        .unwrap_or(&empty);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(17),
            Constraint::Percentage(17),
            Constraint::Percentage(17),
            Constraint::Percentage(17),
            Constraint::Percentage(16),
            Constraint::Percentage(16),
        ])
        .split(area);

    let (cpu_points, cpu_latest) = percent_points(history, now, |p| p.cpu_percent as f64);
    let (mem_points, mem_latest) = percent_points(history, now, |p| p.mem_percent as f64);
    let (net_points, net_latest) = rate_points(history, now, |p| p.net_total);
    let (block_points, block_latest) = rate_points(history, now, |p| p.block_total);
    let (pids_points, pids_latest) = scaled_points(history, now, |p| p.pids as f64);
    let (mem_usage_points, mem_usage_latest) =
        scaled_points(history, now, |p| p.mem_usage_bytes as f64);

    let cpu_title = value_title("CPU %", cpu_latest.map(|v| format!("{v:.1}%")));
    let mem_title = value_title("Memory %", mem_latest.map(|v| format!("{v:.1}%")));
    let net_title = value_title(
        "Net I/O",
        net_latest.map(|v| format!("{}/s", format_bytes(v as u64))),
    );
    let block_title = value_title(
        "Block I/O",
        block_latest.map(|v| format!("{}/s", format_bytes(v as u64))),
    );
    let pids_title = value_title("PIDs", pids_latest.map(|v| format!("{v:.0}")));
    let mem_usage_title = value_title(
        "Memory",
        mem_usage_latest.map(|v| format!("{}", format_bytes(v as u64))),
    );

    draw_line_chart(f, chunks[0], cpu_title, cpu_points, Color::LightRed);
    draw_line_chart(f, chunks[1], mem_title, mem_points, Color::Yellow);
    draw_line_chart(f, chunks[2], net_title, net_points, Color::Cyan);
    draw_line_chart(f, chunks[3], block_title, block_points, Color::LightBlue);
    draw_line_chart(f, chunks[4], pids_title, pids_points, Color::Green);
    draw_line_chart(f, chunks[5], mem_usage_title, mem_usage_points, Color::LightGreen);
}

fn draw_line_chart(
    f: &mut Frame,
    area: Rect,
    title: String,
    data: Vec<(f64, f64)>,
    color: Color,
) {
    let block = Block::default().title(title).borders(Borders::ALL);
    if data.is_empty() {
        let p = Paragraph::new("No data").block(block).wrap(Wrap { trim: false });
        f.render_widget(p, area);
        return;
    }

    let dataset = Dataset::default()
        .marker(ratatui::symbols::Marker::Braille)
        .graph_type(GraphType::Line)
        .style(Style::default().fg(color))
        .data(&data);

    let x_max = HISTORY_WINDOW_SECS;
    let chart = Chart::new(vec![dataset])
        .block(block)
        .x_axis(
            Axis::default()
                .bounds([0.0, x_max])
                .labels(vec![Span::raw("5m"), Span::raw("2.5m"), Span::raw("now")]),
        )
        .y_axis(
            Axis::default()
                .bounds([0.0, 100.0])
                .labels(vec![Span::raw("0"), Span::raw("50"), Span::raw("100")]),
        );

    f.render_widget(chart, area);
}

const HISTORY_WINDOW_SECS: f64 = 300.0;

fn percent_points(
    history: &std::collections::VecDeque<crate::app::StatsPoint>,
    now: Instant,
    value: fn(&crate::app::StatsPoint) -> f64,
) -> (Vec<(f64, f64)>, Option<f64>) {
    let mut points = Vec::new();
    let mut latest = None;
    for point in history {
        if let Some(x) = time_x(now, point.at) {
            let mut y = value(point);
            if y < 0.0 {
                y = 0.0;
            }
            if y > 100.0 {
                y = 100.0;
            }
            points.push((x, y));
            latest = Some(value(point));
        }
    }
    (points, latest)
}

fn scaled_points(
    history: &std::collections::VecDeque<crate::app::StatsPoint>,
    now: Instant,
    value: fn(&crate::app::StatsPoint) -> f64,
) -> (Vec<(f64, f64)>, Option<f64>) {
    let mut max: f64 = 0.0;
    for point in history {
        if time_x(now, point.at).is_some() {
            max = max.max(value(point));
        }
    }

    let mut points = Vec::new();
    let mut latest = None;
    for point in history {
        if let Some(x) = time_x(now, point.at) {
            let raw = value(point);
            let y = if max > 0.0 { (raw / max) * 100.0 } else { 0.0 };
            points.push((x, y.clamp(0.0, 100.0)));
            latest = Some(raw);
        }
    }
    (points, latest)
}

fn rate_points(
    history: &std::collections::VecDeque<crate::app::StatsPoint>,
    now: Instant,
    value: fn(&crate::app::StatsPoint) -> u64,
) -> (Vec<(f64, f64)>, Option<f64>) {
    let mut rates: Vec<(Instant, f64)> = Vec::new();
    let mut prev: Option<&crate::app::StatsPoint> = None;
    for point in history {
        let rate = if let Some(prev) = prev {
            let dt = point.at.saturating_duration_since(prev.at).as_secs_f64();
            if dt > 0.0 {
                let cur = value(point);
                let prev_val = value(prev);
                let delta = cur.saturating_sub(prev_val) as f64;
                delta / dt
            } else {
                0.0
            }
        } else {
            0.0
        };
        rates.push((point.at, rate));
        prev = Some(point);
    }

    let mut max: f64 = 0.0;
    for (at, rate) in &rates {
        if time_x(now, *at).is_some() {
            max = max.max(*rate);
        }
    }

    let mut points = Vec::new();
    let mut latest = None;
    for (at, rate) in rates {
        if let Some(x) = time_x(now, at) {
            let y = if max > 0.0 { (rate / max) * 100.0 } else { 0.0 };
            points.push((x, y.clamp(0.0, 100.0)));
            latest = Some(rate);
        }
    }
    (points, latest)
}

fn time_x(now: Instant, at: Instant) -> Option<f64> {
    let age = now.saturating_duration_since(at).as_secs_f64();
    if age > HISTORY_WINDOW_SECS {
        None
    } else {
        Some(HISTORY_WINDOW_SECS - age)
    }
}

fn value_title(label: &str, value: Option<String>) -> String {
    match value {
        Some(v) => format!("{label} {v}"),
        None => label.to_string(),
    }
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    if bytes == 0 {
        return "0 B".to_string();
    }

    let mut size = bytes as f64;
    let mut unit = 0usize;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

fn draw_status(f: &mut Frame, area: Rect, app: &AppState) {
    let (icon, color, label) = match app.engine_daemon_state {
        dockertui_core::daemon::DaemonState::Running => ("●", Color::Green, "Engine: Running"),
        dockertui_core::daemon::DaemonState::Starting => ("●", Color::Yellow, "Engine: Starting"),
        dockertui_core::daemon::DaemonState::Stopping => ("●", Color::Yellow, "Engine: Stopping"),
        dockertui_core::daemon::DaemonState::Stopped => ("●", Color::Red, "Engine: Stopped"),
        dockertui_core::daemon::DaemonState::Unknown => ("●", Color::Gray, "Engine: Unknown"),
    };

    let running = selected_container_is_running(app);

    let logs_span = if running {
        Span::raw("L logs")
    } else {
        Span::styled("L logs", Style::default().fg(Color::DarkGray))
    };

    let shell_span = if running {
        Span::raw("E shell")
    } else {
        Span::styled("E shell", Style::default().fg(Color::DarkGray))
    };

    let line = Line::from(vec![
        Span::styled(format!("{icon} "), Style::default().fg(color)),
        Span::raw(format!("{label}  |  ")),
        Span::raw(&app.status),
        Span::raw("  |  "),
        Span::raw("Tab/Shift+Tab | ↑/↓ | F5 refresh | G resource view | "),
        logs_span,
        Span::raw(" | "),
        shell_span,
        Span::raw(" | Ctrl+E start engine | Ctrl+X stop | Ctrl+R restart | Q quit"),
    ]);

    let p = Paragraph::new(line).block(Block::default().borders(Borders::ALL));
    f.render_widget(p, area);
}
