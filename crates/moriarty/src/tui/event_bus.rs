use std::future;

use crossterm::event::{Event as TermEvent, EventStream, KeyEvent};
use futures::{Stream, StreamExt};
use miette::IntoDiagnostic;

pub fn input_stream() -> impl Stream<Item = miette::Result<Event>> {
    let input = EventStream::new();

    input.filter_map(|event| {
        future::ready(match event.into_diagnostic() {
            Ok(event) => event.to_maybe_event().map(Ok),
            Err(error) => Some(Err(error.context("failed to read terminal events"))),
        })
    })
}

pub enum Event {
    UI(UIEvent),
}

pub enum UIEvent {
    Key(KeyEvent),
    #[allow(dead_code)]
    Paste(String),
    Render,
}

impl From<UIEvent> for Event {
    fn from(value: UIEvent) -> Self {
        Event::UI(value)
    }
}

pub trait MaybeIntoEvent {
    fn to_maybe_event(self) -> Option<Event>;
}

impl MaybeIntoEvent for TermEvent {
    fn to_maybe_event(self) -> Option<Event> {
        match self {
            Self::FocusGained | Self::FocusLost | Self::Mouse(_) => None,
            Self::Key(key) => Some(UIEvent::Key(key).into()),
            Self::Paste(paste) => Some(UIEvent::Paste(paste).into()),
            Self::Resize(_, _) => Some(UIEvent::Render.into()),
        }
    }
}
