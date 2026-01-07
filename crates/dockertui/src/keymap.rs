use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy)]
pub enum Action {
    Quit,
    NextTab,
    PrevTab,
    Refresh,
    Down,
    Up,

    Start,
    Stop,
    Restart,
    StartAll,
    StopAll,
    Logs,
    Shell,
    Remove,

    ToggleFollow,
    StartEngine,
    StopEngine,
    RestartEngine,
}

#[derive(Default)]
pub struct Keymap;

impl Keymap {
    pub fn map(&self, key: KeyEvent) -> Option<Action> {
        use KeyCode::*;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        match key.code {
            Char('q') | Esc => Some(Action::Quit),

            Tab if key.modifiers.is_empty() => Some(Action::NextTab),
            BackTab if key.modifiers.contains(KeyModifiers::SHIFT) => Some(Action::PrevTab),

            F(5) => Some(Action::Refresh),

            Down | Char('j') => Some(Action::Down),
            Up | Char('k') => Some(Action::Up),

            Char('s') => Some(Action::Start),
            Char('S') => Some(Action::StartAll),

            Char('x') if ctrl => Some(Action::StopEngine),
            Char('x') => Some(Action::Stop),
            Char('X') => Some(Action::StopAll),

            Char('r') if ctrl => Some(Action::RestartEngine),
            Char('r') => Some(Action::Restart),

            Char('l') => Some(Action::Logs),

            Delete => Some(Action::Remove),

            Char('f') => Some(Action::ToggleFollow),

            Char('e') if ctrl => Some(Action::StartEngine),
            Char('e') => Some(Action::Shell),

            Char('w') if ctrl => Some(Action::Quit),
            _ => None,
        }
    }
}
