//! The consent prompt of User Account Control (ADR 0222): a Windows program asks to run a program
//! with administrator rights, and the user answers Yes or No.
//!
//! The compositor draws it, never the app: the whole screen is dimmed and a card in the middle
//! names the program, its publisher (from the file's signature, else "Unknown") and its path, with
//! **Yes** and **No**. While it shows, every key and click is the prompt's (the compositor gives
//! the apps none), so only the person at the real keyboard and pointer answers. No has the focus at
//! first: Enter on an unexpected prompt says no. Tab and the arrows move the focus; Escape is No.
//! Prompts asked while one shows wait their turn.
//!
//! ```text
//!  ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
//!  ░░░░╭──────────────────────────────────────╮░░░░░░
//!  ░░░░│ ⛨ Allow this app to make changes?    │░░░░░░
//!  ░░░░│   ChromeSetup.exe                    │░░░░░░
//!  ░░░░│   Publisher: Google LLC (not verified)│░░░░░░
//!  ░░░░│   C:\Users\poly\Downloads\Chrome…   │░░░░░░
//!  ░░░░│                     [ Yes ] [[ No ]] │░░░░░░
//!  ░░░░╰──────────────────────────────────────╯░░░░░░
//! ```

use crate::draw::{Face, Style, Text, fill, rounded_border, rounded_outline, rounded_rect};
use crate::icons::lucide;
use alloc::collections::VecDeque;
use alloc::string::String;
use poly_wayland::compose::{Framebuffer, Rect, Scale};

/// The measurements (logical pixels).
pub mod look {
    pub const WIDTH: i32 = 480;
    pub const PADDING: i32 = 20;
    pub const RADIUS: i32 = 12;
    pub const ICON: i32 = 20;
    pub const TITLE_SIZE: f32 = 16.0;
    pub const TEXT_SIZE: f32 = 13.0;
    pub const LINE: i32 = 22;
    pub const TITLE_LINE: i32 = 28;
    pub const BUTTON_WIDTH: i32 = 88;
    pub const BUTTON_HEIGHT: i32 = 32;
    pub const BUTTON_GAP: i32 = 8;
    /// The screen behind: black at 55 %.
    pub const DIM_ALPHA: u8 = 140;
    /// Fills: the card's ring at 10 %, a button at 10 %, the focused one's ring at 70 %.
    pub const RING_ALPHA: u8 = 26;
    pub const BUTTON_ALPHA: u8 = 26;
    pub const FOCUS_ALPHA: u8 = 179;
    pub const SECONDARY_ALPHA: u8 = 166;
    pub const TITLE: &str = "Allow this app to make changes to your device?";
}

/// The keys a prompt knows (the compositor maps the keyboard's codes to them).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptKey {
    /// Enter: the button with the focus.
    Enter,
    /// Escape: No.
    Escape,
    /// Tab or an arrow: the focus to the other button.
    Toggle,
}

/// One question.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prompt {
    /// The kernel's request number, for the answer.
    pub request: u32,
    pub program: String,
    /// The signer's name, if the file is signed.
    pub publisher: Option<String>,
    pub path: String,
}

/// The prompts: the one showing first.
#[derive(Debug, Default)]
pub struct Consent {
    queue: VecDeque<Prompt>,
    /// Yes has the focus (else No).
    yes_focused: bool,
    answers: alloc::vec::Vec<(u32, bool)>,
}

/// The card and its buttons (logical), on a `width` × `height` screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub card: Rect,
    pub yes: Rect,
    pub no: Rect,
}

impl Consent {
    /// Ask `p` (shown at once, or after those before it).
    pub fn ask(&mut self, p: Prompt) {
        if self.queue.is_empty() {
            self.yes_focused = false;
        }
        self.queue.push_back(p);
    }

    /// The prompt showing.
    pub fn current(&self) -> Option<&Prompt> {
        self.queue.front()
    }

    pub fn is_open(&self) -> bool {
        !self.queue.is_empty()
    }

    /// Whether Yes has the focus.
    pub fn yes_focused(&self) -> bool {
        self.yes_focused
    }

    /// The answers given since the last call: `(request, yes)`.
    pub fn take_answers(&mut self) -> alloc::vec::Vec<(u32, bool)> {
        core::mem::take(&mut self.answers)
    }

    fn answer(&mut self, yes: bool) {
        if let Some(p) = self.queue.pop_front() {
            self.answers.push((p.request, yes));
        }
        self.yes_focused = false;
    }

    /// Drop the prompts of `requests` that nobody needs to answer any more.
    pub fn withdraw(&mut self, request: u32) {
        let front = self.queue.front().map(|p| p.request);
        self.queue.retain(|p| p.request != request);
        if front == Some(request) {
            self.yes_focused = false;
        }
    }

    /// A key while a prompt shows (all keys are the prompt's). Whether anything changed.
    pub fn key(&mut self, key: PromptKey) -> bool {
        if !self.is_open() {
            return false;
        }
        match key {
            PromptKey::Enter => self.answer(self.yes_focused),
            PromptKey::Escape => self.answer(false),
            PromptKey::Toggle => self.yes_focused = !self.yes_focused,
        }
        true
    }

    /// A left press at logical (`x`, `y`) while a prompt shows: a button answers, anything else
    /// does nothing (the press is the prompt's either way). Whether anything changed.
    pub fn press(&mut self, x: i32, y: i32, width: i32, height: i32, text: &Text) -> bool {
        let Some(l) = self.layout(width, height, text) else { return false };
        if l.yes.contains(x, y) {
            self.answer(true);
            true
        } else if l.no.contains(x, y) {
            self.answer(false);
            true
        } else {
            false
        }
    }

    /// Where the card and its buttons are.
    pub fn layout(&self, width: i32, height: i32, _text: &Text) -> Option<Layout> {
        self.current()?;
        let w = look::WIDTH.min(width - 2 * look::PADDING).max(200);
        let h = 2 * look::PADDING + look::TITLE_LINE + 3 * look::LINE + look::PADDING + look::BUTTON_HEIGHT;
        let card = Rect::new((width - w) / 2, (height - h) / 2, w, h);
        let by = card.bottom() - look::PADDING - look::BUTTON_HEIGHT;
        let no =
            Rect::new(card.right() - look::PADDING - look::BUTTON_WIDTH, by, look::BUTTON_WIDTH, look::BUTTON_HEIGHT);
        let yes = Rect::new(no.x - look::BUTTON_GAP - look::BUTTON_WIDTH, by, look::BUTTON_WIDTH, look::BUTTON_HEIGHT);
        Some(Layout { card, yes, no })
    }

    /// Draw the prompt over everything: the dimmed screen and the card.
    pub fn draw(&self, fb: &mut dyn Framebuffer, clip: Rect, s: Scale, width: i32, height: i32, text: &Text) {
        let (Some(p), Some(l)) = (self.current(), self.layout(width, height, text)) else { return };
        let pal = crate::theme::pal();
        let ink = pal.ink;
        fill(fb, clip, Rect::new(0, 0, width * s, height * s), 0, look::DIM_ALPHA);
        let c = l.card.scale(s);
        let r = look::RADIUS * s;
        rounded_rect(fb, clip, c, r, pal.bar, 255);
        rounded_outline(fb, clip, c, r, ink, look::RING_ALPHA);
        // The shield and the question.
        let pad = look::PADDING * s;
        let title = Style::new(Face::Semibold, look::TITLE_SIZE).scaled(s);
        let body = Style::new(Face::Sans, look::TEXT_SIZE).scaled(s);
        let strong = Style::new(Face::Medium, look::TEXT_SIZE).scaled(s);
        let top = c.y + pad;
        let icon = look::ICON * s;
        lucide::INFO.draw(
            fb,
            clip,
            (c.x + pad) as f32,
            (top + (look::TITLE_LINE * s - icon) / 2) as f32,
            icon as f32,
            ink,
            255,
        );
        let tx = c.x + pad + icon + 10 * s;
        let right = c.right() - pad;
        text.draw_style(
            fb,
            clip,
            tx as f32,
            text.centre(top, look::TITLE_LINE * s, title),
            right,
            look::TITLE,
            title,
            ink,
            255,
        );
        // The program, its publisher, its path.
        let mut y = top + look::TITLE_LINE * s;
        let line = look::LINE * s;
        text.draw_style(fb, clip, tx as f32, text.centre(y, line, strong), right, &p.program, strong, ink, 255);
        y += line;
        let publisher = match &p.publisher {
            Some(name) => alloc::format!("Publisher: {name} (signature not verified)"),
            None => String::from("Publisher: Unknown"),
        };
        text.draw_style(
            fb,
            clip,
            tx as f32,
            text.centre(y, line, body),
            right,
            &publisher,
            body,
            ink,
            look::SECONDARY_ALPHA,
        );
        y += line;
        text.draw_style(
            fb,
            clip,
            tx as f32,
            text.centre(y, line, body),
            right,
            &p.path,
            body,
            ink,
            look::SECONDARY_ALPHA,
        );
        // Yes and No.
        for (b, label, focused) in [(l.yes, "Yes", self.yes_focused), (l.no, "No", !self.yes_focused)] {
            let b = b.scale(s);
            let br = 8 * s;
            rounded_rect(fb, clip, b, br, ink, look::BUTTON_ALPHA);
            if focused {
                rounded_border(fb, clip, b, br, 2 * s, ink, look::FOCUS_ALPHA);
            }
            let wl = text.measure(label, strong) as i32;
            text.draw_style(
                fb,
                clip,
                (b.x + (b.width - wl) / 2) as f32,
                text.centre(b.y, b.height, strong),
                b.right(),
                label,
                strong,
                ink,
                255,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prompt(request: u32) -> Prompt {
        Prompt {
            request,
            program: "ChromeSetup.exe".into(),
            publisher: Some("Google LLC".into()),
            path: "C:\\Users\\poly\\Downloads\\ChromeSetup.exe".into(),
        }
    }

    #[test]
    fn no_has_the_focus_and_keys_answer() {
        let mut c = Consent::default();
        assert!(!c.key(PromptKey::Enter));
        c.ask(prompt(1));
        c.ask(prompt(2));
        assert!(c.is_open() && !c.yes_focused());
        // Enter on the first: No.
        assert!(c.key(PromptKey::Enter));
        assert_eq!(c.take_answers(), [(1, false)]);
        // The second: Tab to Yes, Enter.
        assert_eq!(c.current().map(|p| p.request), Some(2));
        assert!(c.key(PromptKey::Toggle) && c.yes_focused());
        assert!(c.key(PromptKey::Enter));
        assert_eq!(c.take_answers(), [(2, true)]);
        assert!(!c.is_open());
        // Escape, and the focus back to No for a new prompt.
        c.ask(prompt(3));
        c.key(PromptKey::Toggle);
        c.key(PromptKey::Escape);
        assert_eq!(c.take_answers(), [(3, false)]);
        c.ask(prompt(4));
        assert!(!c.yes_focused());
    }

    #[test]
    fn the_buttons_answer_and_the_rest_of_the_screen_does_not() {
        let mut c = Consent::default();
        let text = Text::new(None);
        assert!(c.layout(1280, 800, &text).is_none());
        c.ask(prompt(7));
        let l = c.layout(1280, 800, &text).unwrap();
        assert!(l.card.contains(l.yes.x, l.yes.y) && l.card.contains(l.no.x, l.no.y));
        assert_eq!(l.card.x + l.card.width / 2, 640);
        // Outside the buttons: nothing.
        assert!(!c.press(5, 5, 1280, 800, &text));
        assert!(!c.press(l.card.x + 2, l.card.y + 2, 1280, 800, &text));
        assert!(c.is_open());
        assert!(c.press(l.yes.x + 1, l.yes.y + 1, 1280, 800, &text));
        assert_eq!(c.take_answers(), [(7, true)]);
        c.ask(prompt(8));
        assert!(c.press(l.no.x + 1, l.no.y + 1, 1280, 800, &text));
        assert_eq!(c.take_answers(), [(8, false)]);
    }

    #[test]
    fn a_withdrawn_prompt_goes() {
        let mut c = Consent::default();
        c.ask(prompt(1));
        c.ask(prompt(2));
        c.withdraw(1);
        assert_eq!(c.current().map(|p| p.request), Some(2));
        assert!(c.take_answers().is_empty());
    }

    #[test]
    fn it_draws_over_the_whole_screen() {
        let mut c = Consent::default();
        c.ask(prompt(1));
        let text = Text::new(None);
        let mut fb = poly_wayland::compose::MemoryFramebuffer::new(640, 400);
        for y in 0..400 {
            fb.row_mut(y).fill(0xffff_ffff);
        }
        c.draw(&mut fb, Rect::new(0, 0, 640, 400), Scale::ONE, 640, 400, &text);
        // A corner is dimmed (no longer white), the card's middle is the bar's colour.
        assert_ne!(fb.row_mut(1)[1], 0xffff_ffff);
        let l = c.layout(640, 400, &text).unwrap();
        let mid = fb.row_mut((l.card.y + 4) as u32)[(l.card.x + l.card.width / 2) as usize];
        assert_eq!(mid & 0xff_ffff, crate::theme::pal().bar & 0xff_ffff);
    }
}
