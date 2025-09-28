use std::future;

use crossterm::event::{Event as TermEvent, EventStream, KeyEvent, MouseEvent};
use futures::{Stream, StreamExt};
use miette::{IntoDiagnostic, Report, Result};

pub fn input_stream() -> impl Stream<Item = Result<Event>> {
    let input = EventStream::new();

    input.filter_map(|event| {
        future::ready(match event.into_diagnostic() {
            Ok(event) => event.to_maybe_event().map(|v| Ok(v)),
            Err(error) => Some(Err(error.context("failed to read terminal events"))),
        })
    })
}

pub enum Event {
    App(AppEvent),
    Render,
}

pub enum AppEvent {
    Key(KeyEvent),
    Paste(String),
}

impl From<AppEvent> for Event {
    fn from(value: AppEvent) -> Self {
        Self::App(value)
    }
}

pub trait MaybeIntoEvent {
    fn to_maybe_event(self) -> Option<Event>;
}

impl MaybeIntoEvent for TermEvent {
    fn to_maybe_event(self) -> Option<Event> {
        match self {
            Self::FocusGained | Self::FocusLost | Self::Mouse(_) => None,
            Self::Key(key) => Some(AppEvent::Key(key).into()),
            Self::Paste(paste) => Some(AppEvent::Paste(paste).into()),
            Self::Resize(_, _) => Some(Event::Render),
        }
    }
}
