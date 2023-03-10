use crate::cui::window::general::tabs;
use crate::cui::window::general::tabs::{TabVariant, Tabs};
use crate::cui::window::message::{ActionMessage, Exchanger};
use crate::cui::window::{CuiComponent, RenderOpts};
use crate::cui::AppState;
use crate::fire;
use crossterm::event::KeyEvent;
use std::collections::HashMap;
use std::default::Default;
use std::io::StdoutLock;
use tui::backend::CrosstermBackend;
use tui::layout::{Constraint, Direction, Layout, Rect};
use tui::Frame;

pub struct WindowDeck {
    name: &'static str,
    visible_window: &'static str,
    in_focus_window: Option<&'static str>,
    tabs: Tabs,
    windows: HashMap<&'static str, Box<dyn CuiComponent>>,
}

impl WindowDeck {
    pub fn new(
        name: &'static str,
        windows: Vec<Box<dyn CuiComponent>>,
        state_asserts: HashMap<&'static str, AppState>,
    ) -> Self {
        let tab_variants = windows
            .iter()
            .map(|component| {
                let c_name = component.name();

                if let Some(state) = state_asserts.get(c_name) {
                    TabVariant::contextual(
                        c_name,
                        ActionMessage::ActivateComponent { activate: c_name },
                        *state,
                        name,
                    )
                } else {
                    TabVariant::new(
                        c_name,
                        ActionMessage::ActivateComponent { activate: c_name },
                        name,
                    )
                }
            })
            .collect::<Vec<_>>();

        let tabs = tabs::Tabs::new("deck_tabs", "", tab_variants);

        Self {
            name,
            visible_window: windows[0].name(),
            in_focus_window: None,
            tabs,
            windows: windows.into_iter().map(|c| (c.name(), c)).collect(),
        }
    }

    pub fn drop_focus(&mut self) {
        self.in_focus_window = None
    }
}

impl CuiComponent for WindowDeck {
    fn render(
        &self,
        frame: &mut Frame<CrosstermBackend<StdoutLock>>,
        rect: Rect,
        mut opts: RenderOpts,
    ) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(0)
            .constraints([Constraint::Length(3), Constraint::Min(2)])
            .split(rect);

        if self.in_focus_window.is_some() {
            opts.in_focus = true;
        }

        self.tabs.render(frame, chunks[0], opts);
        self.windows[self.visible_window].render(frame, chunks[1], opts);
    }

    fn handle_user_event(&mut self, e: KeyEvent) {
        self.tabs.handle_user_event(e);
        if let Some(in_focus_component) = self.in_focus_window {
            if let Some(component) = self.windows.get_mut(in_focus_component) {
                component.handle_user_event(e);
            }
        }
    }

    fn update(&mut self) -> anyhow::Result<()> {
        for msg in Exchanger::current().pop(self.name) {
            match msg {
                ActionMessage::ActivateComponent { activate } => {
                    self.visible_window = activate;
                    fire!(ActionMessage::FocusOnComponent {focus_on: activate} => "app_window");
                }
                ActionMessage::FocusOnComponent { focus_on } => {
                    if self.windows.get(focus_on).is_some() {
                        self.in_focus_window = Some(focus_on);
                    }
                }
                _ => {}
            }
        }

        self.windows.iter_mut().try_for_each(|(_, w)| w.update())
    }

    fn name(&self) -> &'static str {
        self.name
    }
}
