use anyhow::Result;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind, MouseEvent,
        MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use dockertui_core::engine::{Engine, LogsOptions};
use dockertui_core::models::{ContainerRow, ContainerStats, ImageRow, VolumeRow};
use futures::StreamExt;
use ratatui::{backend::CrosstermBackend, Terminal};
use ratatui::layout::{Constraint, Direction, Layout};
use portable_pty::PtySize;
use std::{collections::{HashMap, VecDeque}, io, sync::Arc, time::Duration, time::Instant};
use std::io::Read;
use tokio::sync::{mpsc, oneshot};

use crate::keymap::{Action, Keymap};
use crate::ui::{draw, ResourceTab, Tab};

enum UiMsg {
    LogLine(String),
    LogStopped(String),
    DaemonState(dockertui_core::daemon::DaemonState),
    ContainersStopping(Vec<String>), // ids
    ContainersStopped,
    ContainersActivityStart(Vec<String>), // ids
    ContainersActivityDone(String),
    Status(String),
    SetContainers(Vec<ContainerRow>),
    ContainerStats { id: String, stats: ContainerStats },
    ContainerStatsError { id: String, error: String },
    ShellOutput(String),
    ShellClosed,
}

pub struct AppState {
    pub tab: Tab,
    pub resource_tab: ResourceTab,
    pub status: String,

    pub containers: Vec<ContainerRow>,
    pub selected_container: usize,

    pub images: Vec<ImageRow>,
    pub selected_image: usize,

    pub volumes: Vec<VolumeRow>,
    pub selected_volume: usize,

    // Logs view
    pub logs_title: String,
    pub logs_lines: Vec<String>,
    pub logs_follow: bool,
    pub logs_scroll: usize,
    pub logs_viewport_height: u16,

    pub logs_running: bool,
    pub logs_cancel: Option<oneshot::Sender<()>>,

    pub engine_daemon_state: dockertui_core::daemon::DaemonState,
    pub stopping_containers: HashMap<String, usize>,

    pub container_stats: Option<ContainerStats>,
    pub container_stats_for: Option<String>,
    pub container_stats_loading: bool,
    pub container_stats_error: Option<String>,
    pub container_stats_updated_at: Option<Instant>,
    pub container_stats_requested_at: Option<Instant>,
    pub container_stats_history: HashMap<String, VecDeque<StatsPoint>>,

    pub shell_active: bool,
    pub shell_title: String,
    pub shell_lines: Vec<String>,
    pub shell_current: String,
    pub shell_session: Option<crate::shell::ShellSession>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            tab: Tab::Containers,
            resource_tab: ResourceTab::Stats,
            status: "Ready".into(),

            containers: vec![],
            selected_container: 0,

            images: vec![],
            selected_image: 0,

            volumes: vec![],
            selected_volume: 0,

            logs_title: "Logs".into(),
            logs_lines: vec![],
            logs_follow: true,
            logs_scroll: 0,
            logs_viewport_height: 0,

            logs_running: false,
            logs_cancel: None,

            engine_daemon_state: dockertui_core::daemon::DaemonState::Unknown,
            stopping_containers: HashMap::new(),
            container_stats: None,
            container_stats_for: None,
            container_stats_loading: false,
            container_stats_error: None,
            container_stats_updated_at: None,
            container_stats_requested_at: None,
            container_stats_history: HashMap::new(),
            shell_active: false,
            shell_title: String::new(),
            shell_lines: Vec::new(),
            shell_current: String::new(),
            shell_session: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct StatsPoint {
    pub at: Instant,
    pub cpu_percent: u64,
    pub mem_percent: u64,
    pub net_total: u64,
    pub block_total: u64,
    pub pids: u64,
    pub mem_usage_bytes: u64,
}

pub async fn run(engine: Arc<dyn Engine>) -> Result<()> {
    enable_raw_mode()?;
    let (ui_tx, mut ui_rx) = mpsc::unbounded_channel::<UiMsg>();
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let keymap = Keymap::default();
    let mut app = AppState::new();
    let mut last_daemon_poll = Instant::now();
    app.status = format!("Connected: {}", engine.name());
    if let Ok(s) = dockertui_core::daemon::status().await {
        app.engine_daemon_state = s; // Default state
    }

    // Initial data
    refresh_tab(engine.clone(), &mut app).await;
    request_selected_container_stats(engine.clone(), &mut app, ui_tx.clone());

    let tick_rate = Duration::from_millis(30);
    let mut last_auto_refresh = Instant::now();

    loop {
        if last_daemon_poll.elapsed() > Duration::from_millis(500) {
            let tx = ui_tx.clone();
            tokio::spawn(async move {
                if let Ok(s) = dockertui_core::daemon::status().await {
                    let _ = tx.send(UiMsg::DaemonState(s));
                }
            });
            last_daemon_poll = Instant::now();
            if app.tab == Tab::Containers
                && app.engine_daemon_state == dockertui_core::daemon::DaemonState::Running
                && last_auto_refresh.elapsed() > Duration::from_secs(1)
            {
                refresh_tab(engine.clone(), &mut app).await;
                request_selected_container_stats(engine.clone(), &mut app, ui_tx.clone());
                last_auto_refresh = Instant::now();
            }
        }

        // pump messages
        while let Ok(msg) = ui_rx.try_recv() {
            match msg {
                UiMsg::DaemonState(state) => {
                    app.engine_daemon_state = state;
                }
                UiMsg::ContainersStopping(ids) => {
                    for id in ids {
                        app.stopping_containers.insert(id, 0);
                    }
                    app.status = "Stopping running containers...".into();
                }
                UiMsg::ContainersStopped => {
                    app.stopping_containers.clear();
                    app.status = "Containers stopped. Stopping engine...".into();
                }
                UiMsg::ContainersActivityStart(ids) => {
                    for id in ids {
                        app.stopping_containers.insert(id, 0);
                    }
                }
                UiMsg::ContainersActivityDone(message) => {
                    app.stopping_containers.clear();
                    app.status = message;
                }
                UiMsg::Status(s) => {
                    app.status = s;
                }
                UiMsg::LogLine(line) => {
                    let cleaned = sanitize_log_line(&line);
                    app.logs_lines.push(cleaned);
                    if app.logs_lines.len() > 3000 {
                        let extra = app.logs_lines.len() - 3000;
                        app.logs_lines.drain(0..extra);
                        if !app.logs_follow {
                            if app.logs_scroll >= extra {
                                app.logs_scroll -= extra;
                            } else {
                                app.logs_scroll = 0;
                            }
                        }
                    }
                }
                UiMsg::LogStopped(reason) => {
                    app.logs_running = false;
                    app.logs_cancel = None;
                    app.status = reason;
                }
                UiMsg::SetContainers(list) => {
                    app.containers = list;
                    if app.selected_container >= app.containers.len() {
                        app.selected_container = app.containers.len().saturating_sub(1);
                    }
                    request_selected_container_stats(engine.clone(), &mut app, ui_tx.clone());
                }
                UiMsg::ContainerStats { id, stats } => {
                    if app.container_stats_for.as_deref() == Some(id.as_str()) {
                        app.container_stats = Some(stats);
                        app.container_stats_loading = false;
                        app.container_stats_error = None;
                        app.container_stats_updated_at = Some(Instant::now());
                        push_stats_history(&mut app, &id);
                    }
                }
                UiMsg::ContainerStatsError { id, error } => {
                    if app.container_stats_for.as_deref() == Some(id.as_str()) {
                        app.container_stats = None;
                        app.container_stats_loading = false;
                        app.container_stats_error = Some(error);
                        app.container_stats_updated_at = None;
                    }
                }
                UiMsg::ShellOutput(text) => {
                    push_shell_output(&mut app, &text);
                }
                UiMsg::ShellClosed => {
                    end_shell(engine.clone(), &mut app).await;
                }
            }
        }

        for v in app.stopping_containers.values_mut() {
            *v = (*v + 1) % 10;
        }
        
        terminal.autoresize()?;
        let size = terminal.size()?;
        let main_area = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(8),
                Constraint::Length(3),
                Constraint::Min(1),
                Constraint::Length(3),
            ])
            .split(size)[2];
        app.logs_viewport_height = main_area.height.saturating_sub(2);
        if app.shell_active {
            resize_shell(&mut app, &mut terminal)?;
        }
        terminal.draw(|f| draw(f, &app))?;

        if event::poll(tick_rate)? {
            match event::read()? {
                Event::Resize(_, _) => {
                    terminal.autoresize()?;
                    if app.shell_active {
                        resize_shell(&mut app, &mut terminal)?;
                    }
                }
                Event::Mouse(mouse) => {
                    if app.tab == Tab::Logs {
                        handle_logs_mouse_scroll(&mut app, mouse);
                    }
                    if app.tab == Tab::Containers {
                        handle_resource_tabs_click(&mut app, &terminal, mouse);
                    }
                }
                Event::Key(key) => {
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }
                    if app.shell_active {
                        if let Some(session) = app.shell_session.as_mut() {
                            let _ = write_shell_input(session, key);
                        }
                        continue;
                    }

                    if let Some(action) = keymap.map(key) {
                        if app.tab == Tab::Logs {
                            if matches!(action, Action::Quit) {
                                // stop logs
                                if let Some(cancel) = app.logs_cancel.take() {
                                    let _ = cancel.send(());
                                }
                                app.logs_running = false;
                                app.tab = Tab::Containers;
                                app.status = "Back to containers".into();
                                continue;
                            }
                        }

                        match action {
                            Action::Quit => break,
                            Action::Logs => {
                                if !selected_container_is_running(&app) {
                                    app.status = "Logs unavailable: container is not running".into();
                                } else {
                                    open_logs(engine.clone(), &mut app, ui_tx.clone()).await;
                                }
                            }
                            Action::NextTab => {
                                app.tab = app.tab.next();
                                refresh_tab(engine.clone(), &mut app).await;
                                request_selected_container_stats(engine.clone(), &mut app, ui_tx.clone());
                            }
                            Action::PrevTab => {
                                app.tab = app.tab.prev();
                                refresh_tab(engine.clone(), &mut app).await;
                                request_selected_container_stats(engine.clone(), &mut app, ui_tx.clone());
                            }
                            Action::Refresh => {
                                refresh_tab(engine.clone(), &mut app).await;
                                request_selected_container_stats(engine.clone(), &mut app, ui_tx.clone());
                            }

                            Action::Down => {
                                move_down(&mut app);
                                request_selected_container_stats(engine.clone(), &mut app, ui_tx.clone());
                            }
                            Action::Up => {
                                move_up(&mut app);
                                request_selected_container_stats(engine.clone(), &mut app, ui_tx.clone());
                            }

                            Action::Start => start_selected(engine.clone(), &mut app, ui_tx.clone()).await,
                            Action::Stop => stop_selected(engine.clone(), &mut app, ui_tx.clone()).await,
                            Action::Restart => restart_selected(engine.clone(), &mut app, ui_tx.clone()).await,
                            Action::StartAll => start_all_containers(engine.clone(), &mut app, ui_tx.clone()).await,
                            Action::StopAll => stop_all_containers(engine.clone(), &mut app, ui_tx.clone()).await,

                            Action::Shell => {
                                if !selected_container_is_running(&app) {
                                    app.status = "Shell unavailable: container is not running".into();
                                } else if let Err(e) = shell_into(engine.clone(), &mut app, &mut terminal, ui_tx.clone()).await {
                                    app.status = format!("Shell error: {e}");
                                }
                            }

                            Action::Remove => remove_selected(engine.clone(), &mut app).await,
                            Action::ToggleFollow => {
                                app.logs_follow = !app.logs_follow;
                                if !app.logs_follow {
                                    app.logs_scroll = max_logs_scroll(&app);
                                }
                                app.status = format!("Logs follow: {}", app.logs_follow);
                            }
                            Action::ToggleResourceTab => {
                                if app.tab == Tab::Containers {
                                    app.resource_tab = match app.resource_tab {
                                        ResourceTab::Stats => ResourceTab::Graphs,
                                        ResourceTab::Graphs => ResourceTab::Stats,
                                    };
                                    app.status = match app.resource_tab {
                                        ResourceTab::Stats => "Resource view: Stats".into(),
                                        ResourceTab::Graphs => "Resource view: Graphs".into(),
                                    };
                                }
                            }

                            Action::StartEngine => {
                                app.status = "Starting Docker Engine...".into();
                                tokio::spawn(async {
                                    let _ = dockertui_core::daemon::start_hard().await;
                                });
                            }
                            Action::StopEngine => {
                                app.status = "Stopping engine (draining containers)...".into();
                                app.engine_daemon_state = dockertui_core::daemon::DaemonState::Stopping;

                                let tx = ui_tx.clone();
                                let engine = engine.clone();

                                tokio::spawn(async move {
                                    // 1) list containers
                                    let rows = match engine.list_containers().await {
                                        Ok(Ok(rows)) => rows,
                                        Ok(Err(e)) => {
                                            let _ = tx.send(UiMsg::Status(format!("Failed to list containers: {e}")));
                                            return;
                                        }
                                        Err(e) => {
                                            let _ = tx.send(UiMsg::Status(format!("List containers task error: {e}")));
                                            return;
                                        }
                                    };

                                    let running_ids: Vec<String> = rows
                                        .into_iter()
                                        .filter(|c| c.state == "running")
                                        .map(|c| c.id)
                                        .collect();

                                    let _ = tx.send(UiMsg::ContainersStopping(running_ids.clone()));

                                    // 2) stop containers (sequential MVP; we can parallelize later)
                                    for id in running_ids {
                                        match engine.stop_container(id).await {
                                            Ok(Ok(())) => {}
                                            Ok(Err(e)) => {
                                                let _ = tx.send(UiMsg::Status(format!("Error stopping container: {e}")));
                                            }
                                            Err(e) => {
                                                let _ = tx.send(UiMsg::Status(format!("Stop container task error: {e}")));
                                            }
                                        }
                                    }

                                    let _ = tx.send(UiMsg::ContainersStopped);

                                    match engine.list_containers().await {
                                        Ok(Ok(list)) => { let _ = tx.send(UiMsg::SetContainers(list)); }
                                        Ok(Err(e)) => { let _ = tx.send(UiMsg::Status(format!("Refresh after stop failed: {e}"))); }
                                        Err(e) => { let _ = tx.send(UiMsg::Status(format!("Refresh task error: {e}"))); }
                                    }

                                    // 3) stop daemon
                                    if let Err(e) = dockertui_core::daemon::stop_hard().await {
                                        let _ = tx.send(UiMsg::Status(format!("Failed to stop engine: {e}")));
                                        return;
                                    }

                                    let _ = tx.send(UiMsg::Status("Engine stopped".into()));
                                });
                            }
                            Action::RestartEngine => {
                                app.status = "Restarting Docker Engine...".into();
                                tokio::spawn(async {
                                    let _ = dockertui_core::daemon::restart().await;
                                });
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // cleanup
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

fn handle_logs_mouse_scroll(app: &mut AppState, mouse: MouseEvent) {
    let delta = match mouse.kind {
        MouseEventKind::ScrollUp => -3,
        MouseEventKind::ScrollDown => 3,
        _ => 0,
    };

    if delta == 0 {
        return;
    }

    if app.logs_follow {
        app.logs_follow = false;
        app.status = "Logs follow: false".into();
        app.logs_scroll = max_logs_scroll(app);
    }

    if delta < 0 {
        app.logs_scroll = app.logs_scroll.saturating_sub((-delta) as usize);
    } else {
        let max_scroll = max_logs_scroll(app);
        app.logs_scroll = app.logs_scroll.saturating_add(delta as usize).min(max_scroll);
    }
}

fn handle_resource_tabs_click(
    app: &mut AppState,
    terminal: &Terminal<CrosstermBackend<io::Stdout>>,
    mouse: MouseEvent,
) {
    use crossterm::event::MouseButton;
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left)
        | MouseEventKind::Up(MouseButton::Left)
        | MouseEventKind::Drag(MouseButton::Left) => {}
        _ => return,
    }
    if app.shell_active || app.tab != Tab::Containers {
        return;
    }

    let size = match terminal.size() {
        Ok(size) => size,
        Err(_) => return,
    };
    let rect = match resource_tabs_rect(size) {
        Some(rect) => rect,
        None => return,
    };

    let x = mouse.column;
    let y = mouse.row;
    if x < rect.x || x >= rect.x + rect.width || y < rect.y || y >= rect.y + rect.height {
        return;
    }

    let mid = rect.x + rect.width / 2;
    app.resource_tab = if x < mid {
        ResourceTab::Stats
    } else {
        ResourceTab::Graphs
    };
}

fn resource_tabs_rect(area: ratatui::layout::Rect) -> Option<ratatui::layout::Rect> {
    if area.width < 10 || area.height < 10 {
        return None;
    }
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(8),
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(3),
        ])
        .split(area);
    let main = chunks[2];
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(68), Constraint::Percentage(32)])
        .split(main);
    let resource_area = panes[1];
    if resource_area.width < 3 || resource_area.height < 3 {
        return None;
    }

    let inner = ratatui::layout::Rect {
        x: resource_area.x + 1,
        y: resource_area.y + 1,
        width: resource_area.width.saturating_sub(2),
        height: resource_area.height.saturating_sub(2),
    };
    if inner.height < 3 {
        return None;
    }
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(inner);
    Some(sections[0])
}

fn sanitize_log_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            match chars.peek() {
                Some('[') => {
                    chars.next();
                    while let Some(c) = chars.next() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    loop {
                        match chars.next() {
                            Some('\u{0007}') => break,
                            Some('\u{1b}') => {
                                if matches!(chars.peek(), Some('\\')) {
                                    chars.next();
                                    break;
                                }
                            }
                            Some(_) => {}
                            None => break,
                        }
                    }
                }
                Some(_) => {
                    chars.next();
                }
                None => {}
            }
            continue;
        }

        if ch == '\r' {
            continue;
        }

        if ch.is_control() && ch != '\t' {
            continue;
        }

        out.push(ch);
    }

    out
}

fn max_logs_scroll(app: &AppState) -> usize {
    let viewport = app.logs_viewport_height as usize;
    app.logs_lines.len().saturating_sub(viewport)
}

async fn refresh_tab(engine: Arc<dyn Engine>, app: &mut AppState) {
    if app.engine_daemon_state != dockertui_core::daemon::DaemonState::Running {
        app.status = "Engine is stopped; start it to refresh".into();
        return;
    }
    match app.tab {
        Tab::Containers => {
            app.status = "Refreshing containers...".into();
            match engine.list_containers().await {
                Ok(Ok(list)) => {
                    app.containers = list;
                    if app.selected_container >= app.containers.len() {
                        app.selected_container = app.containers.len().saturating_sub(1);
                    }
                    app.status = format!("Containers: {}", app.containers.len());
                }
                Ok(Err(e)) => app.status = format!("Error: {e}"),
                Err(e) => app.status = format!("Task error: {e}"),
            }
        }
        Tab::Images => {
            app.status = "Refreshing images...".into();
            match engine.list_images().await {
                Ok(Ok(list)) => {
                    app.images = list;
                    if app.selected_image >= app.images.len() {
                        app.selected_image = app.images.len().saturating_sub(1);
                    }
                    app.status = format!("Images: {}", app.images.len());
                }
                Ok(Err(e)) => app.status = format!("Error: {e}"),
                Err(e) => app.status = format!("Task error: {e}"),
            }
        }
        Tab::Volumes => {
            app.status = "Refreshing volumes...".into();
            match engine.list_volumes().await {
                Ok(Ok(list)) => {
                    app.volumes = list;
                    if app.selected_volume >= app.volumes.len() {
                        app.selected_volume = app.volumes.len().saturating_sub(1);
                    }
                    app.status = format!("Volumes: {}", app.volumes.len());
                }
                Ok(Err(e)) => app.status = format!("Error: {e}"),
                Err(e) => app.status = format!("Task error: {e}"),
            }
        }
        Tab::Logs => {
            // logs are handled on open
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_log_line_strips_ansi_and_controls() {
        let input = "hi\x1b[31mred\x1b[0m";
        assert_eq!(sanitize_log_line(input), "hired");

        let input = "a\u{0001}b\u{0002}c";
        assert_eq!(sanitize_log_line(input), "abc");

        let input = "\x1b]0;title\x07ok";
        assert_eq!(sanitize_log_line(input), "ok");
    }

    #[test]
    fn max_logs_scroll_clamps_to_viewport() {
        let mut app = AppState::new();
        app.logs_lines = (0..10).map(|n| format!("line {n}")).collect();
        app.logs_viewport_height = 4;
        assert_eq!(max_logs_scroll(&app), 6);

        app.logs_viewport_height = 20;
        assert_eq!(max_logs_scroll(&app), 0);
    }
}

fn move_down(app: &mut AppState) {
    match app.tab {
        Tab::Containers => {
            if !app.containers.is_empty() {
                app.selected_container = (app.selected_container + 1).min(app.containers.len() - 1);
            }
        }
        Tab::Images => {
            if !app.images.is_empty() {
                app.selected_image = (app.selected_image + 1).min(app.images.len() - 1);
            }
        }
        Tab::Volumes => {
            if !app.volumes.is_empty() {
                app.selected_volume = (app.selected_volume + 1).min(app.volumes.len() - 1);
            }
        }
        Tab::Logs => {
            if app.logs_follow {
                app.logs_follow = false;
                app.status = "Logs follow: false".into();
                app.logs_scroll = max_logs_scroll(app);
            }
            let max_scroll = max_logs_scroll(app);
            app.logs_scroll = app.logs_scroll.saturating_add(1).min(max_scroll);
        }
    }
}

pub fn selected_container_is_running(app: &AppState) -> bool {
    if app.tab != Tab::Containers || app.containers.is_empty() {
        return false;
    }
    app.containers
        .get(app.selected_container)
        .map(|c| c.state == "running")
        .unwrap_or(false)
}

fn move_up(app: &mut AppState) {
    match app.tab {
        Tab::Containers => {
            app.selected_container = app.selected_container.saturating_sub(1);
        }
        Tab::Images => {
            app.selected_image = app.selected_image.saturating_sub(1);
        }
        Tab::Volumes => {
            app.selected_volume = app.selected_volume.saturating_sub(1);
        }
        Tab::Logs => {
            if app.logs_follow {
                app.logs_follow = false;
                app.status = "Logs follow: false".into();
                app.logs_scroll = max_logs_scroll(app);
            }
            app.logs_scroll = app.logs_scroll.saturating_sub(1);
        }
    }
}

fn request_selected_container_stats(
    engine: Arc<dyn Engine>,
    app: &mut AppState,
    ui_tx: tokio::sync::mpsc::UnboundedSender<UiMsg>,
) {
    if app.tab != Tab::Containers {
        return;
    }

    let Some(row) = app.containers.get(app.selected_container) else {
        app.container_stats = None;
        app.container_stats_for = None;
        app.container_stats_loading = false;
        app.container_stats_error = None;
        app.container_stats_updated_at = None;
        app.container_stats_requested_at = None;
        return;
    };

    let id = row.id.clone();
    let same_id = app.container_stats_for.as_deref() == Some(id.as_str());
    if same_id && app.container_stats_loading {
        return;
    }

    if !same_id {
        app.container_stats = None;
        app.container_stats_error = None;
        app.container_stats_loading = false;
        app.container_stats_updated_at = None;
        app.container_stats_requested_at = None;
    }

    app.container_stats_for = Some(id.clone());

    if app.engine_daemon_state != dockertui_core::daemon::DaemonState::Running {
        app.container_stats = None;
        app.container_stats_loading = false;
        app.container_stats_error = Some("Engine is not running".into());
        app.container_stats_updated_at = None;
        return;
    }

    if row.state != "running" {
        app.container_stats = None;
        app.container_stats_loading = false;
        app.container_stats_error = Some("Container is not running".into());
        app.container_stats_updated_at = None;
        return;
    }

    let refresh_window = Duration::from_millis(800);
    if same_id {
        if let Some(updated_at) = app.container_stats_updated_at {
            if updated_at.elapsed() < refresh_window {
                return;
            }
        }
        if let Some(requested_at) = app.container_stats_requested_at {
            if requested_at.elapsed() < refresh_window {
                return;
            }
        }
    }

    app.container_stats_loading = app.container_stats.is_none() || !same_id;
    app.container_stats_error = None;
    app.container_stats_requested_at = Some(Instant::now());

    tokio::spawn(async move {
        let result = match engine.container_stats(id.clone()).await {
            Ok(Ok(stats)) => Ok(stats),
            Ok(Err(e)) => Err(format!("{e}")),
            Err(e) => Err(format!("Task error: {e}")),
        };

        match result {
            Ok(stats) => {
                let _ = ui_tx.send(UiMsg::ContainerStats { id, stats });
            }
            Err(error) => {
                let _ = ui_tx.send(UiMsg::ContainerStatsError { id, error });
            }
        }
    });
}

fn push_stats_history(app: &mut AppState, id: &str) {
    if app.container_stats_for.as_deref() != Some(id) {
        return;
    }
    let Some(stats) = &app.container_stats else {
        return;
    };

    let history = app
        .container_stats_history
        .entry(id.to_string())
        .or_insert_with(VecDeque::new);

    let point = StatsPoint {
        at: Instant::now(),
        cpu_percent: stats.cpu_percent_value.max(0.0).round() as u64,
        mem_percent: stats.mem_percent_value.max(0.0).round() as u64,
        net_total: stats.net_rx_bytes.saturating_add(stats.net_tx_bytes),
        block_total: stats.block_read_bytes.saturating_add(stats.block_write_bytes),
        pids: stats.pids_value,
        mem_usage_bytes: stats.mem_usage_bytes,
    };

    history.push_back(point);

    let max_age = Duration::from_secs(300);
    while let Some(front) = history.front() {
        if front.at.elapsed() > max_age {
            history.pop_front();
        } else {
            break;
        }
    }

    if history.len() > 400 {
        let extra = history.len() - 400;
        history.drain(0..extra);
    }
}

async fn start_selected(
    engine: Arc<dyn Engine>,
    app: &mut AppState,
    ui_tx: tokio::sync::mpsc::UnboundedSender<UiMsg>,
) {
    if app.tab != Tab::Containers || app.containers.is_empty() {
        return;
    }
    let id = app.containers[app.selected_container].id.clone();
    app.status = format!("Starting {id}...");
    let _ = ui_tx.send(UiMsg::ContainersActivityStart(vec![id.clone()]));
    tokio::spawn(async move {
        let status = match engine.start_container(id).await {
            Ok(Ok(())) => "Started container".to_string(),
            Ok(Err(e)) => format!("Error: {e}"),
            Err(e) => format!("Task error: {e}"),
        };

        match engine.list_containers().await {
            Ok(Ok(list)) => {
                let _ = ui_tx.send(UiMsg::SetContainers(list));
            }
            Ok(Err(e)) => {
                let _ = ui_tx.send(UiMsg::Status(format!("Refresh failed: {e}")));
            }
            Err(e) => {
                let _ = ui_tx.send(UiMsg::Status(format!("Refresh task error: {e}")));
            }
        }

        let _ = ui_tx.send(UiMsg::ContainersActivityDone(status));
    });
}

async fn stop_selected(
    engine: Arc<dyn Engine>,
    app: &mut AppState,
    ui_tx: tokio::sync::mpsc::UnboundedSender<UiMsg>,
) {
    if app.tab != Tab::Containers || app.containers.is_empty() {
        return;
    }
    let id = app.containers[app.selected_container].id.clone();
    app.status = format!("Stopping {id}...");
    let _ = ui_tx.send(UiMsg::ContainersActivityStart(vec![id.clone()]));
    tokio::spawn(async move {
        let status = match engine.stop_container(id).await {
            Ok(Ok(())) => "Stopped container".to_string(),
            Ok(Err(e)) => format!("Error: {e}"),
            Err(e) => format!("Task error: {e}"),
        };

        match engine.list_containers().await {
            Ok(Ok(list)) => {
                let _ = ui_tx.send(UiMsg::SetContainers(list));
            }
            Ok(Err(e)) => {
                let _ = ui_tx.send(UiMsg::Status(format!("Refresh failed: {e}")));
            }
            Err(e) => {
                let _ = ui_tx.send(UiMsg::Status(format!("Refresh task error: {e}")));
            }
        }

        let _ = ui_tx.send(UiMsg::ContainersActivityDone(status));
    });
}

async fn restart_selected(
    engine: Arc<dyn Engine>,
    app: &mut AppState,
    ui_tx: tokio::sync::mpsc::UnboundedSender<UiMsg>,
) {
    if app.tab != Tab::Containers || app.containers.is_empty() {
        return;
    }
    let id = app.containers[app.selected_container].id.clone();
    app.status = format!("Restarting {id}...");
    let _ = ui_tx.send(UiMsg::ContainersActivityStart(vec![id.clone()]));
    tokio::spawn(async move {
        let status = match engine.restart_container(id).await {
            Ok(Ok(())) => "Restarted container".to_string(),
            Ok(Err(e)) => format!("Error: {e}"),
            Err(e) => format!("Task error: {e}"),
        };

        match engine.list_containers().await {
            Ok(Ok(list)) => {
                let _ = ui_tx.send(UiMsg::SetContainers(list));
            }
            Ok(Err(e)) => {
                let _ = ui_tx.send(UiMsg::Status(format!("Refresh failed: {e}")));
            }
            Err(e) => {
                let _ = ui_tx.send(UiMsg::Status(format!("Refresh task error: {e}")));
            }
        }

        let _ = ui_tx.send(UiMsg::ContainersActivityDone(status));
    });
}

async fn start_all_containers(
    engine: Arc<dyn Engine>,
    app: &mut AppState,
    ui_tx: tokio::sync::mpsc::UnboundedSender<UiMsg>,
) {
    if app.tab != Tab::Containers {
        return;
    }
    app.status = "Starting all containers...".into();
    tokio::spawn(async move {
        let list = match engine.list_containers().await {
            Ok(Ok(list)) => list,
            Ok(Err(e)) => {
                let _ = ui_tx.send(UiMsg::Status(format!("Error: {e}")));
                return;
            }
            Err(e) => {
                let _ = ui_tx.send(UiMsg::Status(format!("Task error: {e}")));
                return;
            }
        };

        let targets: Vec<String> = list
            .iter()
            .filter(|c| c.state != "running")
            .map(|c| c.id.clone())
            .collect();
        let total = targets.len();
        let _ = ui_tx.send(UiMsg::ContainersActivityStart(targets.clone()));

        let mut started = 0usize;
        for id in targets {
            match engine.start_container(id).await {
                Ok(Ok(())) => started += 1,
                Ok(Err(e)) => {
                    let _ = ui_tx.send(UiMsg::Status(format!("Error starting container: {e}")));
                }
                Err(e) => {
                    let _ = ui_tx.send(UiMsg::Status(format!("Start container task error: {e}")));
                }
            }
        }

        match engine.list_containers().await {
            Ok(Ok(list)) => {
                let _ = ui_tx.send(UiMsg::SetContainers(list));
            }
            Ok(Err(e)) => {
                let _ = ui_tx.send(UiMsg::Status(format!("Refresh failed: {e}")));
            }
            Err(e) => {
                let _ = ui_tx.send(UiMsg::Status(format!("Refresh task error: {e}")));
            }
        }

        let _ = ui_tx.send(UiMsg::ContainersActivityDone(format!(
            "Started {started}/{total} containers"
        )));
    });
}

async fn stop_all_containers(
    engine: Arc<dyn Engine>,
    app: &mut AppState,
    ui_tx: tokio::sync::mpsc::UnboundedSender<UiMsg>,
) {
    if app.tab != Tab::Containers {
        return;
    }
    app.status = "Stopping all containers...".into();
    tokio::spawn(async move {
        let list = match engine.list_containers().await {
            Ok(Ok(list)) => list,
            Ok(Err(e)) => {
                let _ = ui_tx.send(UiMsg::Status(format!("Error: {e}")));
                return;
            }
            Err(e) => {
                let _ = ui_tx.send(UiMsg::Status(format!("Task error: {e}")));
                return;
            }
        };

        let targets: Vec<String> = list
            .iter()
            .filter(|c| c.state == "running")
            .map(|c| c.id.clone())
            .collect();
        let total = targets.len();
        let _ = ui_tx.send(UiMsg::ContainersActivityStart(targets.clone()));

        let mut stopped = 0usize;
        for id in targets {
            match engine.stop_container(id).await {
                Ok(Ok(())) => stopped += 1,
                Ok(Err(e)) => {
                    let _ = ui_tx.send(UiMsg::Status(format!("Error stopping container: {e}")));
                }
                Err(e) => {
                    let _ = ui_tx.send(UiMsg::Status(format!("Stop container task error: {e}")));
                }
            }
        }

        match engine.list_containers().await {
            Ok(Ok(list)) => {
                let _ = ui_tx.send(UiMsg::SetContainers(list));
            }
            Ok(Err(e)) => {
                let _ = ui_tx.send(UiMsg::Status(format!("Refresh failed: {e}")));
            }
            Err(e) => {
                let _ = ui_tx.send(UiMsg::Status(format!("Refresh task error: {e}")));
            }
        }

        let _ = ui_tx.send(UiMsg::ContainersActivityDone(format!(
            "Stopped {stopped}/{total} containers"
        )));
    });
}

async fn open_logs(engine: Arc<dyn Engine>, app: &mut AppState, ui_tx: tokio::sync::mpsc::UnboundedSender<UiMsg>) {
    if app.tab != Tab::Containers || app.containers.is_empty() {
        return;
    }

    let row = app.containers[app.selected_container].clone();
    app.tab = Tab::Logs;
    app.logs_title = format!("Logs: {} ({})", row.name, &row.id[..row.id.len().min(12)]);
    app.logs_lines.clear();
    app.logs_scroll = 0;
    app.logs_follow = true;

    // cancel any existing log tail
    if let Some(cancel) = app.logs_cancel.take() {
        let _ = cancel.send(());
    }

    let (cancel_tx, mut cancel_rx) = oneshot::channel::<()>();
    app.logs_cancel = Some(cancel_tx);
    app.logs_running = true;
    app.status = "Tailing logs... (press q to exit logs)".into();

    let id = row.id.clone();
    let follow = app.logs_follow;

    // request the stream from engine
    let handle = engine.container_logs(
        id.clone(),
        LogsOptions { follow, tail: Some("200".into()), timestamps: false },
    );

    tokio::spawn(async move {
        let stream_res = handle.await;
        let mut stream = match stream_res {
            Ok(Ok(s)) => s, // NOTE: trait now returns Pin<Box<..>>
            Ok(Err(e)) => {
                let _ = ui_tx.send(UiMsg::LogStopped(format!("Log error: {e}")));
                return;
            }
            Err(e) => {
                let _ = ui_tx.send(UiMsg::LogStopped(format!("Log task error: {e}")));
                return;
            }
        };

        // read stream until cancelled
        loop {
            let mut pinned = stream.as_mut();

            tokio::select! {
                _ = &mut cancel_rx => {
                    let _ = ui_tx.send(UiMsg::LogStopped("Stopped log tail".into()));
                    break;
                }

                item = pinned.next() => {
                    match item {
                        Some(Ok(chunk)) => {
                            for line in chunk.lines() {
                                let _ = ui_tx.send(UiMsg::LogLine(line.to_string()));
                            }
                        }
                        Some(Err(e)) => {
                            let _ = ui_tx.send(UiMsg::LogStopped(format!("Log stream error: {e}")));
                            break;
                        }
                        None => {
                            let _ = ui_tx.send(UiMsg::LogStopped("Log stream ended".into()));
                            break;
                        }
                    }
                }
            }
        }
    });
}


async fn shell_into(
    engine: Arc<dyn Engine>,
    app: &mut AppState,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ui_tx: tokio::sync::mpsc::UnboundedSender<UiMsg>,
) -> Result<()> {
    if app.tab != Tab::Containers || app.containers.is_empty() {
        return Ok(());
    }
    let row = app.containers[app.selected_container].clone();

    let id_short = if row.id.len() > 12 { &row.id[..12] } else { &row.id };
    app.shell_active = true;
    app.shell_title = format!("Shell: {} ({id_short})", row.name);
    app.status = format!("Shell into {}... (type exit to return)", row.name);
    app.shell_lines.clear();
    app.shell_current.clear();

    terminal.autoresize()?;
    terminal.draw(|f| draw(f, app))?;

    let size = terminal.size()?;
    let main_area = main_area_rect(size);
    let session = crate::shell::spawn_shell_pty(engine.kind(), &row.id, main_area.width, main_area.height)?;
    let mut reader = session.master.try_clone_reader()?;

    let tx = ui_tx.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    let _ = tx.send(UiMsg::ShellClosed);
                    break;
                }
                Ok(n) => {
                    let text = String::from_utf8_lossy(&buf[..n]).to_string();
                    let _ = tx.send(UiMsg::ShellOutput(text));
                }
                Err(_) => {
                    let _ = tx.send(UiMsg::ShellClosed);
                    break;
                }
            }
        }
    });

    app.shell_session = Some(session);
    resize_shell(app, terminal)?;
    Ok(())
}

async fn end_shell(engine: Arc<dyn Engine>, app: &mut AppState) {
    if !app.shell_active {
        return;
    }
    app.shell_active = false;
    app.shell_title.clear();
    app.shell_session = None;
    app.tab = Tab::Containers;
    refresh_tab(engine, app).await;
    app.status = "Returned from shell".into();
}

fn main_area_rect(area: ratatui::layout::Rect) -> ratatui::layout::Rect {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1), Constraint::Length(3)])
        .split(area)[1]
}

fn resize_shell(
    app: &mut AppState,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> Result<()> {
    if let Some(session) = app.shell_session.as_mut() {
        let size = terminal.size()?;
        let main_area = main_area_rect(size);
        session.master.resize(PtySize {
            rows: main_area.height.max(1),
            cols: main_area.width.max(1),
            pixel_width: 0,
            pixel_height: 0,
        })?;
    }
    Ok(())
}

fn write_shell_input(
    session: &mut crate::shell::ShellSession,
    key: crossterm::event::KeyEvent,
) -> io::Result<()> {
    use crossterm::event::KeyCode;
    use crossterm::event::KeyModifiers;

    let mut bytes = Vec::new();
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);

    match key.code {
        KeyCode::Char(c) => {
            if ctrl {
                let code = (c as u8) & 0x1f;
                bytes.push(code);
            } else {
                if alt {
                    bytes.push(0x1b);
                }
                let mut buf = [0; 4];
                bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
        KeyCode::Enter => bytes.push(b'\r'),
        KeyCode::Tab => bytes.push(b'\t'),
        KeyCode::Backspace => bytes.push(0x7f),
        KeyCode::Esc => bytes.push(0x1b),
        KeyCode::Up => bytes.extend_from_slice(b"\x1b[A"),
        KeyCode::Down => bytes.extend_from_slice(b"\x1b[B"),
        KeyCode::Right => bytes.extend_from_slice(b"\x1b[C"),
        KeyCode::Left => bytes.extend_from_slice(b"\x1b[D"),
        KeyCode::Home => bytes.extend_from_slice(b"\x1b[H"),
        KeyCode::End => bytes.extend_from_slice(b"\x1b[F"),
        KeyCode::PageUp => bytes.extend_from_slice(b"\x1b[5~"),
        KeyCode::PageDown => bytes.extend_from_slice(b"\x1b[6~"),
        KeyCode::Delete => bytes.extend_from_slice(b"\x1b[3~"),
        KeyCode::Insert => bytes.extend_from_slice(b"\x1b[2~"),
        _ => {}
    }

    if !bytes.is_empty() {
        session.writer.write_all(&bytes)?;
        session.writer.flush()?;
    }

    Ok(())
}

fn push_shell_output(app: &mut AppState, text: &str) {
    enum AnsiState {
        Normal,
        Esc,
        Csi(String),
        Osc,
        OscEsc,
    }

    let mut state = AnsiState::Normal;
    let mut saw_cr = false;

    for ch in text.chars() {
        match state {
            AnsiState::Normal => {
                match ch {
                    '\x1b' => {
                        state = AnsiState::Esc;
                        continue;
                    }
                    '\n' => {
                        if !saw_cr || !app.shell_current.is_empty() {
                            app.shell_lines.push(std::mem::take(&mut app.shell_current));
                        } else {
                            app.shell_current.clear();
                        }
                        saw_cr = false;
                    }
                    '\r' => {
                        if !app.shell_current.is_empty() {
                            app.shell_lines.push(std::mem::take(&mut app.shell_current));
                        } else {
                            app.shell_current.clear();
                        }
                        saw_cr = true;
                    }
                    '\t' => {
                        app.shell_current.push_str("    ");
                        saw_cr = false;
                    }
                    '\x08' | '\x7f' => {
                        app.shell_current.pop();
                        saw_cr = false;
                    }
                    _ => {
                        if !ch.is_control() {
                            app.shell_current.push(ch);
                        }
                        saw_cr = false;
                    }
                }
            }
            AnsiState::Esc => {
                state = match ch {
                    '[' => AnsiState::Csi(String::new()),
                    ']' => AnsiState::Osc,
                    _ => AnsiState::Normal,
                };
            }
            AnsiState::Csi(mut params) => {
                if ('@'..='~').contains(&ch) {
                    if ch == 'J' && params.contains('2') {
                        app.shell_lines.clear();
                        app.shell_current.clear();
                    }
                    if ch == 'K' {
                        app.shell_current.clear();
                    }
                    state = AnsiState::Normal;
                } else {
                    params.push(ch);
                    state = AnsiState::Csi(params);
                }
            }
            AnsiState::Osc => {
                if ch == '\x07' {
                    state = AnsiState::Normal;
                } else if ch == '\x1b' {
                    state = AnsiState::OscEsc;
                }
            }
            AnsiState::OscEsc => {
                if ch == '\\' {
                    state = AnsiState::Normal;
                } else {
                    state = AnsiState::Osc;
                }
            }
        }
    }

    let max_lines = 5000;
    if app.shell_lines.len() > max_lines {
        let extra = app.shell_lines.len() - max_lines;
        app.shell_lines.drain(0..extra);
    }
}


async fn remove_selected(engine: Arc<dyn Engine>, app: &mut AppState) {
    match app.tab {
        Tab::Images => {
            if app.images.is_empty() {
                return;
            }
            let id = app.images[app.selected_image].id.clone();
            app.status = format!("Removing image {id}...");
            match engine.remove_image(id, true).await {
                Ok(Ok(())) => {
                    app.status = "Image removed".into();
                    refresh_tab(engine, app).await;
                }
                Ok(Err(e)) => app.status = format!("Error: {e}"),
                Err(e) => app.status = format!("Task error: {e}"),
            }
        }
        Tab::Volumes => {
            if app.volumes.is_empty() {
                return;
            }
            let name = app.volumes[app.selected_volume].name.clone();
            app.status = format!("Removing volume {name}...");
            match engine.remove_volume(name, true).await {
                Ok(Ok(())) => {
                    app.status = "Volume removed".into();
                    refresh_tab(engine, app).await;
                }
                Ok(Err(e)) => app.status = format!("Error: {e}"),
                Err(e) => app.status = format!("Task error: {e}"),
            }
        }
        _ => {}
    }
}
